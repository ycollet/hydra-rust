//! MIDI input, porting the real-world `hydra-midi` community extension
//! (https://github.com/arnoson/hydra-midi, loaded via `loadScript()` in
//! real hydra.js - there is no MIDI support in hydra-synth's own core at
//! all) natively, the same way `src/library.glsl`'s ported functions are:
//! `note()`/`cc()` and friends are registered directly in `eval.rs` rather
//! than staying no-ops.
//!
//! Deliberate simplifications from the real extension (each a "no worse
//! than a hard error" trade-off, matching e.g. `ease()`'s own "accepted
//! but not faithful" treatment elsewhere in this project):
//! - All MIDI channels and input devices are merged into one - `channel`/
//!   `input` arguments and `midi.channel(n)`/`.input(n)` are accepted but
//!   ignored. Real hydra-midi's own per-channel/per-input filtering needs
//!   a much bigger keying scheme (see its `getMidiWildcards`) for little
//!   real benefit in a typical one-controller setup.
//! - `.adsr(a,d,s,r)`'s envelope value is multiplied by the velocity
//!   captured at the moment the note was triggered, held for the envelope's
//!   whole lifetime (including release) - real hydra-midi's own JS instead
//!   reads the note's *live* velocity each time, which (as far as this
//!   port's author could tell from the minified closure chain) actually
//!   drops to 0 the instant the note is released, silencing the release
//!   phase entirely. That's very likely an upstream quirk rather than
//!   intended behavior; this port implements the musically-obvious
//!   interpretation instead.

pub const NUM_MIDI_NOTES: usize = 128;
pub const NUM_MIDI_CC: usize = 128;
/// A small fixed pool, one slot per `.adsr(...)` call site in a script -
/// mirrors `audio::NUM_FFT_BINS`'s own fixed-size-uniform-array approach.
pub const NUM_MIDI_ENVELOPES: usize = 16;
/// Real hydra-midi's own default when `.smooth()` is called with no
/// explicit factor.
pub const DEFAULT_CC_SMOOTH: f32 = 0.01;

#[cfg(feature = "midi")]
mod imp {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use midir::{MidiInput, MidiInputConnection};

    use super::{NUM_MIDI_CC, NUM_MIDI_ENVELOPES, NUM_MIDI_NOTES};

    fn ramp_factor(elapsed: f32, duration: f32) -> f32 {
        if duration <= 0.0 {
            1.0
        } else {
            (elapsed / duration).clamp(0.0, 1.0)
        }
    }

    fn linear_ramp(factor: f32, from: f32, to: f32) -> f32 {
        from + (to - from) * factor
    }

    /// Direct port of real hydra-midi's `lib/Envelope.ts` (attack/decay/
    /// sustain/release, linear ramps throughout), except the final value is
    /// multiplied by the velocity captured at `trigger()` rather than a
    /// live-read velocity - see the module doc comment.
    #[derive(Debug, Clone, Copy)]
    struct Envelope {
        active: bool,
        note_on: bool,
        start_time: Option<f32>,
        gate_duration: Option<f32>,
        velocity: f32,
        a: f32,
        d: f32,
        s: f32,
        r: f32,
    }

    impl Envelope {
        fn new(a: f32, d: f32, s: f32, r: f32) -> Self {
            Self {
                active: false,
                note_on: false,
                start_time: None,
                gate_duration: None,
                velocity: 0.0,
                a,
                d,
                s,
                r,
            }
        }

        fn trigger(&mut self, velocity: f32) {
            self.start_time = None;
            self.gate_duration = None;
            self.note_on = true;
            self.active = true;
            self.velocity = velocity;
        }

        fn stop(&mut self) {
            self.note_on = false;
        }

