//! OSC (Open Sound Control) input, modeled on the real-world
//! [hydra-osc](https://github.com/ojack/hydra-osc) example (a browser
//! WebSocket bridge to a local UDP `osc-js` server) and its more complete
//! sibling, atom-hydra's own
//! [`osc-loader.js`](https://github.com/hydra-synth/atom-hydra/blob/master/lib/osc-loader.js)
//! (a Node child-process UDP bridge, `PORT = 57101` by default), ported
//! natively as a plain UDP listener - no browser/WebSocket bridge or child
//! process needed since this isn't running in a browser sandbox.
//!
//! Deliberate simplifications from both real implementations (each a "no
//! worse than a hard error" trade-off, matching e.g. midi.rs's own
//! documented scope reductions):
//! - Real hydra-osc/atom-hydra expose an `EventEmitter`-style
//!   `_osc.on(address, callback)`, running an arbitrary per-message JS
//!   callback. Hydra-rust scripts compile to a single static GLSL
//!   expression per chain (see midi.rs's own note on `.value(fn)` being
//!   unsupported for the same reason) - there's nowhere to run a per-frame
//!   Rhai closure. Instead, `_osc.get(address[, argIndex])` returns a
//!   reactive value read directly into a GLSL uniform, the same
//!   value-not-callback shape `note()`/`cc()` already use for MIDI.
//! - Each distinct `_osc.get(...)` call site claims one slot from a small
//!   fixed pool (`NUM_OSC_SLOTS`), assigned round-robin at eval time -
//!   mirrors `midi::NUM_MIDI_ENVELOPES`'s own fixed-size-uniform-array
//!   approach, needed here because (unlike MIDI's 0-127 note/CC numbers)
//!   OSC addresses are arbitrary strings with no natural small upper bound.
//! - OSC bundles/timetags are not specially interpreted - every contained
//!   message is just applied immediately, in order, as if it had arrived
//!   as its own plain message (a bundle's own timetag is ignored).

pub const NUM_OSC_SLOTS: usize = 32;
/// Real atom-hydra's own default (`const PORT = 57101` in `lib/main.js`).
pub const DEFAULT_OSC_PORT: u16 = 57101;

#[cfg(feature = "osc")]
mod imp {
    use std::collections::HashMap;
    use std::net::UdpSocket;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use rosc::{OscBundle, OscMessage, OscPacket, OscType};

    use super::NUM_OSC_SLOTS;

    fn osc_type_to_f32(v: &OscType) -> Option<f32> {
        match v {
            OscType::Int(i) => Some(*i as f32),
            OscType::Float(f) => Some(*f),
            OscType::Double(d) => Some(*d as f32),
            OscType::Long(l) => Some(*l as f32),
            OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            // Real hydra-osc/atom-hydra pass strings/blobs/etc. straight
            // through to JS untouched - there's no numeric GLSL-uniform
            // equivalent for those here, so they're just not readable via
            // `.get()` (same "no worse than a hard error" trade-off as
            // everything else in this module).
            _ => None,
        }
    }

    fn apply_message(values: &mut HashMap<String, Vec<f32>>, msg: OscMessage) {
        let args: Vec<f32> = msg.args.iter().filter_map(osc_type_to_f32).collect();
        values.insert(msg.addr, args);
    }

    /// Recurses into (possibly nested) bundles - see the module doc
    /// comment on why timetags aren't honored.
    fn apply_packet(values: &mut HashMap<String, Vec<f32>>, packet: OscPacket) {
        match packet {
            OscPacket::Message(msg) => apply_message(values, msg),
            OscPacket::Bundle(OscBundle { content, .. }) => {
                for inner in content {
                    apply_packet(values, inner);
                }
            }
        }
    }

    struct SharedState {
        /// Latest received argument list per OSC address.
        values: HashMap<String, Vec<f32>>,
    }

    impl SharedState {
        fn new() -> Self {
            Self {
                values: HashMap::new(),
            }
        }
    }

    #[derive(Clone)]
    struct SlotBinding {
        address: String,
        arg_index: usize,
    }

