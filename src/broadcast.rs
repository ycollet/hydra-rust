//! Broadcasts this app's own rendered output for `broadcastStream(port)`/
//! `stopBroadcast()` (see eval.rs) - the send-side counterpart to
//! `stream.rs`'s `StreamManager`, and the scripted equivalent of the
//! standalone `examples/webrtc_broadcast.rs` tool (which predates this and
//! remains useful for broadcasting a webcam/file/test pattern without
//! running the full app at all - the two don't share more code than the
//! `vp8_media_engine`/`ice_config` helpers already do, since one has
//! `ffmpeg` capture its own input while this one is *fed* frames the app
//! already rendered).
//!
//! Unlike `StreamManager` (one manager, `NUM_SOURCES` independent slots),
//! there's only ever one broadcast: this app's own single rendered output,
//! analogous to real hydra.js's own single `pb.setName()` per page. Any
//! number of viewers can connect, though - each gets its own
//! `RTCPeerConnection`/DTLS-SRTP session (WebRTC has no concept of one
//! connection with multiple remote peers), but they all share a *single*
//! `ffmpeg` encode, fanned out by cloning each encoded RTP packet
//! (`rtc::rtp::packet::Packet` derives `Clone`) to every connected viewer's
//! own `TrackLocalStaticRTP`. This matters because VP8 encoding is the
//! proven CPU bottleneck here (see `MAX_BROADCAST_WIDTH`'s own doc
//! comment), and running one independent encode per viewer would multiply
//! exactly the cost already fixed once.
//!
//! Frames arrive via `push_frame`, called from the GL render callback
//! (`app.rs::paint_background`) with whatever was just drawn to the actual
//! window - i.e. exactly what real hydra.js's `canvas.captureStream()`
//! would capture, respecting `render()`'s current mode. `ffmpeg` (already a
//! dependency via `video`/`stream`) is fed those frames over its own stdin
//! (`-f rawvideo -i pipe:0`) to encode them to VP8/RTP, the mirror image of
//! how `stream.rs` reads `ffmpeg`'s decoded output - `webrtc-rs` still only
//! ever handles transport, never codec work, in either direction.