        fn value(&mut self, time_ms: f32) -> f32 {
            if !self.active {
                return 0.0;
            }
            let start = *self.start_time.get_or_insert(time_ms);
            let elapsed = time_ms - start;
            let (a, d, s) = (self.a, self.d, self.s);

            let raw = if elapsed < a {
                linear_ramp(ramp_factor(elapsed, a), 0.0, 1.0)
            } else if elapsed < a + d && s > 0.0 {
                linear_ramp(ramp_factor(elapsed - a, d), 1.0, s)
            } else if self.note_on && s > 0.0 {
                s
            } else {
                let gate_duration = *self.gate_duration.get_or_insert(elapsed);
                let factor = ramp_factor(elapsed - gate_duration, self.r);
                if factor >= 1.0 {
                    self.active = false;
                }
                let from = if s > 0.0 { s } else { 1.0 };
                linear_ramp(factor, from, 0.0)
            };

            raw * self.velocity
        }
    }

    #[derive(Clone, Copy)]
    struct EnvelopeSlot {
        note: i64,
        envelope: Envelope,
    }

    struct SharedState {
        note_velocity: [f32; NUM_MIDI_NOTES],
        cc_value: [f32; NUM_MIDI_CC],
        cc_smooth_factor: [f32; NUM_MIDI_CC],
        /// Polyphonic key pressure (status `0xA0`), per note - real
        /// hydra-midi's `aft(note)`.
        aftertouch: [f32; NUM_MIDI_NOTES],
        /// Channel pressure (status `0xD0`) - real hydra-midi's `aft()`
        /// with no note argument.
        channel_aftertouch: f32,
        slots: [Option<EnvelopeSlot>; NUM_MIDI_ENVELOPES],
    }

    impl SharedState {
        fn new() -> Self {
            Self {
                note_velocity: [0.0; NUM_MIDI_NOTES],
                cc_value: [0.0; NUM_MIDI_CC],
                cc_smooth_factor: [0.0; NUM_MIDI_CC],
                aftertouch: [0.0; NUM_MIDI_NOTES],
                channel_aftertouch: 0.0,
                slots: [None; NUM_MIDI_ENVELOPES],
            }
        }

        /// `velocity` is already normalized to `[0, 1]` - callers that
        /// only have a raw `0..127` MIDI byte (the system-MIDI-port path,
        /// see `handle_message` below) divide by 127 before calling this.
        fn note_on(&mut self, note: u8, velocity: f32) {
            if velocity <= 0.0 {
                // Many controllers send note-on with velocity 0 instead of
                // a real note-off - conventional MIDI shorthand.
                self.note_off(note);
                return;
            }
            let note = note as usize;
            if note >= NUM_MIDI_NOTES {
                return;
            }
            self.note_velocity[note] = velocity;
            for slot in self.slots.iter_mut().flatten() {
                if slot.note == note as i64 {
                    slot.envelope.trigger(velocity);
                }
            }
        }

        fn note_off(&mut self, note: u8) {
            let note = note as usize;
            if note >= NUM_MIDI_NOTES {
                return;
            }
            self.note_velocity[note] = 0.0;
            for slot in self.slots.iter_mut().flatten() {
                if slot.note == note as i64 {
                    slot.envelope.stop();
                }
            }
        }

        fn control_change(&mut self, index: u8, value: f32) {
            let index = index as usize;
            if index < NUM_MIDI_CC {
                self.cc_value[index] = value;
            }
        }

        fn poly_aftertouch(&mut self, note: u8, value: f32) {
            let note = note as usize;
            if note < NUM_MIDI_NOTES {
                self.aftertouch[note] = value;
            }
        }

        fn set_channel_aftertouch(&mut self, value: f32) {
            self.channel_aftertouch = value;
        }
    }

