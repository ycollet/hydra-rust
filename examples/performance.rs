use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[cfg(feature = "gui")]
#[path = "../src/bin/hydra/highlight.rs"]
mod highlight;

#[cfg(target_os = "macos")]
#[path = "performance/gpu.rs"]
mod gpu;
#[cfg(target_os = "macos")]
#[path = "../tests/support/native_gl.rs"]
mod native_gl;

#[derive(Clone, Copy, Default)]
struct AllocationCounts {
    calls: u64,
    requested_bytes: u64,
}

thread_local! {
    static COUNTS: Cell<Option<AllocationCounts>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn allocated(bytes: usize) {
    let _ = COUNTS.try_with(|counts| {
        if let Some(mut value) = counts.get() {
            value.calls += 1;
            value.requested_bytes += bytes as u64;
            counts.set(Some(value));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            allocated(size);
        }
        result
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct CountScope;

impl CountScope {
    fn start() -> Self {
        COUNTS.set(Some(AllocationCounts::default()));
        Self
    }

    fn finish(self) -> AllocationCounts {
        COUNTS.replace(None).unwrap()
    }
}

impl Drop for CountScope {
    fn drop(&mut self) {
        COUNTS.set(None);
    }
}

#[derive(Debug)]
struct Options {
    samples: usize,
    iterations: usize,
    gpu: bool,
    output: Option<PathBuf>,
    label: String,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self {
            samples: 15,
            iterations: 20,
            gpu: false,
            output: None,
            label: String::new(),
        };
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--gpu" => options.gpu = true,
                "--samples" | "--iterations" => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("{argument} needs a value"))?;
                    let value = value
                        .parse::<usize>()
                        .map_err(|_| format!("Invalid {argument}: {value}"))?;
                    if !(1..=10_000).contains(&value) {
                        return Err(format!("{argument} must be between 1 and 10000"));
                    }
                    if argument == "--samples" {
                        options.samples = value;
                    } else {
                        options.iterations = value;
                    }
                }
                "--output" => {
                    options.output = Some(args.next().ok_or("--output needs a path")?.into())
                }
                "--label" => options.label = args.next().ok_or("--label needs a value")?,
                _ => return Err(format!("Unknown argument: {argument}")),
            }
        }
        Ok(options)
    }
}

