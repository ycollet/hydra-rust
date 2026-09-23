//! Receives a video stream from another hydra-rust instance for
//! `initStream()` (see eval.rs), delivering it through the same
//! texture-slot upload path webcam/image/GIF/video frames use
//! (`ShaderRenderer::upload_source`, see renderer.rs).
//!
//! This is **not** interoperable with real hydra.js's own `initStream`/
//! `pb.setName()` - that feature is itself currently broken in the live
//! hydra.js editor (its signaling server hasn't been touched since 2024),
//! its wire protocol (`socket.io` + `simple-peer`) is bespoke and
//! undocumented, and there is no real-world session to target even if it
//! weren't. This is a **hydra-rust-to-hydra-rust** feature instead: one
//! instance runs the `examples/webrtc_broadcast.rs` companion tool (not a
//! Rhai function - see that file), the other calls `s0.initStream("host:port")`
//! to receive it.
//!
//! Signaling is a single direct TCP connection rather than a relay server -
//! this instance connects to `addr` (the broadcaster must already be
//! listening there), reads one SDP offer, sends back one SDP answer once
//! its own ICE candidate gathering completes (non-trickle ICE - simpler
//! than juggling a live signaling channel for individual candidates), then
//! the TCP connection's job is done; actual media flows over the
//! separately-negotiated ICE/UDP path.
//!
//! **No STUN server** - deliberately, found via real end-to-end testing,
//! not just theory: a public STUN server makes the ICE agent gather a
//! server-reflexive candidate (this machine's public NAT address) alongside
//! its host candidates, and on this network that public candidate got
//! selected over the working host/loopback one, breaking the connection
//! outright (`Message too long`, since the write path to that address
//! wasn't actually usable here). This project's target use case - two
//! hydra-rust instances given each other's direct IP:port, typically on the
//! same LAN or even the same machine - never needs NAT traversal at all;
//! host candidates alone connect directly. Traversing separate NATs over
//! the open internet is out of scope for this direct-address design (it
//! would need a TURN relay, not just STUN, to be reliable anyway).
//!
//! Video is always VP8 (the only codec this module registers, since both
//! ends are always hydra-rust - there's no browser to inter-negotiate
//! with). `webrtc-rs` itself never decodes video to pixels (confirmed
//! against its own examples - it only handles ICE/DTLS/SRTP transport and
//! RTP relaying), so decoding reuses the same standalone `ffmpeg`
//! subprocess `video.rs` already depends on: incoming RTP packets are
//! relayed verbatim to a local UDP socket that `ffmpeg` listens on (told
//! the negotiated codec/payload type via a small generated SDP file), and
//! `ffmpeg`'s decoded `rawvideo` output is read exactly like
//! `VideoManager::init_video` already does. `ffmpeg` is always the codec
//! engine; `webrtc-rs` is always just the transport - this feature adds no
//! second video-codec library on top of the one hydra-rust already needs.