    /// A cheap, cloneable handle onto the shared MIDI state - the MIDI
    /// equivalent of `audio::AudioBuffer`. Feeding events into this
    /// doesn't require a system MIDI port at all: the desktop app's
    /// [`MidiManager`] uses one internally (fed by real `midir` input
    /// callbacks), while `hydra-plugin` feeds it directly from the host's
    /// own structured [`nih_plug::prelude::NoteEvent`]s instead, with no
    /// `midir`/system-MIDI-device dependency at all.
    #[derive(Clone)]
    pub struct MidiBridge(Arc<Mutex<SharedState>>);

    impl Default for MidiBridge {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MidiBridge {
        pub fn new() -> Self {
            let bridge = Self(Arc::new(Mutex::new(SharedState::new())));
            // Initialize macOS's lazily allocated native mutex off the audio thread.
            drop(bridge.0.lock().unwrap());
            bridge
        }

        /// `velocity` is normalized to `[0, 1]`, matching nih-plug's own
        /// `NoteEvent::NoteOn::velocity` representation directly - no
        /// `u8`-byte round trip needed for host-fed events.
        pub fn note_on(&self, note: u8, velocity: f32) {
            self.0.lock().unwrap().note_on(note, velocity);
        }

        pub fn note_off(&self, note: u8) {
            self.0.lock().unwrap().note_off(note);
        }

        /// Polyphonic key pressure, `pressure` normalized to `[0, 1]`.
        pub fn poly_pressure(&self, note: u8, pressure: f32) {
            self.0.lock().unwrap().poly_aftertouch(note, pressure);
        }

        /// A MIDI control change, `value` normalized to `[0, 1]`.
        pub fn control_change(&self, cc: u8, value: f32) {
            self.0.lock().unwrap().control_change(cc, value);
        }

        /// Channel pressure, `pressure` normalized to `[0, 1]`.
        pub fn channel_pressure(&self, pressure: f32) {
            self.0.lock().unwrap().set_channel_aftertouch(pressure);
        }

        pub fn set_cc_smooth(&self, index: usize, factor: f32) {
            if index < NUM_MIDI_CC {
                self.0.lock().unwrap().cc_smooth_factor[index] = factor;
            }
        }

        /// (Re)assigns envelope slot `slot` to watch `note`, with the given
        /// ADSR parameters (milliseconds for a/d/r, a 0-1 sustain level).
        pub fn set_adsr_slot(&self, slot: usize, note: i64, a: f32, d: f32, s: f32, r: f32) {
            if slot >= NUM_MIDI_ENVELOPES {
                return;
            }
            let mut state = self.0.lock().unwrap();
            state.slots[slot] = Some(EnvelopeSlot {
                note,
                envelope: Envelope::new(a, d, s, r),
            });
        }

        /// Parses a raw 2-3 byte MIDI message (as delivered by a system
        /// MIDI port via `midir`) and applies it - only used by
        /// [`MidiManager::ensure_started`]'s connection callback.
        fn handle_raw_message(&self, message: &[u8]) {
            let Some(&status) = message.first() else {
                return;
            };
            let kind = status & 0xF0;
            match kind {
                0x90 if message.len() >= 3 => self.note_on(message[1], message[2] as f32 / 127.0),
                0x80 if message.len() >= 3 => self.note_off(message[1]),
                0xB0 if message.len() >= 3 => {
                    self.control_change(message[1], message[2] as f32 / 127.0)
                }
                // Polyphonic key pressure - two data bytes (note, pressure).
                0xA0 if message.len() >= 3 => {
                    self.poly_pressure(message[1], message[2] as f32 / 127.0)
                }
                // Channel pressure - only *one* data byte (pressure),
                // unlike every message kind above - a real 2-byte MIDI
                // message, not a truncated 3-byte one.
                0xD0 if message.len() >= 2 => self.channel_pressure(message[1] as f32 / 127.0),
                _ => {}
            }
        }
    }

