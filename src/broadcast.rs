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
    use std::future::{Future, poll_fn};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
    use std::pin::pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::task::Poll;
    use std::thread;
    use std::time::{Duration, Instant};

    use bytes::BytesMut;
    use ffmpeg_sidecar::command::FfmpegCommand;
    use rtc::media_stream::MediaStreamTrack;
    use rtc::peer_connection::sdp::RTCSessionDescription;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::shared::marshal::Unmarshal;
    use webrtc::media_stream::track_local::TrackLocal;
    use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
    use webrtc::peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    };
    use webrtc::runtime::{JoinHandle, Receiver, Runtime, Sender, channel, timeout};

    use crate::stream::{VP8_CLOCK_RATE, VP8_PAYLOAD_TYPE, ice_config, vp8_media_engine};

    type FrameSlot = Mutex<Option<(u32, u32, Vec<u8>)>>;

    /// Flips GL's bottom-up row order to the top-down order `ffmpeg`/RTP
    /// expect, and stashes the result in `sink` (latest-frame-wins, like
    /// every other manager here) - a free function, not a `BroadcastManager`
    /// method, so the GL render callback (which only holds a cloned `Arc`,
    /// not the whole manager - see `frame_sink`) can call it directly.
    pub fn push_captured_frame(sink: &FrameSlot, width: u32, height: u32, rgb_bottom_up: &[u8]) {
        push_owned_captured_frame(sink, width, height, rgb_bottom_up.to_vec());
    }

    pub fn push_owned_captured_frame(
        sink: &FrameSlot,
        width: u32,
        height: u32,
        mut pixels: Vec<u8>,
    ) {
        let row_bytes = (width as usize) * 3;
        if pixels.len() != row_bytes * height as usize {
            log::warn!("broadcastStream: captured frame dimensions do not match its pixels");
            return;
        }
        for y in 0..height as usize / 2 {
            let (top, bottom) = pixels.split_at_mut((height as usize - 1 - y) * row_bytes);
            top[y * row_bytes..(y + 1) * row_bytes].swap_with_slice(&mut bottom[..row_bytes]);
        }
        *sink.lock().unwrap() = Some((width, height, pixels));
    }

    struct BroadcastState {
        stopped: Arc<AtomicBool>,
        latest_frame: Arc<FrameSlot>,
        capture_demand: Arc<AtomicBool>,
    }

    impl Drop for BroadcastState {
        fn drop(&mut self) {
            // Tells the broadcast thread's frame-feed loop to stop, kill
            // ffmpeg, and close the peer connection - see run_broadcast.
            self.stopped.store(true, Ordering::Relaxed);
            self.capture_demand.store(false, Ordering::Release);
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

        pub fn needs_capture(&self) -> bool {
            self.state.as_ref().is_some_and(|state| {
                !state.stopped.load(Ordering::Relaxed)
                    && state.capture_demand.load(Ordering::Acquire)
            })
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
            let capture_demand = Arc::new(AtomicBool::new(false));

            let frame_slot = latest_frame.clone();
            let stop_flag = stopped.clone();
            let demand_flag = capture_demand.clone();
            thread::Builder::new()
                .name("hydra-broadcast".into())
                .spawn(move || run_broadcast(port, frame_slot, stop_flag, demand_flag))
                .expect("spawn broadcast thread");

            self.state = Some(BroadcastState {
                stopped,
                latest_frame,
                capture_demand,
            });
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
        cancelled: Arc<AtomicBool>,
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
            } else if matches!(
                state,
                RTCPeerConnectionState::Failed
                    | RTCPeerConnectionState::Closed
                    | RTCPeerConnectionState::Disconnected
            ) {
                self.cancelled.store(true, Ordering::Release);
            }
        }
    }

    const VIEWER_QUEUE_CAPACITY: usize = 256;
    const MAX_HANDSHAKES: usize = 4;
    static ACTIVE_HANDSHAKES: AtomicUsize = AtomicUsize::new(0);
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
    const ENCODER_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_SDP_BYTES: u64 = 1024 * 1024;

    type Packet = rtc::rtp::packet::Packet;

    /// Full queues disconnect viewers instead of corrupting encoded frames.
    struct Delivery<T> {
        packets: Sender<T>,
        cancelled: Arc<AtomicBool>,
    }

    impl<T> Delivery<T> {
        fn enqueue(&self, packet: T) {
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            if let Err(error) = self.packets.try_send(packet) {
                self.cancelled.store(true, Ordering::Release);
                match error {
                    webrtc::runtime::TrySendError::Full(_) => {
                        log::warn!("broadcastStream: disconnecting slow viewer: RTP queue is full");
                    }
                    webrtc::runtime::TrySendError::Disconnected(_) => {
                        log::debug!("broadcastStream: viewer delivery queue closed");
                    }
                }
            }
        }
    }

    struct Viewer {
        delivery: Delivery<Packet>,
        task: Box<dyn JoinHandle>,
    }
    type Viewers = Mutex<Vec<Viewer>>;

    async fn race_cancellation<T>(
        operation: impl Future<Output = T>,
        cancellation: impl Future<Output = &'static str>,
    ) -> Result<T, &'static str> {
        let mut operation = pin!(operation);
        let mut cancellation = pin!(cancellation);
        poll_fn(|cx| {
            if let Poll::Ready(reason) = cancellation.as_mut().poll(cx) {
                return Poll::Ready(Err(reason));
            }
            operation.as_mut().poll(cx).map(Ok)
        })
        .await
    }

    async fn cancellable<T>(
        runtime: &dyn Runtime,
        stopped: &AtomicBool,
        cancelled: &AtomicBool,
        deadline: Option<Instant>,
        operation: impl Future<Output = T>,
    ) -> Result<T, &'static str> {
        race_cancellation(operation, async {
            loop {
                if stopped.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) {
                    return "cancelled";
                }
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return "timed out";
                }
                runtime.sleep(POLL_INTERVAL).await;
            }
        })
        .await
    }

    async fn close_peer(runtime: &dyn Runtime, peer: &dyn PeerConnection) {
        match timeout(runtime, CLOSE_TIMEOUT, peer.close()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("broadcastStream: closing viewer failed: {error}"),
            Err(error) => log::warn!("broadcastStream: closing viewer timed out: {error}"),
        }
    }

    struct EncoderProcess {
        child: std::process::Child,
        killed: bool,
        writing_since: Option<Instant>,
    }

    // Supervision needs the process handle while its worker blocks on stdin.
    type EncoderSlot = Arc<Mutex<Option<EncoderProcess>>>;

    fn kill_encoder(process: &EncoderSlot) {
        if let Some(process) = process.lock().unwrap().as_mut()
            && !process.killed
        {
            if let Err(error) = process.child.kill() {
                log::warn!("broadcastStream: killing encoder failed: {error}");
            }
            process.killed = true;
        }
    }

    fn reap_encoder(process: &EncoderSlot) {
        let child = process.lock().unwrap().take();
        if let Some(mut process) = child {
            if !process.killed
                && let Err(error) = process.child.kill()
            {
                log::warn!("broadcastStream: killing encoder failed: {error}");
            }
            if let Err(error) = process.child.wait() {
                log::warn!("broadcastStream: reaping encoder failed: {error}");
            }
        }
    }

    struct HandshakeWorker {
        _permit: HandshakePermit<'static>,
        socket: TcpStream,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
        thread: thread::JoinHandle<()>,
        task: Box<dyn JoinHandle>,
    }

    struct HandshakePermit<'a>(&'a AtomicUsize);

    impl Drop for HandshakePermit<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn reserve_handshake(active: &AtomicUsize) -> Option<HandshakePermit<'_>> {
        let mut count = active.load(Ordering::Acquire);
        while count < MAX_HANDSHAKES {
            match active.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(HandshakePermit(active)),
                Err(current) => count = current,
            }
        }
        None
    }

    impl HandshakeWorker {
        fn cancel(&self) {
            self.cancelled.store(true, Ordering::Release);
            if let Err(error) = self.socket.shutdown(Shutdown::Both)
                && error.kind() != std::io::ErrorKind::NotConnected
            {
                log::warn!("broadcastStream: cancelling handshake socket failed: {error}");
            }
        }
    }

    #[derive(Default)]
    struct Workers {
        encoder: Option<thread::JoinHandle<()>>,
        process: EncoderSlot,
        handshakes: Vec<HandshakeWorker>,
        viewers: Arc<Viewers>,
        fanout: Option<Box<dyn JoinHandle>>,
    }

    impl Workers {
        fn reap_handshakes(&mut self) {
            let mut index = 0;
            while index < self.handshakes.len() {
                let worker = &self.handshakes[index];
                if Instant::now() >= worker.deadline {
                    worker.cancel();
                }
                if worker.thread.is_finished() && worker.task.is_finished() {
                    let worker = self.handshakes.swap_remove(index);
                    if worker.thread.join().is_err() {
                        log::error!("broadcastStream: handshake worker panicked");
                    }
                } else {
                    index += 1;
                }
            }
        }

        async fn stop(&self, runtime: &dyn Runtime) {
            kill_encoder(&self.process);
            for worker in &self.handshakes {
                worker.cancel();
            }
            for viewer in self.viewers.lock().unwrap().iter() {
                viewer.delivery.cancelled.store(true, Ordering::Release);
            }
            let deadline = Instant::now() + CLOSE_TIMEOUT + Duration::from_secs(1);
            loop {
                let finished = self
                    .handshakes
                    .iter()
                    .all(|worker| worker.task.is_finished())
                    && self
                        .viewers
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|v| v.task.is_finished())
                    && self.fanout.as_ref().is_none_or(|task| task.is_finished());
                if finished {
                    break;
                }
                if Instant::now() >= deadline {
                    log::warn!("broadcastStream: aborting unfinished session tasks");
                    for worker in &self.handshakes {
                        worker.task.abort();
                    }
                    for viewer in self.viewers.lock().unwrap().iter() {
                        viewer.task.abort();
                    }
                    if let Some(task) = &self.fanout {
                        task.abort();
                    }
                    break;
                }
                runtime.sleep(POLL_INTERVAL).await;
            }
        }

        fn join(self) {
            if let Some(encoder) = self.encoder
                && encoder.join().is_err()
            {
                log::error!("broadcastStream: encoder worker panicked");
            }
            reap_encoder(&self.process);
            for worker in self.handshakes {
                if worker.thread.join().is_err() {
                    log::error!("broadcastStream: handshake worker panicked");
                }
            }
        }
    }

    /// Runs on its own dedicated OS thread for as long as the broadcast is
    /// active - mirrors `stream.rs::run_receiver`'s shape.
    fn run_broadcast(
        port: u16,
        latest_frame: Arc<FrameSlot>,
        stopped: Arc<AtomicBool>,
        capture_demand: Arc<AtomicBool>,
    ) {
        let Some(runtime) = webrtc::runtime::default_runtime() else {
            log::warn!("broadcastStream: no async runtime available");
            return;
        };
        let runtime_for_session = runtime.clone();
        let demand_for_session = capture_demand.clone();
        let mut workers = Workers::default();
        runtime.block_on(Box::pin(async {
            if let Err(e) = broadcast_session(
                port,
                latest_frame,
                stopped.clone(),
                demand_for_session,
                runtime_for_session.clone(),
                &mut workers,
            )
            .await
            {
                log::warn!("broadcastStream: session ended: {e}");
            }
            stopped.store(true, Ordering::Release);
            capture_demand.store(false, Ordering::Release);
            workers.stop(runtime_for_session.as_ref()).await;
        }));
        workers.join();
    }

    async fn broadcast_session(
        port: u16,
        latest_frame: Arc<FrameSlot>,
        stopped: Arc<AtomicBool>,
        capture_demand: Arc<AtomicBool>,
        runtime: Arc<dyn Runtime>,
        workers: &mut Workers,
    ) -> Result<(), String> {
        let listener =
            TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("bind {port}: {e}"))?;
        // Non-blocking so the accept loop below can also check `stopped`
        // periodically - a plain blocking `accept()` has no way to be
        // cancelled once nothing is connecting.
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set nonblocking: {e}"))?;
        log::info!("broadcastStream: listening on 0.0.0.0:{port}");

        let viewers = workers.viewers.clone();
        let ssrc = std::process::id().wrapping_mul(2_654_435_761);

        // Bind the socket ffmpeg will send its encoded RTP to - fixed for
        // the life of the broadcast even though ffmpeg itself gets
        // started/stopped repeatedly below (viewers coming and going,
        // window resizes).
        let std_sock = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let rtp_port = std_sock.local_addr().map_err(|e| e.to_string())?.port();
        let sock = runtime
            .wrap_udp_socket(std_sock)
            .map_err(|e| e.to_string())?;

        // Each viewer drains its own queue, so one stalled transport cannot block fanout.
        let viewers_for_fanout = viewers.clone();
        let stopped_for_fanout = stopped.clone();
        let runtime_for_fanout = runtime.clone();
        workers.fanout = Some(runtime.spawn(Box::pin(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                let result = cancellable(
                    runtime_for_fanout.as_ref(),
                    &stopped_for_fanout,
                    &stopped_for_fanout,
                    None,
                    sock.recv_from(&mut buf),
                )
                .await;
                let n = match result {
                    Ok(Ok((n, _))) => n,
                    Ok(Err(error)) => {
                        log::warn!("broadcastStream: RTP receive failed: {error}");
                        stopped_for_fanout.store(true, Ordering::Release);
                        break;
                    }
                    Err(_) => break,
                };
                let mut bytes = BytesMut::from(&buf[..n]);
                let Ok(mut packet) = rtc::rtp::packet::Packet::unmarshal(&mut bytes) else {
                    continue;
                };
                packet.header.ssrc = ssrc;

                for viewer in viewers_for_fanout.lock().unwrap().iter() {
                    viewer.delivery.enqueue(packet.clone());
                }
            }
        })));

        // Spawns/kills `ffmpeg` as the viewer count transitions to/from
        // zero, and feeds it captured frames only while at least one
        // viewer is actually watching - no point paying VP8's real CPU
        // cost (see MAX_BROADCAST_WIDTH) encoding for an empty room.
        let latest_frame_for_feed = latest_frame.clone();
        let stopped_for_feed = stopped.clone();
        let demand_for_feed = capture_demand.clone();
        let process = workers.process.clone();
        workers.encoder = Some(
            thread::Builder::new()
                .name("hydra-broadcast-encoder".into())
                .spawn(move || {
                    let mut current_dims: Option<(u32, u32)> = None;
                    let mut stdin: Option<std::process::ChildStdin> = None;

                    while !stopped_for_feed.load(Ordering::Relaxed) {
                        thread::sleep(POLL_INTERVAL);

                        let killed = process.lock().unwrap().as_ref().is_some_and(|p| p.killed);
                        if !demand_for_feed.load(Ordering::Acquire) || killed {
                            stdin = None;
                            reap_encoder(&process);
                            current_dims = None;
                            latest_frame_for_feed.lock().unwrap().take();
                            continue;
                        }

                        let Some((w, h, pixels)) = latest_frame_for_feed.lock().unwrap().take()
                        else {
                            continue;
                        };

                        if current_dims != Some((w, h)) {
                            stdin = None;
                            reap_encoder(&process);
                            match spawn_encoder(w, h, rtp_port) {
                                Ok((child, input)) => {
                                    {
                                        let mut guard = process.lock().unwrap();
                                        *guard = Some(EncoderProcess {
                                            child,
                                            killed: false,
                                            writing_since: None,
                                        });
                                    }
                                    if stopped_for_feed.load(Ordering::Acquire)
                                        || !demand_for_feed.load(Ordering::Acquire)
                                    {
                                        reap_encoder(&process);
                                        continue;
                                    }
                                    current_dims = Some((w, h));
                                    stdin = Some(input);
                                }
                                Err(e) => {
                                    log::warn!(
                                        "broadcastStream: failed to start ffmpeg encoder: {e}"
                                    );
                                    current_dims = None;
                                    continue;
                                }
                            }
                        }

                        if let Some(input) = &mut stdin {
                            if let Some(process) = process.lock().unwrap().as_mut() {
                                process.writing_since = Some(Instant::now());
                            }
                            let result = input.write_all(&pixels);
                            if let Some(process) = process.lock().unwrap().as_mut() {
                                process.writing_since = None;
                            }
                            if let Err(error) = result {
                                if !stopped_for_feed.load(Ordering::Acquire)
                                    && demand_for_feed.load(Ordering::Acquire)
                                {
                                    log::warn!(
                                        "broadcastStream: writing encoder frame failed: {error}"
                                    );
                                }
                                stdin = None;
                                reap_encoder(&process);
                                current_dims = None;
                            }
                        }
                    }

                    drop(stdin);
                    reap_encoder(&process);
                })
                .map_err(|error| format!("spawn encoder worker: {error}"))?,
        );

        // Keep supervision responsive even when encoder or handshake I/O stalls.
        while !stopped.load(Ordering::Relaxed) {
            workers.reap_handshakes();
            let watching = {
                let mut viewers = viewers.lock().unwrap();
                viewers.retain(|viewer| !viewer.task.is_finished());
                viewers
                    .iter()
                    .any(|viewer| !viewer.delivery.cancelled.load(Ordering::Acquire))
            };
            capture_demand.store(watching, Ordering::Release);
            let stalled = workers
                .process
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|process| {
                    !process.killed
                        && process
                            .writing_since
                            .is_some_and(|start| start.elapsed() >= ENCODER_WRITE_TIMEOUT)
                });
            if stalled {
                log::warn!("broadcastStream: encoder pipe stalled; restarting encoder");
            }
            if !watching || stalled {
                kill_encoder(&workers.process);
            }
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
                    match start_handshake(
                        tcp,
                        runtime.clone(),
                        viewers.clone(),
                        stopped.clone(),
                        ssrc,
                    ) {
                        Ok(worker) => workers.handshakes.push(worker),
                        Err(error) => {
                            log::warn!("broadcastStream: starting handshake failed: {error}")
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    runtime.sleep(POLL_INTERVAL).await;
                }
                Err(e) => {
                    log::warn!("broadcastStream: accept error: {e}");
                    break;
                }
            }
        }

        Ok(())
    }

    fn start_handshake(
        tcp: TcpStream,
        runtime: Arc<dyn Runtime>,
        viewers: Arc<Viewers>,
        stopped: Arc<AtomicBool>,
        ssrc: u32,
    ) -> Result<HandshakeWorker, String> {
        let permit = reserve_handshake(&ACTIVE_HANDSHAKES).ok_or("handshake limit reached")?;
        tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(|e| e.to_string())?;
        tcp.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(|e| e.to_string())?;
        let socket = tcp.try_clone().map_err(|e| e.to_string())?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let (offer_tx, offer_rx) = mpsc::sync_channel::<String>(1);
        let (answer_tx, answer_rx) = channel::<Result<RTCSessionDescription, String>>(1);
        let stopped_for_io = stopped.clone();
        let cancelled_for_io = cancelled.clone();
        let thread = thread::Builder::new()
            .name("hydra-broadcast-handshake".into())
            .spawn(move || {
                let result = (|| {
                    let offer = loop {
                        if stopped_for_io.load(Ordering::Acquire)
                            || cancelled_for_io.load(Ordering::Acquire)
                            || Instant::now() >= deadline
                        {
                            return Err("handshake I/O cancelled or timed out".to_string());
                        }
                        match offer_rx.recv_timeout(POLL_INTERVAL) {
                            Ok(offer) => break offer,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err("offer channel closed".to_string());
                            }
                        }
                    };
                    writeln!(&tcp, "{offer}").map_err(|e| format!("write offer: {e}"))?;
                    let mut answer = String::new();
                    BufReader::new((&tcp).take(MAX_SDP_BYTES + 1))
                        .read_line(&mut answer)
                        .map_err(|e| format!("read answer: {e}"))?;
                    if answer.len() as u64 > MAX_SDP_BYTES || !answer.ends_with('\n') {
                        return Err("answer is oversized or incomplete".to_string());
                    }
                    serde_json::from_str(answer.trim()).map_err(|e| format!("decode answer: {e}"))
                })();
                if let Err(error) = &result {
                    log::warn!("broadcastStream: handshake I/O failed: {error}");
                }
                let _ = answer_tx.try_send(result);
                if let Err(error) = tcp.shutdown(Shutdown::Both)
                    && error.kind() != std::io::ErrorKind::NotConnected
                {
                    log::warn!("broadcastStream: closing handshake socket failed: {error}");
                }
            })
            .map_err(|e| format!("spawn handshake worker: {e}"))?;

        let cancelled_for_task = cancelled.clone();
        let runtime_for_task = runtime.clone();
        let task = runtime.spawn(Box::pin(async move {
            if let Err(error) = handshake_viewer(
                offer_tx,
                answer_rx,
                runtime_for_task,
                viewers,
                stopped,
                cancelled_for_task.clone(),
                deadline,
                ssrc,
            )
            .await
            {
                cancelled_for_task.store(true, Ordering::Release);
                log::warn!("broadcastStream: viewer handshake failed: {error}");
            }
        }));
        Ok(HandshakeWorker {
            _permit: permit,
            socket,
            cancelled,
            deadline,
            thread,
            task,
        })
    }

    /// Only the dedicated handshake worker touches the blocking TCP socket.
    #[allow(clippy::too_many_arguments)]
    async fn handshake_viewer(
        offer_tx: mpsc::SyncSender<String>,
        mut answer_rx: Receiver<Result<RTCSessionDescription, String>>,
        runtime: Arc<dyn Runtime>,
        viewers: Arc<Viewers>,
        stopped: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
        ssrc: u32,
    ) -> Result<(), String> {
        let (media_engine, registry) = vp8_media_engine()?;
        let config = ice_config().build();
        let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
        let (connected_tx, mut connected_rx) = channel::<()>(1);
        let handler = Arc::new(Handler {
            gather_complete_tx,
            connected_tx,
            cancelled: cancelled.clone(),
        });

        let video_codec = RTCRtpCodec {
            mime_type: rtc::peer_connection::configuration::media_engine::MIME_TYPE_VP8.to_owned(),
            clock_rate: VP8_CLOCK_RATE,
            channels: 0,
            sdp_fmtp_line: "".to_owned(),
            rtcp_feedback: vec![],
        };
        let track: Arc<TrackLocalStaticRTP> =
            Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
                "hydra-rust-broadcast-stream".to_string(),
                "hydra-rust-broadcast-video".to_string(),
                "hydra-rust-broadcast".to_string(),
                RtpCodecKind::Video,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec: video_codec,
                    ..Default::default()
                }],
            )));

        let peer_connection: Arc<dyn PeerConnection> = Arc::new(
            cancellable(
                runtime.as_ref(),
                &stopped,
                &cancelled,
                Some(deadline),
                PeerConnectionBuilder::new()
                    .with_configuration(config)
                    .with_media_engine(media_engine)
                    .with_interceptor_registry(registry)
                    .with_handler(Arc::clone(&handler) as Arc<dyn PeerConnectionEventHandler>)
                    .with_runtime(runtime.clone())
                    .with_dedicated_reactor_pool_size(1)
                    .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
                    .build(),
            )
            .await
            .map_err(str::to_string)?
            .map_err(|e| format!("build peer connection: {e}"))?,
        );

        let result = cancellable(
            runtime.as_ref(),
            &stopped,
            &cancelled,
            Some(deadline),
            async {
                peer_connection
                    .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
                    .await
                    .map_err(|e| format!("add track: {e}"))?;

                let offer = peer_connection
                    .create_offer(None)
                    .await
                    .map_err(|e| e.to_string())?;
                peer_connection
                    .set_local_description(offer)
                    .await
                    .map_err(|e| e.to_string())?;
                gather_complete_rx
                    .recv()
                    .await
                    .ok_or("ICE gathering channel closed")?;
                let local_desc = peer_connection
                    .local_description()
                    .await
                    .ok_or("no local description after ICE gathering")?;
                offer_tx
                    .try_send(serde_json::to_string(&local_desc).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("send offer to handshake worker: {e}"))?;

                let answer = answer_rx.recv().await.ok_or("answer channel closed")??;
                peer_connection
                    .set_remote_description(answer)
                    .await
                    .map_err(|e| e.to_string())?;

                connected_rx
                    .recv()
                    .await
                    .ok_or("connection channel closed")?;
                Ok::<(), String>(())
            },
        )
        .await
        .map_err(str::to_string)
        .and_then(|result| result);
        if let Err(error) = result {
            close_peer(runtime.as_ref(), peer_connection.as_ref()).await;
            return Err(error);
        }
        log::info!("broadcastStream: viewer negotiated and connected");

        let (packets, mut packet_rx) = channel::<Packet>(VIEWER_QUEUE_CAPACITY);
        let cancelled_for_delivery = cancelled.clone();
        let runtime_for_delivery = runtime.clone();
        let task = runtime.spawn(Box::pin(async move {
            let result = cancellable(
                runtime_for_delivery.as_ref(),
                &stopped,
                &cancelled_for_delivery,
                None,
                async {
                    while let Some(packet) = packet_rx.recv().await {
                        track.write_rtp(packet).await.map_err(|e| e.to_string())?;
                    }
                    Ok::<(), String>(())
                },
            )
            .await;
            if let Ok(Err(error)) = result {
                log::warn!("broadcastStream: viewer RTP write failed: {error}");
            }
            cancelled_for_delivery.store(true, Ordering::Release);
            close_peer(runtime_for_delivery.as_ref(), peer_connection.as_ref()).await;
        }));
        viewers.lock().unwrap().push(Viewer {
            delivery: Delivery { packets, cancelled },
            task,
        });
        Ok(())
    }

    const ENCODE_FPS: u32 = 25;

    /// Limits encoding work before the app transfers pixels from the GPU.
    const MAX_BROADCAST_WIDTH: u32 = 1280;

    pub fn capture_dimensions(width: u32, height: u32) -> Result<[u32; 2], String> {
        if width == 0 || height == 0 {
            return Err("Broadcast capture requires nonzero dimensions".into());
        }
        if width <= MAX_BROADCAST_WIDTH {
            return Ok([width, height]);
        }
        let height =
            ((height as u64 * MAX_BROADCAST_WIDTH as u64 + width as u64) / (2 * width as u64) * 2)
                .max(2) as u32;
        Ok([MAX_BROADCAST_WIDTH, height])
    }

    fn spawn_encoder(
        width: u32,
        height: u32,
        rtp_port: u16,
    ) -> Result<(std::process::Child, std::process::ChildStdin), String> {
        let mut command = FfmpegCommand::new();
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostats",
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
            .args([
                "-deadline",
                "realtime",
                "-cpu-used",
                "4",
                "-b:v",
                "1M",
                "-g",
                &ENCODE_FPS.to_string(),
            ])
            .format("rtp")
            .args(["-payload_type", &VP8_PAYLOAD_TYPE.to_string()])
            .output(format!("udp://127.0.0.1:{rtp_port}"));

        let mut child = command
            .as_inner_mut()
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn ffmpeg: {e}"))?;
        let Some(stdin) = child.stdin.take() else {
            if let Err(error) = child.kill() {
                log::warn!("broadcastStream: killing unpiped encoder failed: {error}");
            }
            if let Err(error) = child.wait() {
                log::warn!("broadcastStream: reaping unpiped encoder failed: {error}");
            }
            return Err("ffmpeg stdin was not piped".into());
        };
        Ok((child, stdin))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn capture_demand_requires_an_active_viewer_and_stops_with_the_session() {
            let stopped = Arc::new(AtomicBool::new(false));
            let demand = Arc::new(AtomicBool::new(false));
            let mut manager = BroadcastManager {
                state: Some(BroadcastState {
                    stopped: stopped.clone(),
                    latest_frame: Arc::new(Mutex::new(None)),
                    capture_demand: demand.clone(),
                }),
            };
            assert!(!manager.needs_capture());
            demand.store(true, Ordering::Release);
            assert!(manager.needs_capture());
            manager.stop();
            assert!(!manager.needs_capture());
            assert!(!demand.load(Ordering::Acquire));
            assert!(stopped.load(Ordering::Relaxed));
        }

        #[test]
        fn capture_dimensions_downscale_before_readback_and_preserve_small_frames() {
            assert_eq!(capture_dimensions(1920, 1080).unwrap(), [1280, 720]);
            assert_eq!(capture_dimensions(800, 601).unwrap(), [800, 601]);
            assert_eq!(capture_dimensions(2560, 1441).unwrap(), [1280, 720]);
            assert!(capture_dimensions(0, 100).is_err());
        }

        #[test]
        fn owned_capture_flips_in_place_and_preserves_the_middle_row() {
            let sink = Mutex::new(None);
            let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
            let pointer = pixels.as_ptr();
            push_owned_captured_frame(&sink, 1, 3, pixels);
            let (_, _, pixels) = sink.lock().unwrap().take().unwrap();
            assert_eq!(pixels, vec![7, 8, 9, 4, 5, 6, 1, 2, 3]);
            assert_eq!(pixels.as_ptr(), pointer);
        }

        #[test]
        fn full_viewer_queue_disconnects_without_dropping_queued_packets() {
            let (packets, mut received) = channel(VIEWER_QUEUE_CAPACITY);
            let delivery = Delivery {
                packets,
                cancelled: Arc::new(AtomicBool::new(false)),
            };
            for packet in 0..VIEWER_QUEUE_CAPACITY {
                delivery.enqueue(packet);
            }
            assert!(!delivery.cancelled.load(Ordering::Acquire));
            delivery.enqueue(VIEWER_QUEUE_CAPACITY);
            assert!(delivery.cancelled.load(Ordering::Acquire));
            delivery.enqueue(VIEWER_QUEUE_CAPACITY + 1);
            for packet in 0..VIEWER_QUEUE_CAPACITY {
                assert_eq!(received.try_recv().unwrap(), packet);
            }
            assert!(matches!(
                received.try_recv(),
                Err(webrtc::runtime::TryRecvError::Empty)
            ));
        }

        #[test]
        fn slow_viewer_does_not_block_another_viewers_delivery() {
            let (slow_tx, _slow_rx) = channel(VIEWER_QUEUE_CAPACITY);
            let (fast_tx, mut fast_rx) = channel(VIEWER_QUEUE_CAPACITY);
            let slow = Delivery {
                packets: slow_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
            };
            let fast = Delivery {
                packets: fast_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
            };
            for packet in 0..VIEWER_QUEUE_CAPACITY * 2 {
                slow.enqueue(packet);
                fast.enqueue(packet);
                assert_eq!(fast_rx.try_recv().unwrap(), packet);
            }
            assert!(slow.cancelled.load(Ordering::Acquire));
            assert!(!fast.cancelled.load(Ordering::Acquire));
        }

        #[test]
        fn closed_viewer_queue_cancels_the_viewer() {
            let (packets, received) = channel::<u8>(VIEWER_QUEUE_CAPACITY);
            let delivery = Delivery {
                packets,
                cancelled: Arc::new(AtomicBool::new(false)),
            };
            drop(received);
            delivery.enqueue(1);
            assert!(delivery.cancelled.load(Ordering::Acquire));
        }

        #[test]
        fn handshake_admission_rejects_a_fifth_active_worker() {
            let active = AtomicUsize::new(0);
            let mut workers = Vec::new();
            for _ in 0..4 {
                workers.push(reserve_handshake(&active).unwrap());
            }
            assert!(reserve_handshake(&active).is_none());
            drop(workers.pop());
            workers.push(reserve_handshake(&active).unwrap());
            assert!(reserve_handshake(&active).is_none());
            drop(workers);
            assert_eq!(active.load(Ordering::Acquire), 0);
        }

        #[test]
        fn cancellation_drops_pending_work_and_wins_over_ready_work() {
            struct PendingWork(Arc<AtomicBool>);
            impl Future for PendingWork {
                type Output = ();
                fn poll(
                    self: std::pin::Pin<&mut Self>,
                    _: &mut std::task::Context<'_>,
                ) -> Poll<()> {
                    Poll::Pending
                }
            }
            impl Drop for PendingWork {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let dropped = Arc::new(AtomicBool::new(false));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            {
                let future = race_cancellation(PendingWork(dropped.clone()), async { "stopped" });
                let mut future = pin!(future);
                assert_eq!(
                    future.as_mut().poll(&mut context),
                    Poll::Ready(Err("stopped"))
                );
            }
            assert!(dropped.load(Ordering::Acquire));
            let mut future = pin!(race_cancellation(async { 42 }, async { "timed out" }));
            assert_eq!(
                future.as_mut().poll(&mut context),
                Poll::Ready(Err("timed out"))
            );
        }

        #[test]
        fn cancellation_checks_session_viewer_and_handshake_deadline() {
            let runtime = webrtc::runtime::default_runtime().unwrap();
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            for (stopped, cancelled, deadline, reason) in [
                (true, false, None, "cancelled"),
                (false, true, None, "cancelled"),
                (false, false, Some(Instant::now()), "timed out"),
            ] {
                let stopped = AtomicBool::new(stopped);
                let cancelled = AtomicBool::new(cancelled);
                let mut future = pin!(cancellable(
                    runtime.as_ref(),
                    &stopped,
                    &cancelled,
                    deadline,
                    std::future::pending::<()>(),
                ));
                assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(Err(reason)));
            }
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
            let mut mgr = BroadcastManager {
                state: Some(BroadcastState {
                    stopped: Arc::new(AtomicBool::new(false)),
                    latest_frame: Arc::new(Mutex::new(None)),
                    capture_demand: Arc::new(AtomicBool::new(false)),
                }),
            };
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
