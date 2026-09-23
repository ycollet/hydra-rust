//! A companion CLI tool for testing/using `s0.initStream("host:port")` (see
//! `src/stream.rs`) - **not** a Rhai-scriptable function itself. Broadcasts
//! a video source (a synthetic test pattern by default, or a real webcam/
//! file via `ffmpeg`'s own `-f`/`-i` arguments) over WebRTC to exactly one
//! `initStream` receiver.
//!
//! This is deliberately a minimal testing/demo tool, not a polished
//! feature - a first-class scripted broadcast function (the equivalent of
//! real hydra.js's `pb.setName()`) is a natural follow-up if this proves
//! useful. In particular, it doesn't clean up its spawned `ffmpeg` child on
//! a hard kill (Ctrl+C) - acceptable for a throwaway demo, not for a real
//! feature.
//!
//! Usage:
//! ```text
//! cargo run --features stream --example webrtc_broadcast -- 9000
//! cargo run --features stream --example webrtc_broadcast -- 9000 --format avfoundation --input 0
//! cargo run --features stream --example webrtc_broadcast -- 9000 --input clip.mp4
//! ```
//! Then, in a hydra-rust script: `s0.initStream("<this-machine's-ip>:9000").out()`.

#[cfg(not(feature = "stream"))]
fn main() {
    eprintln!("this example requires --features stream");
    std::process::exit(1);
}

#[cfg(feature = "stream")]
fn main() {
    env_logger::init();

    let mut args = std::env::args().skip(1);
    let Some(port) = args.next().and_then(|s| s.parse::<u16>().ok()) else {
        eprintln!("usage: webrtc_broadcast <listen-port> [--format <ffmpeg -f>] [--input <ffmpeg -i>]");
        std::process::exit(1);
    };
    let mut input_format: Option<String> = None;
    let mut input = String::new();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--format" => input_format = args.next(),
            "--input" => input = args.next().unwrap_or_default(),
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }

    if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
        eprintln!("ffmpeg was not found on PATH - required to encode the broadcast video");
        std::process::exit(1);
    }

    let Some(runtime) = webrtc::runtime::default_runtime() else {
        eprintln!("no async runtime available");
        std::process::exit(1);
    };
    let runtime2 = runtime.clone();
    runtime.block_on(Box::pin(async move {
        if let Err(e) = broadcast(port, input_format, input, runtime2).await {
            eprintln!("webrtc_broadcast: {e}");
            std::process::exit(1);
        }
    }));
}

#[cfg(feature = "stream")]
async fn broadcast(
    port: u16,
    input_format: Option<String>,
    input: String,
    runtime: std::sync::Arc<dyn webrtc::runtime::Runtime>,
) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, UdpSocket};
    use std::sync::Arc;

    use bytes::BytesMut;
    use ffmpeg_sidecar::command::FfmpegCommand;
    use hydra_rust::stream::{ice_config, vp8_media_engine, VP8_PAYLOAD_TYPE};
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
    use webrtc::runtime::{channel, AsyncUdpSocket, Sender};

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
            println!("connection state: {state}");
            if state == RTCPeerConnectionState::Connected {
                let _ = self.connected_tx.try_send(());
            }
        }
    }

    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| e.to_string())?;
    println!("Listening on 0.0.0.0:{port} - waiting for an initStream receiver to connect...");
    println!("(pass this machine's LAN IP and this port to s0.initStream(\"host:{port}\") elsewhere)");
    let (tcp, peer_addr) = listener.accept().map_err(|e| e.to_string())?;
    println!("receiver connected from {peer_addr}");
    let mut reader = BufReader::new(tcp.try_clone().map_err(|e| e.to_string())?);
    let mut writer = tcp;

    let (media_engine, registry) = vp8_media_engine()?;
    let config = ice_config().build();
    let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
    let (connected_tx, mut connected_rx) = channel::<()>(1);
    let handler = Arc::new(Handler { gather_complete_tx, connected_tx });

    let video_codec = RTCRtpCodec {
        mime_type: rtc::peer_connection::configuration::media_engine::MIME_TYPE_VP8.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line: "".to_owned(),
        rtcp_feedback: vec![],
    };
    let ssrc = std::process::id().wrapping_mul(2_654_435_761);
    let video_track: Arc<TrackLocalStaticRTP> = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
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
        .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal>)
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

    println!("waiting for connection...");
    let _ = connected_rx.recv().await;
    println!("connected! starting ffmpeg encoder...");

    // Bind the receiving socket before ffmpeg starts sending to it.
    let rtp_port = UdpSocket::bind("127.0.0.1:0")
        .and_then(|s| s.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())?;
    let std_sock = UdpSocket::bind(("127.0.0.1", rtp_port)).map_err(|e| e.to_string())?;
    let sock: Arc<dyn AsyncUdpSocket> = runtime.wrap_udp_socket(std_sock).map_err(|e| e.to_string())?;

    let mut command = FfmpegCommand::new();
    if input.is_empty() {
        command.format("lavfi").input("testsrc=size=640x480:rate=25");
    } else {
        if let Some(fmt) = &input_format {
            command.format(fmt);
        }
        command.input(&input);
    }
    command
        .codec_video("libvpx")
        .args(["-deadline", "realtime", "-cpu-used", "4", "-b:v", "1M", "-g", "25"])
        .format("rtp")
        .args(["-payload_type", &VP8_PAYLOAD_TYPE.to_string()])
        .output(format!("udp://127.0.0.1:{rtp_port}"));
    let mut ffmpeg = command.spawn().map_err(|e| format!("spawn ffmpeg: {e}"))?;

    println!("broadcasting - press Ctrl+C to stop");
    let mut buf = vec![0u8; 1500];
    loop {
        let (n, _) = sock.recv_from(&mut buf).await.map_err(|e| e.to_string())?;
        let mut bytes = BytesMut::from(&buf[..n]);
        if let Ok(mut packet) = rtc::rtp::packet::Packet::unmarshal(&mut bytes) {
            packet.header.ssrc = ssrc;
            if video_track.write_rtp(packet).await.is_err() {
                break;
            }
        }
    }

    let _ = ffmpeg.kill();
    let _ = peer_connection.close().await;
    Ok(())
}