    /// A snapshot of every reactive MIDI value, uploaded as uniform arrays
    /// once per rendered frame - mirrors `audio::AudioManager::poll`'s
    /// per-frame-snapshot shape.
    pub struct MidiFrame {
        pub note: [f32; NUM_MIDI_NOTES],
        pub velocity: [f32; NUM_MIDI_NOTES],
        pub cc: [f32; NUM_MIDI_CC],
        pub cc_smoothed: [f32; NUM_MIDI_CC],
        pub envelope: [f32; NUM_MIDI_ENVELOPES],
        pub aftertouch: [f32; NUM_MIDI_NOTES],
        pub channel_aftertouch: f32,
    }

    /// Per-frame polling state (CC smoothing accumulator, envelope clock) -
    /// kept separate from [`MidiBridge`] so each render thread/window can
    /// own its own smoothing state while all sharing the same underlying
    /// note/CC/aftertouch values. Mirrors the
    /// bridge/analyzer split `audio::AudioBuffer`/`audio::AudioAnalyzer`
    /// already use.
    pub struct MidiPoller {
        bridge: MidiBridge,
        start_time: Instant,
        cc_smoothed: [f32; NUM_MIDI_CC],
        cc_smoothed_init: [bool; NUM_MIDI_CC],
    }

    impl MidiPoller {
        pub fn new(bridge: MidiBridge) -> Self {
            Self {
                bridge,
                start_time: Instant::now(),
                cc_smoothed: [0.0; NUM_MIDI_CC],
                cc_smoothed_init: [false; NUM_MIDI_CC],
            }
        }

        pub fn poll(&mut self) -> MidiFrame {
            let mut state = self.bridge.0.lock().unwrap();

            let mut note = [0.0; NUM_MIDI_NOTES];
            for (i, v) in state.note_velocity.iter().enumerate() {
                note[i] = if *v > 0.0 { 1.0 } else { 0.0 };
            }

            for i in 0..NUM_MIDI_CC {
                let target = state.cc_value[i];
                let factor = if state.cc_smooth_factor[i] > 0.0 {
                    state.cc_smooth_factor[i]
                } else {
                    super::DEFAULT_CC_SMOOTH
                };
                if !self.cc_smoothed_init[i] {
                    self.cc_smoothed_init[i] = true;
                    self.cc_smoothed[i] = target;
                } else {
                    self.cc_smoothed[i] += (target - self.cc_smoothed[i]) * factor;
                }
            }

            let elapsed_ms = self.start_time.elapsed().as_secs_f32() * 1000.0;
            let mut envelope = [0.0; NUM_MIDI_ENVELOPES];
            for (i, slot) in state.slots.iter_mut().enumerate() {
                if let Some(slot) = slot {
                    envelope[i] = slot.envelope.value(elapsed_ms);
                }
            }

            MidiFrame {
                note,
                velocity: state.note_velocity,
                cc: state.cc_value,
                cc_smoothed: self.cc_smoothed,
                envelope,
                aftertouch: state.aftertouch,
                channel_aftertouch: state.channel_aftertouch,
            }
        }
    }

    /// The desktop app's own MIDI backend: connects directly to system
    /// MIDI input ports via `midir`. `hydra-plugin` does *not* use this -
    /// it feeds host-provided events straight into a [`MidiBridge`]
    /// instead (see the crate's own `lib.rs`), since the host (not this
    /// process) owns MIDI routing for a plugin.
    pub struct MidiManager {
        bridge: MidiBridge,
        poller: MidiPoller,
        connections: Vec<MidiInputConnection<()>>,
        started: bool,
    }

    impl Default for MidiManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MidiManager {
        pub fn new() -> Self {
            let bridge = MidiBridge::new();
            Self {
                poller: MidiPoller::new(bridge.clone()),
                bridge,
                connections: Vec::new(),
                started: false,
            }
        }