#[cfg(feature = "stream")]
mod imp {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream, UdpSocket};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use bytes::BytesMut;
    use ffmpeg_sidecar::command::FfmpegCommand;
    use rtc::media_stream::MediaStreamTrack;
    use rtc::peer_connection::sdp::RTCSessionDescription;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::shared::marshal::Unmarshal;
    use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
    use webrtc::media_stream::track_local::TrackLocal;
    use webrtc::peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    };
    use webrtc::runtime::{channel, AsyncUdpSocket, Runtime, Sender};

    use crate::stream::{ice_config, vp8_media_engine, VP8_CLOCK_RATE, VP8_PAYLOAD_TYPE};

    type FrameSlot = Mutex<Option<(u32, u32, Vec<u8>)>>;

    /// Flips GL's bottom-up row order to the top-down order `ffmpeg`/RTP
    /// expect, and stashes the result in `sink` (latest-frame-wins, like
    /// every other manager here) - a free function, not a `BroadcastManager`
    /// method, so the GL render callback (which only holds a cloned `Arc`,
    /// not the whole manager - see `frame_sink`) can call it directly.
    pub fn push_captured_frame(sink: &FrameSlot, width: u32, height: u32, rgb_bottom_up: &[u8]) {
        let row_bytes = (width as usize) * 3;
        if rgb_bottom_up.len() != row_bytes * height as usize {
            return;
        }
        let mut flipped = vec![0u8; rgb_bottom_up.len()];
        for y in 0..height as usize {
            let src = &rgb_bottom_up[y * row_bytes..(y + 1) * row_bytes];
            let dst_y = height as usize - 1 - y;
            flipped[dst_y * row_bytes..(dst_y + 1) * row_bytes].copy_from_slice(src);
        }
        *sink.lock().unwrap() = Some((width, height, flipped));
    }

    struct BroadcastState {
        stopped: Arc<AtomicBool>,
        latest_frame: Arc<FrameSlot>,
    }

    impl Drop for BroadcastState {
        fn drop(&mut self) {
            // Tells the broadcast thread's frame-feed loop to stop, kill
            // ffmpeg, and close the peer connection - see run_broadcast.
            self.stopped.store(true, Ordering::Relaxed);
        }
    }

    #[derive(Default)]
    pub struct BroadcastManager {
        state: Option<BroadcastState>,
    }

    impl BroadcastManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn is_active(&self) -> bool {
            self.state.is_some()
        }

        /// A clone of the shared frame slot, if actively broadcasting - for
        /// the GL render callback to capture and call `push_captured_frame`
        /// on directly, without needing `&mut self` (the callback is
        /// `'static` and can't borrow the app).
        pub fn frame_sink(&self) -> Option<Arc<FrameSlot>> {
            self.state.as_ref().map(|s| s.latest_frame.clone())
        }

        /// Starts listening on `port` in the background - a no-op if
        /// already broadcasting (call `stop()` first to change ports).
        pub fn start(&mut self, port: u16) {
            if self.state.is_some() {
                return;
            }
            if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
                log::warn!(
                    "broadcastStream: `ffmpeg` was not found on PATH - install it \
                     (e.g. `brew install ffmpeg`, `apt install ffmpeg`) to broadcast"
                );
                return;
            }

            let latest_frame: Arc<FrameSlot> = Arc::new(Mutex::new(None));
            let stopped = Arc::new(AtomicBool::new(false));

            let frame_slot = latest_frame.clone();
            let stop_flag = stopped.clone();
            thread::Builder::new()
                .name("hydra-broadcast".into())
                .spawn(move || run_broadcast(port, frame_slot, stop_flag))
                .expect("spawn broadcast thread");

            self.state = Some(BroadcastState { stopped, latest_frame });
        }

        /// Convenience wrapper around `push_captured_frame` for callers that
        /// do hold `&mut self` - the GL render callback uses `frame_sink()`
        /// + the free function directly instead (see its own doc comment).
        pub fn push_frame(&mut self, width: u32, height: u32, rgb_bottom_up: &[u8]) {
            if let Some(state) = &self.state {
                push_captured_frame(&state.latest_frame, width, height, rgb_bottom_up);
            }
        }

        pub fn stop(&mut self) {
            self.state = None;
        }
    }

    struct Handler {
        gather_complete_tx: Sender<()>,
        connected_tx: Sender<()>,
    }

    #[async_trait::async_trait]
    impl PeerConnectionEventHandler for Handler {
        async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
            if state == RTCIceGatheringState::Complete {
                let _ = self.gather_complete_tx.try_send(());
            }
        }
        async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
            log::info!("broadcastStream: connection state {state}");
            if state == RTCPeerConnectionState::Connected {
                let _ = self.connected_tx.try_send(());
            }
        }
    }

    /// One connected viewer - its own `RTCPeerConnection` (stored as
    /// `Arc<dyn PeerConnection>`, per `PeerConnectionBuilder::build`'s own
    /// documented pattern for sharing a connection across tasks/state) so
    /// `stop()` can close it, plus the `TrackLocalStaticRTP` the shared
    /// `ffmpeg` encode is fanned out to.
    struct Viewer {
        peer_connection: Arc<dyn PeerConnection>,
        track: Arc<TrackLocalStaticRTP>,
    }
    type Viewers = Mutex<Vec<Viewer>>;

    /// Given each currently-registered viewer's `write_rtp` outcome (in
    /// registry order), returns which indices failed (disconnected),
    /// **descending** so the caller can repeatedly `Vec::remove` them
    /// without earlier removals invalidating later indices. A free
    /// function purely so this bit of indexing logic is testable without a
    /// real WebRTC track.
    fn indices_to_prune(write_results: &[Result<(), ()>]) -> Vec<usize> {
        write_results.iter().enumerate().filter(|(_, r)| r.is_err()).map(|(i, _)| i).rev().collect()
    }

    /// Runs on its own dedicated OS thread for as long as the broadcast is
    /// active - mirrors `stream.rs::run_receiver`'s shape.
    fn run_broadcast(port: u16, latest_frame: Arc<FrameSlot>, stopped: Arc<AtomicBool>) {
        let Some(runtime) = webrtc::runtime::default_runtime() else {
            log::warn!("broadcastStream: no async runtime available");
            return;
        };
        let runtime_for_session = runtime.clone();
        runtime.block_on(Box::pin(async move {
            if let Err(e) = broadcast_session(port, latest_frame, stopped, runtime_for_session).await {
                log::warn!("broadcastStream: session ended: {e}");
            }
        }));
    }

    async fn broadcast_session(
        port: u16,
        latest_frame: Arc<FrameSlot>,
        stopped: Arc<AtomicBool>,
        runtime: Arc<dyn Runtime>,
    ) -> Result<(), String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("bind {port}: {e}"))?;
        // Non-blocking so the accept loop below can also check `stopped`
        // periodically - a plain blocking `accept()` has no way to be
        // cancelled once nothing is connecting.
        listener.set_nonblocking(true).map_err(|e| format!("set nonblocking: {e}"))?;
        log::info!("broadcastStream: listening on 0.0.0.0:{port}");

        let viewers: Arc<Viewers> = Arc::new(Mutex::new(Vec::new()));
        let ssrc = std::process::id().wrapping_mul(2_654_435_761);

        // Bind the socket ffmpeg will send its encoded RTP to - fixed for
        // the life of the broadcast even though ffmpeg itself gets
        // started/stopped repeatedly below (viewers coming and going,
        // window resizes).
        let rtp_port = UdpSocket::bind("127.0.0.1:0")
            .and_then(|s| s.local_addr())
            .map(|a| a.port())
            .map_err(|e| e.to_string())?;
        let std_sock = UdpSocket::bind(("127.0.0.1", rtp_port)).map_err(|e| e.to_string())?;
        let sock: Arc<dyn AsyncUdpSocket> = runtime.wrap_udp_socket(std_sock).map_err(|e| e.to_string())?;

        // Fans ffmpeg's encoded RTP out to every currently-connected
        // viewer, pruning any whose write failed (disconnected) - runs on
        // webrtc-rs's own shared reactor pool for as long as the process
        // lives, same as stream.rs's on_track forward task; it naturally
        // goes idle (never a busy loop) once nothing is sending it packets
        // (no viewers -> no ffmpeg -> nothing arrives here).
        let viewers_for_fanout = viewers.clone();
        runtime.spawn(Box::pin(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                let Ok((n, _)) = sock.recv_from(&mut buf).await else { break };
                let mut bytes = BytesMut::from(&buf[..n]);
                let Ok(mut packet) = rtc::rtp::packet::Packet::unmarshal(&mut bytes) else { continue };
                packet.header.ssrc = ssrc;

                let current: Vec<Arc<TrackLocalStaticRTP>> =
                    viewers_for_fanout.lock().unwrap().iter().map(|v| v.track.clone()).collect();
                let mut results = Vec::with_capacity(current.len());
                for track in &current {
                    results.push(track.write_rtp(packet.clone()).await.map_err(|_| ()));
                }
                let dead = indices_to_prune(&results);
                if !dead.is_empty() {
                    let mut guard = viewers_for_fanout.lock().unwrap();
                    for i in dead {
                        if i < guard.len() {
                            guard.remove(i);
                        }
                    }
                }
            }
        }));

        // Spawns/kills `ffmpeg` as the viewer count transitions to/from
        // zero, and feeds it captured frames only while at least one
        // viewer is actually watching - no point paying VP8's real CPU
        // cost (see MAX_BROADCAST_WIDTH) encoding for an empty room.
        let viewers_for_feed = viewers.clone();
        let latest_frame_for_feed = latest_frame.clone();
        let stopped_for_feed = stopped.clone();
        let runtime_for_feed = runtime.clone();
        runtime.spawn(Box::pin(async move {
            let mut current_dims: Option<(u32, u32)> = None;
            let mut ffmpeg: Option<(ffmpeg_sidecar::child::FfmpegChild, std::process::ChildStdin)> = None;

            while !stopped_for_feed.load(Ordering::Relaxed) {
                runtime_for_feed.sleep(Duration::from_millis(20)).await;

                if viewers_for_feed.lock().unwrap().is_empty() {
                    if let Some((mut child, _)) = ffmpeg.take() {
                        let _ = child.kill();
                    }
                    current_dims = None;
                    latest_frame_for_feed.lock().unwrap().take();
                    continue;
                }

                let Some((w, h, pixels)) = latest_frame_for_feed.lock().unwrap().take() else { continue };

                if current_dims != Some((w, h)) {
                    if let Some((mut child, _)) = ffmpeg.take() {
                        let _ = child.kill();
                    }
                    match spawn_encoder(w, h, rtp_port) {
                        Ok(pair) => {
                            current_dims = Some((w, h));
                            ffmpeg = Some(pair);
                        }
                        Err(e) => {
                            log::warn!("broadcastStream: failed to start ffmpeg encoder: {e}");
                            current_dims = None;
                            continue;
                        }
                    }
                }

                if let Some((_, stdin)) = &mut ffmpeg
                    && stdin.write_all(&pixels).is_err()
                {
                    // ffmpeg died - drop it, a fresh one starts on the next frame.
                    ffmpeg = None;
                    current_dims = None;
                }
            }

            if let Some((mut child, _)) = ffmpeg.take() {
                let _ = child.kill();
            }
        }));

        // Accept loop, on this dedicated thread: a real connection is
        // handed off to its own spawned handshake task immediately, so one
        // slow/stuck negotiation can never block admitting the next viewer.
        while !stopped.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((tcp, peer_addr)) => {
                    log::info!("broadcastStream: viewer connecting from {peer_addr}");
                    // On at least macOS/BSD, a socket accepted from a
                    // non-blocking listener inherits O_NONBLOCK - found via
                    // real testing (the handshake's blocking read/write
                    // calls immediately failed with EAGAIN). The listener
                    // itself must stay non-blocking (so this loop can check
                    // `stopped`), but each individual viewer connection
                    // should behave like a normal blocking socket, matching
                    // handshake_viewer's blocking-I/O assumptions.
                    if let Err(e) = tcp.set_nonblocking(false) {
                        log::warn!("broadcastStream: failed to set viewer socket blocking: {e}");
                        continue;
                    }
                    let viewers2 = viewers.clone();
                    let runtime2 = runtime.clone();
                    runtime.spawn(Box::pin(async move {
                        if let Err(e) = handshake_viewer(tcp, runtime2, viewers2, ssrc).await {
                            log::warn!("broadcastStream: viewer handshake failed: {e}");
                        }
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    runtime.sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    log::warn!("broadcastStream: accept error: {e}");
                    break;
                }
            }
        }

        let closers: Vec<Arc<dyn PeerConnection>> = {
            let mut guard = viewers.lock().unwrap();
            guard.drain(..).map(|v| v.peer_connection).collect()
        };
        for pc in closers {
            let _ = pc.close().await;
        }
        Ok(())
    }

    /// Negotiates one viewer's own `RTCPeerConnection` over its own TCP
    /// connection (a single SDP offer/answer exchange, same non-trickle
    /// ICE flow the original single-viewer code used) and, once connected,
    /// registers it in the shared `viewers` list. Runs as its own spawned
    /// task per accepted connection - a failure here only affects this one
    /// viewer, never the accept loop or any other viewer.
    async fn handshake_viewer(
        tcp: TcpStream,
        runtime: Arc<dyn Runtime>,
        viewers: Arc<Viewers>,
        ssrc: u32,
    ) -> Result<(), String> {
        let mut reader = BufReader::new(tcp.try_clone().map_err(|e| e.to_string())?);
        let mut writer = tcp;

        let (media_engine, registry) = vp8_media_engine()?;
        let config = ice_config().build();
        let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
        let (connected_tx, mut connected_rx) = channel::<()>(1);
        let handler = Arc::new(Handler { gather_complete_tx, connected_tx });

        let video_codec = RTCRtpCodec {
            mime_type: rtc::peer_connection::configuration::media_engine::MIME_TYPE_VP8.to_owned(),
            clock_rate: VP8_CLOCK_RATE,
            channels: 0,
            sdp_fmtp_line: "".to_owned(),
            rtcp_feedback: vec![],
        };
        let track: Arc<TrackLocalStaticRTP> = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "hydra-rust-broadcast-stream".to_string(),
            "hydra-rust-broadcast-video".to_string(),
            "hydra-rust-broadcast".to_string(),
            RtpCodecKind::Video,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
                codec: video_codec,
                ..Default::default()
            }],
        )));

        let peer_connection: Arc<dyn PeerConnection> = Arc::new(
            PeerConnectionBuilder::new()
                .with_configuration(config)
                .with_media_engine(media_engine)
                .with_interceptor_registry(registry)
                .with_handler(Arc::clone(&handler) as Arc<dyn PeerConnectionEventHandler>)
                .with_runtime(runtime.clone())
                .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
                .build()
                .await
                .map_err(|e| format!("build peer connection: {e}"))?,
        );

        peer_connection
            .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await
            .map_err(|e| format!("add track: {e}"))?;

        let offer = peer_connection.create_offer(None).await.map_err(|e| e.to_string())?;
        peer_connection.set_local_description(offer).await.map_err(|e| e.to_string())?;
        let _ = gather_complete_rx.recv().await;
        let local_desc = peer_connection
            .local_description()
            .await
            .ok_or("no local description after ICE gathering")?;
        writeln!(writer, "{}", serde_json::to_string(&local_desc).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;

        let mut answer_line = String::new();
        reader.read_line(&mut answer_line).map_err(|e| e.to_string())?;
        let answer: RTCSessionDescription =
            serde_json::from_str(answer_line.trim()).map_err(|e| e.to_string())?;
        peer_connection.set_remote_description(answer).await.map_err(|e| e.to_string())?;

        let _ = connected_rx.recv().await;
        log::info!("broadcastStream: viewer negotiated and connected");

        viewers.lock().unwrap().push(Viewer { peer_connection, track });
        Ok(())
    }

    const ENCODE_FPS: u32 = 25;

    /// A captured frame wider than this is downscaled (in `ffmpeg`, via
    /// `-vf scale=...`, *before* the VP8 encode step) rather than encoded
    /// at its native size. Found necessary via real end-to-end testing, not
    /// just theory: a Retina display's actual framebuffer resolution
    /// (e.g. 3456x1920 physical pixels behind a smaller logical window
    /// size) is enough raw pixels that realtime `libvpx` encoding fell
    /// far behind - hundreds of percent CPU with no RTP output ever
    /// actually reaching the peer. `read_pixels` itself still reads the
    /// full native resolution (downscaling *before* readback would need
    /// an extra GL render pass); this only caps the far more expensive
    /// encode step.
    const MAX_BROADCAST_WIDTH: u32 = 1280;

    fn spawn_encoder(
        width: u32,
        height: u32,
        rtp_port: u16,
    ) -> Result<(ffmpeg_sidecar::child::FfmpegChild, std::process::ChildStdin), String> {
        let mut command = FfmpegCommand::new();
        command
            .args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-s",
                &format!("{width}x{height}"),
                "-r",
                &ENCODE_FPS.to_string(),
            ])
            .input("pipe:0");
        if width > MAX_BROADCAST_WIDTH {
            // `-2` keeps the height even (required by most codecs,
            // including VP8) while preserving aspect ratio.
            command.args(["-vf", &format!("scale={MAX_BROADCAST_WIDTH}:-2")]);
        }
        command
            .codec_video("libvpx")
            .args(["-deadline", "realtime", "-cpu-used", "4", "-b:v", "1M", "-g", &ENCODE_FPS.to_string()])
            .format("rtp")
            .args(["-payload_type", &VP8_PAYLOAD_TYPE.to_string()])
            .output(format!("udp://127.0.0.1:{rtp_port}"));

        let mut child = command.spawn().map_err(|e| format!("spawn ffmpeg: {e}"))?;
        let stdin = child.take_stdin().ok_or("ffmpeg stdin was not piped")?;
        Ok((child, stdin))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn indices_to_prune_finds_only_failed_writes_in_descending_order() {
            let results: Vec<Result<(), ()>> = vec![Ok(()), Err(()), Ok(()), Err(()), Err(())];
            assert_eq!(indices_to_prune(&results), vec![4, 3, 1]);
        }

        #[test]
        fn indices_to_prune_is_empty_when_everything_succeeded() {
            let results: Vec<Result<(), ()>> = vec![Ok(()), Ok(()), Ok(())];
            assert!(indices_to_prune(&results).is_empty());
        }

        #[test]
        fn indices_to_prune_removal_order_never_invalidates_earlier_indices() {
            let results: Vec<Result<(), ()>> = vec![Err(()), Ok(()), Err(()), Ok(()), Err(())];
            let mut v = vec!["a", "b", "c", "d", "e"];
            for i in indices_to_prune(&results) {
                v.remove(i);
            }
            assert_eq!(v, vec!["b", "d"]);
        }

        #[test]
        fn start_on_a_bad_port_does_not_panic() {
            let mut mgr = BroadcastManager::new();
            // Port 1 is reserved/unlikely to be bindable without privilege -
            // the background thread's bind should just fail and log.
            mgr.start(1);
            assert!(mgr.is_active());
            std::thread::sleep(Duration::from_millis(100));
        }

        #[test]
        fn push_frame_before_any_broadcast_is_a_harmless_no_op() {
            let mut mgr = BroadcastManager::new();
            mgr.push_frame(4, 4, &[0u8; 4 * 4 * 3]);
            assert!(mgr.frame_sink().is_none());
        }

        #[test]
        fn stop_before_start_does_not_panic() {
            let mut mgr = BroadcastManager::new();
            mgr.stop();
            assert!(!mgr.is_active());
        }

        #[test]
        fn starting_twice_is_idempotent() {
            let mut mgr = BroadcastManager::new();
            mgr.start(2);
            let sink1 = mgr.frame_sink();
            mgr.start(3); // should be ignored - already active
            let sink2 = mgr.frame_sink();
            assert!(Arc::ptr_eq(&sink1.unwrap(), &sink2.unwrap()));
        }

        #[test]
        fn push_captured_frame_flips_rows_top_to_bottom() {
            let sink: FrameSlot = Mutex::new(None);
            // A 1x2 image (2 rows, 1 pixel each): bottom-up input is
            // [red, green] (GL row 0 = image bottom); after flipping it
            // should read top-down as [green, red].
            let bottom_up = [255u8, 0, 0, /* row 1 (top) */ 0, 255, 0];
            push_captured_frame(&sink, 1, 2, &bottom_up);
            let (w, h, flipped) = sink.lock().unwrap().take().unwrap();
            assert_eq!((w, h), (1, 2));
            assert_eq!(&flipped[0..3], &[0, 255, 0]);
            assert_eq!(&flipped[3..6], &[255, 0, 0]);
        }
    }
}

#[cfg(feature = "stream")]
pub use imp::*;