#[derive(Serialize)]
struct Measurement {
    name: String,
    timing_kind: String,
    median_us: f64,
    p95_us: f64,
    median_allocation_calls: f64,
    median_requested_bytes: f64,
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[cfg(feature = "javascript")]
fn callback_script(count: usize) -> String {
    if count == 0 {
        return "solid(0,0,0,1).out()".into();
    }
    let mut nodes: Vec<_> = (0..count)
        .map(|index| format!("solid(()=>time+{index},0,0,1)"))
        .collect();
    while nodes.len() > 1 {
        nodes = nodes
            .chunks(2)
            .map(|pair| {
                if pair.len() == 2 {
                    format!("{}.blend({},0.5)", pair[0], pair[1])
                } else {
                    pair[0].clone()
                }
            })
            .collect();
    }
    format!("{}.out()", nodes[0])
}

fn measure(
    results: &mut Vec<Measurement>,
    options: &Options,
    name: &str,
    kind: &str,
    mut operation: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    for _ in 0..3 {
        operation()?;
    }
    let mut timings = Vec::with_capacity(options.samples);
    for _ in 0..options.samples {
        let start = Instant::now();
        for _ in 0..options.iterations {
            operation()?;
        }
        timings.push(start.elapsed().as_secs_f64() * 1e6 / options.iterations as f64);
    }
    let mut calls = Vec::with_capacity(options.samples);
    let mut bytes = Vec::with_capacity(options.samples);
    for _ in 0..options.samples {
        let scope = CountScope::start();
        for _ in 0..options.iterations {
            operation()?;
        }
        let counts = scope.finish();
        calls.push(counts.calls as f64 / options.iterations as f64);
        bytes.push(counts.requested_bytes as f64 / options.iterations as f64);
    }
    timings.sort_by(f64::total_cmp);
    calls.sort_by(f64::total_cmp);
    bytes.sort_by(f64::total_cmp);
    let measurement = Measurement {
        name: name.into(),
        timing_kind: kind.into(),
        median_us: percentile(&timings, 50),
        p95_us: percentile(&timings, 95),
        median_allocation_calls: percentile(&calls, 50),
        median_requested_bytes: percentile(&bytes, 50),
    };
    eprintln!(
        "{name:38} {:10.2} us  p95 {:10.2} us  {:10.1} allocations/op",
        measurement.median_us, measurement.p95_us, measurement.median_allocation_calls
    );
    results.push(measurement);
    Ok(())
}

fn cpu(
    results: &mut Vec<Measurement>,
    options: &Options,
    skipped: &mut Vec<String>,
) -> Result<(), String> {
    measure(results, options, "rhai/eval_simple", "cpu_wall", || {
        black_box(hydra_rust::eval(black_box("osc(10).rotate(0.2).out()"))?);
        Ok(())
    })?;
    let large = format!("{}osc(10).out();", "let value = 1 + 2;\n".repeat(1200));
    measure(
        results,
        options,
        &format!("rhai/preprocess_{}_bytes", large.len()),
        "cpu_wall",
        || {
            black_box(hydra_rust::preprocess(black_box(&large)));
            Ok(())
        },
    )?;
    #[cfg(feature = "fft")]
    {
        use hydra_rust::audio::{AudioAnalyzer, AudioBuffer};
        let buffer = AudioBuffer::new();
        let mut analyzer = AudioAnalyzer::new();
        let samples: Vec<_> = (0..800).map(|i| (i as f32 * 0.1).sin()).collect();
        for _ in 0..128 {
            buffer.push(&samples[..512], 1);
        }
        measure(
            results,
            options,
            "audio/push_512_full_queue",
            "cpu_wall",
            || {
                buffer.push(black_box(&samples[..512]), 1);
                Ok(())
            },
        )?;
        analyzer.poll(&buffer);
        measure(results, options, "audio/poll_idle", "cpu_wall", || {
            black_box(analyzer.poll(&buffer));
            Ok(())
        })?;
        measure(results, options, "audio/push_poll_800", "cpu_wall", || {
            buffer.push(black_box(&samples), 1);
            black_box(analyzer.poll(&buffer));
            Ok(())
        })?;
    }
    #[cfg(not(feature = "fft"))]
    skipped.push("audio: enable the fft feature".into());
    #[cfg(feature = "midi")]
    for inputs in [1, 8] {
        use hydra_rust::midi::{MidiBinding, MidiBridge, MidiFilter, MidiPoller, MidiValueKind};
        let bridge = MidiBridge::new();
        for input in 0..inputs {
            bridge.register_input(input, &format!("benchmark-{input}"));
            bridge.control_change_from(input, 0, 1, 0.5);
        }
        for slot in 0..64 {
            bridge.bind(
                slot,
                MidiBinding {
                    kind: MidiValueKind::Cc,
                    index: None,
                    filter: MidiFilter::default(),
                    smooth: None,
                },
            )?;
        }
        let mut poller = MidiPoller::new(bridge.clone());
        measure(
            results,
            options,
            &format!("midi/64_wildcards_{inputs}_inputs"),
            "cpu_wall",
            || {
                black_box(poller.poll());
                Ok(())
            },
        )?;
        measure(
            results,
            options,
            &format!("midi/64_wildcards_{inputs}_inputs_with_events"),
            "cpu_wall",
            || {
                bridge.control_change_from(inputs - 1, 0, 1, 0.75);
                black_box(poller.poll());
                Ok(())
            },
        )?;
    }
    #[cfg(not(feature = "midi"))]
    skipped.push("MIDI: enable the midi feature".into());
    #[cfg(feature = "javascript")]
    for count in [0, 128] {
        use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime};
        let mut runtime =
            JavascriptRuntime::new(std::env::current_dir().map_err(|error| error.to_string())?)?;
        runtime.eval(&callback_script(count))?;
        let mut time = 0.0;
        measure(
            results,
            options,
            &format!("javascript/frame_{count}_callbacks"),
            "cpu_wall",
            || {
                time += 1.0 / 60.0;
                black_box(runtime.frame(JavascriptFrame {
                    time,
                    ..Default::default()
                })?);
                Ok(())
            },
        )?;
    }
    #[cfg(not(feature = "javascript"))]
    skipped.push("JavaScript: enable the javascript feature".into());
    #[cfg(feature = "gui")]
    {
        let highlighter = highlight::HydraHighlighter::new();
        let text = "osc(10).rotate(0.1).color(1,0.5,0.2).out()\n".repeat(1000);
        let font = eframe::egui::FontId::monospace(14.0);
        measure(
            results,
            options,
            "editor/highlight_1000_lines",
            "cpu_wall",
            || {
                black_box(highlighter.layout_job(&text, &font, eframe::egui::Color32::BLACK));
                Ok(())
            },
        )?;
    }
    #[cfg(not(feature = "gui"))]
    skipped.push("syntax highlighting: enable the gui feature".into());
    // Keep the parameter used when every optional workload is enabled.
    let _ = skipped;
    Ok(())
}