        /// Connects to every currently-available MIDI input port (see the
        /// module doc comment on why devices aren't distinguished). Not
        /// called automatically - only once a script actually calls
        /// `midi.start()`, the same lazy-start-on-script-intent treatment
        /// `AudioManager::ensure_started` gives the microphone.
        pub fn ensure_started(&mut self) {
            if self.started {
                return;
            }
            self.started = true;

            let Ok(probe) = MidiInput::new("hydra-rust-probe") else {
                log::warn!("midi.start(): failed to initialize MIDI input");
                return;
            };
            for port in probe.ports() {
                let name = probe.port_name(&port).unwrap_or_else(|_| "unknown".into());
                let Ok(mut input) = MidiInput::new("hydra-rust") else {
                    continue;
                };
                input.ignore(midir::Ignore::None);
                let bridge = self.bridge.clone();
                match input.connect(
                    &port,
                    "hydra-rust-read",
                    move |_timestamp, message, _| {
                        bridge.handle_raw_message(message);
                    },
                    (),
                ) {
                    Ok(conn) => {
                        log::info!("midi: connected to \"{name}\"");
                        self.connections.push(conn);
                    }
                    Err(e) => log::warn!("midi: failed to connect to \"{name}\": {e}"),
                }
            }
        }

        pub fn pause(&mut self) {
            self.connections.clear();
            self.started = false;
        }

        pub fn set_cc_smooth(&mut self, index: usize, factor: f32) {
            self.bridge.set_cc_smooth(index, factor);
        }

        /// (Re)assigns envelope slot `slot` to watch `note`, with the given
        /// ADSR parameters (milliseconds for a/d/r, a 0-1 sustain level).
        pub fn set_adsr_slot(&mut self, slot: usize, note: i64, a: f32, d: f32, s: f32, r: f32) {
            self.bridge.set_adsr_slot(slot, note, a, d, s, r);
        }