    /// A snapshot of every bound slot's current value, uploaded as one
    /// uniform array once per rendered frame - mirrors
    /// `midi::MidiFrame`'s own per-frame-snapshot shape.
    pub struct OscFrame {
        pub values: [f32; NUM_OSC_SLOTS],
    }

    pub struct OscManager {
        state: Arc<Mutex<SharedState>>,
        stop_flag: Option<Arc<AtomicBool>>,
        thread: Option<thread::JoinHandle<()>>,
        port: Option<u16>,
        slots: [Option<SlotBinding>; NUM_OSC_SLOTS],
    }

    impl Default for OscManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl OscManager {
        pub fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(SharedState::new())),
                stop_flag: None,
                thread: None,
                port: None,
                slots: [const { None }; NUM_OSC_SLOTS],
            }
        }

        /// Binds a local UDP socket and starts a background thread reading
        /// OSC packets from it. Not called automatically - only once a
        /// script actually calls `_osc.start()`, the same lazy-start-on-
        /// script-intent treatment `MidiManager::ensure_started` gives
        /// MIDI input.
        pub fn ensure_started(&mut self, port: u16) {
            if self.port == Some(port) && self.thread.is_some() {
                return;
            }
            self.pause();

            let socket = match UdpSocket::bind(("0.0.0.0", port)) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("_osc.start({port}): failed to bind UDP socket: {e}");
                    return;
                }
            };
            // A short read timeout, rather than blocking forever, so the
            // background thread can notice `stop_flag` and exit promptly
            // once `pause()`/a later `start()` on a different port asks it
            // to.
            if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(200))) {
                log::warn!("_osc.start({port}): failed to configure socket timeout: {e}");
                return;
            }

            let stop_flag = Arc::new(AtomicBool::new(false));
            let state = self.state.clone();
            let thread_stop_flag = stop_flag.clone();
            let handle = thread::spawn(move || {
                let mut buf = [0u8; 65536];
                while !thread_stop_flag.load(Ordering::Relaxed) {
                    match socket.recv(&mut buf) {
                        Ok(size) => match rosc::decoder::decode_udp(&buf[..size]) {
                            Ok((_, packet)) => {
                                let mut state = state.lock().unwrap();
                                apply_packet(&mut state.values, packet);
                            }
                            Err(e) => log::warn!("osc: failed to decode packet: {e:?}"),
                        },
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            // Just a read-timeout tick, so `stop_flag` gets
                            // rechecked - not an actual error.
                        }
                        Err(e) => {
                            log::warn!("osc: UDP read error: {e}");
                            break;
                        }
                    }
                }
            });

            log::info!("osc: listening on UDP port {port}");
            self.stop_flag = Some(stop_flag);
            self.thread = Some(handle);
            self.port = Some(port);
        }

        pub fn pause(&mut self) {
            if let Some(flag) = self.stop_flag.take() {
                flag.store(true, Ordering::Relaxed);
            }
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
            self.port = None;
        }

        /// (Re)assigns slot `slot` to read argument `arg_index` of
        /// whatever's most recently arrived at `address`.
        pub fn bind_slot(&mut self, slot: usize, address: String, arg_index: usize) {
            if slot < NUM_OSC_SLOTS {
                self.slots[slot] = Some(SlotBinding { address, arg_index });
            }
        }

        pub fn poll(&mut self) -> OscFrame {
            let state = self.state.lock().unwrap();
            let mut values = [0.0; NUM_OSC_SLOTS];
            for (i, slot) in self.slots.iter().enumerate() {
                if let Some(binding) = slot
                    && let Some(args) = state.values.get(&binding.address)
                    && let Some(v) = args.get(binding.arg_index)
                {
                    values[i] = *v;
                }
            }
            OscFrame { values }
        }

        /// Every currently-known address and its latest arguments - used
        /// only by `_osc.show()`'s on-screen monitor overlay.
        pub fn snapshot(&self) -> Vec<(String, Vec<f32>)> {
            let state = self.state.lock().unwrap();
            let mut entries: Vec<_> = state
                .values
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries
        }
    }

    impl Drop for OscManager {
        fn drop(&mut self) {
            self.pause();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_plain_message_is_stored_by_address() {
            let mut values = HashMap::new();
            apply_message(
                &mut values,
                OscMessage {
                    addr: "/hue".into(),
                    args: vec![OscType::Float(0.5)],
                },
            );
            assert_eq!(values.get("/hue"), Some(&vec![0.5_f32]));
        }

        #[test]
        fn multiple_args_are_kept_in_order() {
            let mut values = HashMap::new();
            apply_message(
                &mut values,
                OscMessage {
                    addr: "/xyz".into(),
                    args: vec![OscType::Float(1.0), OscType::Int(2), OscType::Float(3.0)],
                },
            );
            assert_eq!(values.get("/xyz"), Some(&vec![1.0, 2.0, 3.0]));
        }

        #[test]
        fn a_later_message_overwrites_an_earlier_one_at_the_same_address() {
            let mut values = HashMap::new();
            apply_message(
                &mut values,
                OscMessage {
                    addr: "/hue".into(),
                    args: vec![OscType::Float(0.1)],
                },
            );
            apply_message(
                &mut values,
                OscMessage {
                    addr: "/hue".into(),
                    args: vec![OscType::Float(0.9)],
                },
            );
            assert_eq!(values.get("/hue"), Some(&vec![0.9_f32]));
        }

        #[test]
        fn non_numeric_args_are_dropped_not_a_panic() {
            let mut values = HashMap::new();
            apply_message(
                &mut values,
                OscMessage {
                    addr: "/mixed".into(),
                    args: vec![
                        OscType::Float(1.0),
                        OscType::String("hello".into()),
                        OscType::Float(2.0),
                    ],
                },
            );
            assert_eq!(values.get("/mixed"), Some(&vec![1.0, 2.0]));
        }

        #[test]
        fn a_bundle_applies_every_contained_message() {
            let mut values = HashMap::new();
            let bundle = OscPacket::Bundle(OscBundle {
                timetag: rosc::OscTime {
                    seconds: 0,
                    fractional: 0,
                },
                content: vec![
                    OscPacket::Message(OscMessage {
                        addr: "/a".into(),
                        args: vec![OscType::Float(1.0)],
                    }),
                    OscPacket::Message(OscMessage {
                        addr: "/b".into(),
                        args: vec![OscType::Float(2.0)],
                    }),
                ],
            });
            apply_packet(&mut values, bundle);
            assert_eq!(values.get("/a"), Some(&vec![1.0_f32]));
            assert_eq!(values.get("/b"), Some(&vec![2.0_f32]));
        }

        #[test]
        fn poll_reads_the_bound_argument_index_and_defaults_unbound_slots_to_zero() {
            let mut mgr = OscManager::new();
            mgr.state
                .lock()
                .unwrap()
                .values
                .insert("/xyz".to_string(), vec![10.0, 20.0, 30.0]);
            mgr.bind_slot(0, "/xyz".to_string(), 1);
            let frame = mgr.poll();
            assert_eq!(frame.values[0], 20.0);
            assert_eq!(frame.values[1], 0.0);
        }

        #[test]
        fn poll_defaults_to_zero_for_an_address_never_received() {
            let mut mgr = OscManager::new();
            mgr.bind_slot(0, "/never-sent".to_string(), 0);
            let frame = mgr.poll();
            assert_eq!(frame.values[0], 0.0);
        }

        #[test]
        fn poll_defaults_to_zero_when_the_bound_arg_index_is_out_of_range() {
            let mut mgr = OscManager::new();
            mgr.state
                .lock()
                .unwrap()
                .values
                .insert("/x".to_string(), vec![1.0]);
            mgr.bind_slot(0, "/x".to_string(), 5);
            let frame = mgr.poll();
            assert_eq!(frame.values[0], 0.0);
        }
    }
}

#[cfg(feature = "osc")]
pub use imp::*;