fn command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "cargo run --release --example performance -- [--gpu] [--samples N] [--iterations N] [--output FILE] [--label TEXT]"
        );
        return Ok(());
    }
    if cfg!(debug_assertions) {
        return Err("Performance measurements require --release".into());
    }
    let options = Options::parse(args)?;
    #[cfg(not(target_os = "macos"))]
    if options.gpu {
        return Err("--gpu currently requires macOS; CPU workloads remain portable".into());
    }
    let mut results = Vec::new();
    let mut skipped = Vec::new();
    cpu(&mut results, &options, &mut skipped)?;
    let graphics = if options.gpu {
        #[cfg(target_os = "macos")]
        {
            gpu::run(&mut results, &options)?
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Err("--gpu currently requires macOS".into());
        }
    } else {
        skipped.push("offscreen graphics: pass --gpu on macOS".into());
        serde_json::Value::Null
    };
    #[cfg(target_os = "macos")]
    let processor = command("sysctl", &["-n", "machdep.cpu.brand_string"])?;
    #[cfg(not(target_os = "macos"))]
    let processor = std::env::consts::ARCH.to_string();
    let report = serde_json::json!({
        "schema_version": 1,
        "environment": {
            "unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_secs(),
            "label": options.label,
            "revision": command("git", &["rev-parse", "HEAD"])?,
            "tracked_changes": command("git", &["diff", "HEAD", "--stat"])?,
            "source_status": command("git", &["status", "--short"])?,
            "rustc": command("rustc", &["--version"])?,
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "processor": processor,
            "features": {"fft": cfg!(feature="fft"), "midi": cfg!(feature="midi"), "javascript": cfg!(feature="javascript"), "gui": cfg!(feature="gui")},
            "graphics": graphics,
            "profile": "release",
        },
        "method": {
            "samples": options.samples, "iterations_per_sample": options.iterations, "warmup_operations": 3,
            "statistics": "nearest-rank median and p95 of batch means, not individual latency percentiles",
            "allocations": "separate pass; successful Rust allocation/reallocation calls on this thread only; requested bytes, not peak memory",
            "application_fps": null,
        },
        "measurements": results,
        "skipped": skipped,
    });
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(path) = options.output {
        std::fs::write(&path, format!("{json}\n"))
            .map_err(|error| format!("{}: {error}", path.display()))?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("performance: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_reject_missing_invalid_and_unknown_values() {
        for args in [
            vec!["--samples"],
            vec!["--iterations", "0"],
            vec!["--samples", "no"],
            vec!["--other"],
        ] {
            assert!(Options::parse(args.into_iter().map(String::from)).is_err());
        }
        let options =
            Options::parse(["--samples", "3", "--iterations", "2", "--gpu"].map(String::from))
                .unwrap();
        assert_eq!(
            (options.samples, options.iterations, options.gpu),
            (3, 2, true)
        );
    }

    #[test]
    fn counting_scope_stops_after_errors_and_counts_only_enabled_allocations() {
        let scope = CountScope::start();
        let values = black_box(Vec::<u8>::with_capacity(black_box(123)));
        let counts = scope.finish();
        assert_eq!((counts.calls, counts.requested_bytes), (1, 123));
        drop(values);
        assert!(COUNTS.get().is_none());
        let result = measure(
            &mut Vec::new(),
            &Options::parse([]).unwrap(),
            "failure",
            "cpu_wall",
            {
                let mut calls = 0;
                move || {
                    calls += 1;
                    if calls > 3 + 15 * 20 {
                        Err("allocation pass failure".into())
                    } else {
                        Ok(())
                    }
                }
            },
        );
        assert!(result.is_err());
        assert!(COUNTS.get().is_none());
    }

    #[test]
    fn percentiles_use_nearest_rank_on_sorted_batch_means() {
        assert_eq!(percentile(&[1., 2., 3., 4.], 50), 2.);
        assert_eq!(percentile(&[1., 2., 3., 4.], 95), 4.);
    }

    #[cfg(feature = "fft")]
    #[test]
    fn warmed_audio_polling_reuses_storage_for_idle_and_active_samples() {
        use hydra_rust::audio::{AudioAnalyzer, AudioBuffer};
        let buffer = AudioBuffer::new();
        let mut analyzer = AudioAnalyzer::new();
        let samples = [0.25; 800];
        buffer.push(&samples, 1);
        analyzer.poll(&buffer);
        let scope = CountScope::start();
        for _ in 0..8 {
            black_box(analyzer.poll(&buffer));
            buffer.push(&samples, 1);
            black_box(analyzer.poll(&buffer));
        }
        let counts = scope.finish();
        assert_eq!(counts.calls, 0);
    }

    #[test]
    fn large_plain_sketch_preprocessing_does_not_allocate_per_identifier() {
        let large = format!("{}osc(10).out();", "let value = 1 + 2;\n".repeat(1200));
        let scope = CountScope::start();
        let processed = hydra_rust::preprocess(&large);
        let counts = scope.finish();
        assert!(processed.contains("osc(10).out()"));
        assert!(counts.calls < 100, "{} allocations", counts.calls);
    }

    #[cfg(feature = "javascript")]
    #[test]
    fn callback_workload_keeps_all_128_callbacks_reachable() {
        use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime};
        let mut runtime = JavascriptRuntime::new(std::env::current_dir().unwrap()).unwrap();
        let graph = runtime.eval(&callback_script(128)).unwrap();
        assert!(
            graph.shaders[0]
                .as_ref()
                .unwrap()
                .contains("iJsValues[127]")
        );
        let values = runtime
            .frame(JavascriptFrame {
                time: 1.0,
                ..Default::default()
            })
            .unwrap();
        for (index, value) in values.iter().enumerate() {
            assert_eq!(*value, index as f32 + 1.0);
        }
        let scope = CountScope::start();
        let values = runtime
            .frame(JavascriptFrame {
                time: 2.0,
                ..Default::default()
            })
            .unwrap();
        let counts = scope.finish();
        assert_eq!(counts.calls, 0);
        for (index, value) in values.iter().enumerate() {
            assert_eq!(*value, index as f32 + 2.0);
        }
    }
}