        pub fn poll(&mut self) -> MidiFrame {
            self.poller.poll()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{Envelope, MidiBridge};

        #[test]
        fn poly_aftertouch_sets_the_given_notes_value() {
            let bridge = MidiBridge::new();
            bridge.handle_raw_message(&[0xA0, 60, 100]);
            assert_eq!(bridge.0.lock().unwrap().aftertouch[60], 100.0 / 127.0);
        }

        #[test]
        fn channel_aftertouch_sets_the_shared_value_from_a_two_byte_message() {
            let bridge = MidiBridge::new();
            bridge.handle_raw_message(&[0xD0, 64]);
            assert_eq!(bridge.0.lock().unwrap().channel_aftertouch, 64.0 / 127.0);
        }

        #[test]
        fn aftertouch_on_different_notes_is_independent() {
            let bridge = MidiBridge::new();
            bridge.handle_raw_message(&[0xA0, 10, 20]);
            bridge.handle_raw_message(&[0xA0, 11, 40]);
            let guard = bridge.0.lock().unwrap();
            assert_eq!(guard.aftertouch[10], 20.0 / 127.0);
            assert_eq!(guard.aftertouch[11], 40.0 / 127.0);
        }

        #[test]
        fn a_truncated_message_of_an_otherwise_three_byte_kind_is_ignored_not_a_panic() {
            let bridge = MidiBridge::new();
            bridge.handle_raw_message(&[0x90, 60]); // note-on missing its velocity byte
            bridge.handle_raw_message(&[0xA0, 60]); // poly aftertouch missing its value byte
            let guard = bridge.0.lock().unwrap();
            assert_eq!(guard.note_velocity[60], 0.0);
            assert_eq!(guard.aftertouch[60], 0.0);
        }

        // `Envelope::value`'s first call after `trigger()` lazily anchors
        // `start_time` to *that call's own argument* (matching real
        // Envelope.ts's `this.startTime ??= time`) - every test below
        // makes an initial `value(0.0)` call for exactly this reason, so
        // every subsequent argument is elapsed milliseconds since trigger,
        // not a raw wall-clock reading.

        #[test]
        fn inactive_envelope_is_silent() {
            let mut env = Envelope::new(100.0, 100.0, 1.0, 100.0);
            assert_eq!(env.value(0.0), 0.0);
        }

        #[test]
        fn attack_ramps_linearly_from_zero_to_one() {
            let mut env = Envelope::new(100.0, 100.0, 1.0, 100.0);
            env.trigger(1.0);
            assert_eq!(env.value(0.0), 0.0);
            assert_eq!(env.value(50.0), 0.5);
        }

        #[test]
        fn decay_ramps_from_one_to_sustain_level() {
            let mut env = Envelope::new(100.0, 100.0, 0.5, 100.0);
            env.trigger(1.0);
            env.value(0.0); // attack start
            assert_eq!(env.value(100.0), 1.0); // attack complete, decay start
            assert_eq!(env.value(150.0), 0.75); // halfway through decay
            assert_eq!(env.value(200.0), 0.5); // decay complete, at sustain
        }

        #[test]
        fn sustain_holds_while_the_note_is_still_on() {
            let mut env = Envelope::new(100.0, 100.0, 0.5, 100.0);
            env.trigger(1.0);
            env.value(0.0);
            assert_eq!(env.value(500.0), 0.5);
            assert_eq!(env.value(10_000.0), 0.5);
        }

        #[test]
        fn release_starts_from_one_when_sustain_level_is_zero() {
            // real Envelope.ts: "If there was no sustain, there also was no
            // decay so we can start the release at 1.0".
            let mut env = Envelope::new(100.0, 100.0, 0.0, 100.0);
            env.trigger(1.0);
            env.value(0.0); // attack start
            assert_eq!(env.value(100.0), 1.0); // attack complete; s=0 skips
            // decay/sustain entirely, release begins here at `from = 1.0`
            assert_eq!(env.value(150.0), 0.5); // halfway through release
            assert_eq!(env.value(200.0), 0.0); // release complete
        }

        #[test]
        fn release_ramps_to_zero_after_note_off() {
            let mut env = Envelope::new(100.0, 100.0, 0.5, 100.0);
            env.trigger(1.0);
            env.value(0.0); // attack start
            env.stop(); // released while conceptually sustaining
            assert_eq!(env.value(300.0), 0.5); // release begins here, at `s`
            assert_eq!(env.value(350.0), 0.25); // halfway through release
            assert_eq!(env.value(400.0), 0.0); // release complete
        }

        #[test]
        fn release_gate_duration_anchors_to_when_release_first_began() {
            // the release ramp's own duration is measured from whenever
            // release was *first evaluated*, not from `stop()` itself or
            // from any later `value()` call.
            let mut env = Envelope::new(0.0, 0.0, 0.0, 100.0);
            env.trigger(1.0);
            env.stop();
            assert_eq!(env.value(0.0), 1.0); // release begins immediately
            assert_eq!(env.value(50.0), 0.5); // still anchored to t=0
        }

        #[test]
        fn envelope_becomes_inactive_once_release_finishes() {
            let mut env = Envelope::new(0.0, 0.0, 0.0, 100.0);
            env.trigger(1.0);
            env.stop();
            env.value(0.0);
            assert_eq!(env.value(100.0), 0.0);
            // Stays silent afterward rather than restarting.
            assert_eq!(env.value(1000.0), 0.0);
        }

        #[test]
        fn value_is_scaled_by_the_velocity_captured_at_trigger() {
            let mut env = Envelope::new(100.0, 100.0, 1.0, 100.0);
            env.trigger(0.5);
            env.value(0.0); // attack start
            assert_eq!(env.value(100.0), 0.5); // attack complete (raw 1.0) * 0.5
        }

        #[test]
        fn zero_duration_stages_are_instantaneous() {
            let mut env = Envelope::new(0.0, 0.0, 1.0, 0.0);
            env.trigger(1.0);
            assert_eq!(env.value(0.0), 1.0);
        }
    }
}

#[cfg(feature = "midi")]
pub use imp::*;
