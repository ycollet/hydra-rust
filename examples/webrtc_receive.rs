//! A companion CLI tool for testing/verifying `s0.initStream("host:port")`
//! (see `src/stream.rs`) outside the full `hydra` GUI app - the "client"
//! counterpart to `examples/webrtc_broadcast.rs` (the "server"). Connects
//! and receives decoded frames through the exact same `eval()` ->
//! `SourceRequest::InitStream` -> `StreamManager` path the real `hydra`
//! binary uses, then reports what arrives.
//!
//! Usage:
//! ```text
//! cargo run --features stream --example webrtc_receive -- 127.0.0.1:9000
//! cargo run --features stream --example webrtc_receive -- 127.0.0.1:9000 --save frame.ppm
//! ```
//! `--save <path.ppm>` writes the first received frame to a plain `.ppm`
//! image (no extra dependency needed - just a short header in front of the
//! already-RGB8 pixel bytes), so you can open it and confirm what's
//! actually coming through, not just trust a frame-count log line.

#[cfg(not(feature = "stream"))]
fn main() {
    eprintln!("this example requires --features stream");
    std::process::exit(1);
}

#[cfg(feature = "stream")]
fn main() {
    env_logger::init();

    let mut args = std::env::args().skip(1);
    let Some(addr) = args.next() else {
        eprintln!("usage: webrtc_receive <host:port> [--save <path.ppm>]");
        std::process::exit(1);
    };
    let mut save_path: Option<String> = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--save" => save_path = args.next(),
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }

    // Goes through the exact same eval() -> SourceRequest -> StreamManager
    // path app.rs's evaluate()/paint_background() do - this is a faithful
    // stand-in for the real `hydra` binary, not a reimplementation.
    let script = format!("s0.initStream(\"{addr}\").out()");
    let result = hydra_rust::eval(&script).expect("eval");

    let mut mgr = hydra_rust::stream::StreamManager::new();
    for req in &result.source_requests {
        if let hydra_rust::eval::SourceRequest::InitStream { slot, addr } = req {
            println!("connecting to {addr} (slot {slot})...");
            mgr.init_stream(*slot, addr.clone());
        }
    }

    println!("waiting for frames - press Ctrl+C to stop");
    let mut count: u64 = 0;
    let mut saved = false;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let Some(frame) = mgr.poll(0) else { continue };
        count += 1;
        if count % 10 == 1 {
            println!("received {count} frame(s) so far - latest {}x{}", frame.width, frame.height);
        }
        if !saved && let Some(path) = &save_path {
            match save_ppm(path, &frame) {
                Ok(()) => println!("saved a frame to {path}"),
                Err(e) => eprintln!("failed to save {path}: {e}"),
            }
            saved = true;
        }
    }
}

#[cfg(feature = "stream")]
fn save_ppm(path: &str, frame: &hydra_rust::SourceFrame) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    write!(file, "P6\n{} {}\n255\n", frame.width, frame.height)?;
    file.write_all(&frame.pixels)
}