#[cfg(feature = "stream")]
mod imp {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpStream, UdpSocket};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use ffmpeg_sidecar::command::FfmpegCommand;
    use rtc::interceptor::Registry;
    use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
    use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_VP8};
    use rtc::peer_connection::configuration::RTCConfigurationBuilder;
    use rtc::peer_connection::sdp::RTCSessionDescription;
    use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind};
    use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
    use rtc::shared::marshal::Marshal;
    use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
    use webrtc::peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    };
    use webrtc::runtime::{channel, AsyncUdpSocket, Runtime, Sender};

    use crate::source::{SourceFrame, NUM_SOURCES};

    pub const VP8_PAYLOAD_TYPE: u8 = 96;
    pub const VP8_CLOCK_RATE: u32 = 90000;

    pub fn vp8_media_engine() -> Result<(MediaEngine, Registry), String> {
        let mut media_engine = MediaEngine::default();
        let video_codec = RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP8.to_owned(),
                clock_rate: VP8_CLOCK_RATE,
                channels: 0,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: VP8_PAYLOAD_TYPE,
        };
        media_engine
            .register_codec(video_codec, RtpCodecKind::Video)
            .map_err(|e| e.to_string())?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)
            .map_err(|e| e.to_string())?;
        Ok((media_engine, registry))
    }

    /// No ICE (STUN/TURN) servers - see the module doc comment for why:
    /// this feature's direct-address design never needs NAT traversal, and
    /// a STUN server actively broke connectivity in real testing on this
    /// network (a server-reflexive candidate got selected over the working
    /// host one).
    pub fn ice_config() -> RTCConfigurationBuilder {
        RTCConfigurationBuilder::new()
    }

    /// A one-shot SDP file describing an incoming RTP/VP8 stream on
    /// `127.0.0.1:<port>`, for `ffmpeg` to listen on and decode - written
    /// once per `init_stream` call into the OS temp directory.
    pub(crate) fn write_rtp_sdp_file(port: u16) -> std::io::Result<std::path::PathBuf> {
        let path = std::env::temp_dir().join(format!("hydra-rust-stream-{port}.sdp"));
        let sdp = format!(
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=hydra-rust\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=video {port} RTP/AVP {VP8_PAYLOAD_TYPE}\r\na=rtpmap:{VP8_PAYLOAD_TYPE} VP8/{VP8_CLOCK_RATE}\r\n"
        );
        std::fs::write(&path, sdp)?;
        Ok(path)
    }

    struct StreamSlot {
        stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
        latest: Arc<Mutex<Option<SourceFrame>>>,
    }

    impl Drop for StreamSlot {
        fn drop(&mut self) {
            // Tells the receive session's background task to close the
            // peer connection and stop; the ffmpeg child it owns is
            // dropped (and killed, per FfmpegChild's own Drop) along with
            // it. Ignored if the receiver end already hung up (session
            // already ended on its own, e.g. the peer disconnected).
            if let Some(tx) = self.stop_tx.take() {
                let _ = tx.send(());
            }
        }
    }

    #[derive(Default)]
    pub struct StreamManager {
        slots: [Option<StreamSlot>; NUM_SOURCES],
        warned_missing_ffmpeg: bool,
    }

    impl StreamManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn init_stream(&mut self, slot: usize, addr: String) {
            if slot >= NUM_SOURCES {
                return;
            }
            if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
                if !self.warned_missing_ffmpeg {
                    self.warned_missing_ffmpeg = true;
                    log::warn!(
                        "initStream: `ffmpeg` was not found on PATH - install it \
                         (e.g. `brew install ffmpeg`, `apt install ffmpeg`) to decode incoming streams"
                    );
                }
                return;
            }

            // Replace whatever was previously streaming into this slot -
            // the outgoing `StreamSlot`'s `Drop` impl stops its session.
            self.slots[slot] = None;

            let latest = Arc::new(Mutex::new(None));
            let latest_writer = latest.clone();
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();

            thread::Builder::new()
                .name(format!("hydra-stream-{slot}"))
                .spawn(move || run_receiver(addr, latest_writer, stop_rx))
                .expect("spawn stream-receive thread");

            self.slots[slot] = Some(StreamSlot { stop_tx: Some(stop_tx), latest });
        }

        /// Returns the newest decoded frame for `slot`, if the background
        /// decode pipeline has produced one since the last call.
        pub fn poll(&mut self, slot: usize) -> Option<SourceFrame> {
            self.slots.get(slot)?.as_ref()?.latest.lock().unwrap().take()
        }

        /// Stops every active stream - called on app exit, mirroring
        /// `VideoManager::stop_all`.
        pub fn stop_all(&mut self) {
            for slot in &mut self.slots {
                *slot = None;
            }
        }
    }

    struct Handler {
        runtime: Arc<dyn Runtime>,
        gather_complete_tx: Sender<()>,
        connected_tx: Sender<()>,
        rtp_forward_addr: std::net::SocketAddr,
    }

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for Handler {
        async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
            if state == RTCIceGatheringState::Complete {
                let _ = self.gather_complete_tx.try_send(());
            }
        }

        async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
            log::info!("initStream: connection state {state}");
            if state == RTCPeerConnectionState::Connected {
                let _ = self.connected_tx.try_send(());
            }
        }

        async fn on_track(&self, track: Arc<dyn TrackRemote>) {
            let forward_addr = self.rtp_forward_addr;
            let std_sock = match UdpSocket::bind("127.0.0.1:0") {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("initStream: failed to bind RTP-forward socket: {e}");
                    return;
                }
            };
            let sock: Arc<dyn AsyncUdpSocket> = match self.runtime.wrap_udp_socket(std_sock) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("initStream: failed to wrap RTP-forward socket: {e}");
                    return;
                }
            };
            self.runtime.spawn(Box::pin(async move {
                let mut buf = vec![0u8; 1500];
                while let Some(evt) = track.poll().await {
                    if let TrackRemoteEvent::OnRtpPacket(packet) = evt
                        && let Ok(n) = packet.marshal_to(&mut buf)
                        && sock.send_to(&buf[..n], forward_addr).await.is_err()
                    {
                        break;
                    }
                }
            }));
        }
    }

    /// Runs on its own dedicated OS thread (one per active slot) - owns a
    /// `webrtc-rs` runtime for as long as the stream is active, mirroring
    /// the "one thread owns one continuous resource" shape already used
    /// for the camera/audio/MIDI/video managers.
    fn run_receiver(
        addr: String,
        latest: Arc<Mutex<Option<SourceFrame>>>,
        stop_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        let Some(runtime) = webrtc::runtime::default_runtime() else {
            log::warn!("initStream: no async runtime available");
            return;
        };
        let runtime_for_session = runtime.clone();
        runtime.block_on(Box::pin(async move {
            if let Err(e) = receive_session(addr, latest, runtime_for_session, stop_rx).await {
                log::warn!("initStream: session ended: {e}");
            }
        }));
    }

    async fn receive_session(
        addr: String,
        latest: Arc<Mutex<Option<SourceFrame>>>,
        runtime: Arc<dyn Runtime>,
        stop_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<(), String> {
        // The signaling handshake below is a handful of short, one-shot
        // reads/writes on a dedicated thread with exactly one task ever
        // running on it - using plain blocking `std::net::TcpStream` here
        // (rather than an async socket) never risks starving anything else,
        // unlike it would on a shared multi-tasked runtime.
        let stream = TcpStream::connect(&addr).map_err(|e| format!("connect to {addr}: {e}"))?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut writer = stream;

        let (media_engine, registry) = vp8_media_engine()?;
        let config = ice_config().build();

        let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
        let (connected_tx, mut connected_rx) = channel::<()>(1);

        // The RTP-forwarding port is chosen up front so the ffmpeg SDP file
        // (below) can name it before any packets arrive.
        let rtp_port = pick_udp_port().map_err(|e| format!("pick RTP port: {e}"))?;
        let rtp_forward_addr: std::net::SocketAddr =
            format!("127.0.0.1:{rtp_port}").parse().map_err(|e: std::net::AddrParseError| e.to_string())?;

        let handler = Arc::new(Handler {
            runtime: runtime.clone(),
            gather_complete_tx,
            connected_tx,
            rtp_forward_addr,
        });

        let peer_connection = PeerConnectionBuilder::new()
            .with_configuration(config)
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_handler(Arc::clone(&handler) as Arc<dyn PeerConnectionEventHandler>)
            .with_runtime(runtime.clone())
            .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
            .build()
            .await
            .map_err(|e| format!("build peer connection: {e}"))?;

        peer_connection
            .add_transceiver_from_kind(
                RtpCodecKind::Video,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| format!("add recvonly transceiver: {e}"))?;

        let mut offer_line = String::new();
        reader.read_line(&mut offer_line).map_err(|e| format!("read offer: {e}"))?;
        let offer: RTCSessionDescription =
            serde_json::from_str(offer_line.trim()).map_err(|e| format!("parse offer: {e}"))?;
        peer_connection
            .set_remote_description(offer)
            .await
            .map_err(|e| format!("set remote description: {e}"))?;
        let answer = peer_connection
            .create_answer(None)
            .await
            .map_err(|e| format!("create answer: {e}"))?;
        peer_connection
            .set_local_description(answer)
            .await
            .map_err(|e| format!("set local description: {e}"))?;

        let _ = gather_complete_rx.recv().await;
        let local_desc = peer_connection
            .local_description()
            .await
            .ok_or_else(|| "no local description after ICE gathering".to_string())?;
        let answer_json = serde_json::to_string(&local_desc).map_err(|e| e.to_string())?;
        writeln!(writer, "{answer_json}").map_err(|e| format!("write answer: {e}"))?;

        // Only spin up ffmpeg once the connection actually comes up - no
        // point decoding a stream that never arrives.
        let _ = connected_rx.recv().await;
        let ffmpeg_child = spawn_ffmpeg_decoder(rtp_port, latest)?;

        let _ = stop_rx.await;
        drop(ffmpeg_child);
        let _ = peer_connection.close().await;
        Ok(())
    }

    fn pick_udp_port() -> std::io::Result<u16> {
        Ok(UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port())
    }

    fn spawn_ffmpeg_decoder(
        rtp_port: u16,
        latest: Arc<Mutex<Option<SourceFrame>>>,
    ) -> Result<ffmpeg_sidecar::child::FfmpegChild, String> {
        let sdp_path = write_rtp_sdp_file(rtp_port).map_err(|e| format!("write SDP file: {e}"))?;

        let mut command = FfmpegCommand::new();
        command
            .args(["-protocol_whitelist", "file,udp,rtp"])
            .input(sdp_path.to_string_lossy().as_ref())
            .rawvideo();

        let mut child = command.spawn().map_err(|e| format!("spawn ffmpeg: {e}"))?;
        let iter = child.iter().map_err(|e| {
            let _ = child.kill();
            format!("read ffmpeg output: {e}")
        })?;

        thread::Builder::new()
            .name(format!("hydra-stream-decode-{rtp_port}"))
            .spawn(move || {
                for frame in iter.filter_frames() {
                    let source_frame = SourceFrame {
                        pixels: frame.data,
                        width: frame.width,
                        height: frame.height,
                    };
                    *latest.lock().unwrap() = Some(source_frame);
                }
                let _ = std::fs::remove_file(&sdp_path);
            })
            .expect("spawn stream-decode thread");

        Ok(child)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn init_stream_on_an_out_of_range_slot_does_not_panic() {
            let mut mgr = StreamManager::new();
            mgr.init_stream(NUM_SOURCES, "127.0.0.1:1".into());
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn poll_on_a_never_used_slot_is_none() {
            let mut mgr = StreamManager::new();
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn stop_all_clears_every_slot() {
            let mut mgr = StreamManager::new();
            mgr.init_stream(0, "127.0.0.1:1".into());
            mgr.stop_all();
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn init_stream_with_an_unreachable_address_never_panics() {
            let mut mgr = StreamManager::new();
            // Port 1 is a reserved/unlikely-to-be-listening port - the
            // background thread's TCP connect should just fail and log,
            // never panic, regardless of whether ffmpeg is installed.
            mgr.init_stream(0, "127.0.0.1:1".into());
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(mgr.poll(0).is_none());
        }

        #[test]
        fn vp8_media_engine_registers_without_error() {
            assert!(vp8_media_engine().is_ok());
        }

        #[test]
        fn rtp_sdp_file_contains_the_chosen_port_and_payload_type() {
            let path = write_rtp_sdp_file(54321).expect("write sdp file");
            let contents = std::fs::read_to_string(&path).expect("read sdp file");
            assert!(contents.contains("m=video 54321 RTP/AVP 96"));
            assert!(contents.contains("a=rtpmap:96 VP8/90000"));
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(feature = "stream")]
pub use imp::*;
