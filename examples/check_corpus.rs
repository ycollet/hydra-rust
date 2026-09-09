//! Conformance-check a directory tree of `.hydra` sketch files against
//! `hydra_rust::eval()` directly (no window, no GL context), so large
//! corpora can be checked in seconds rather than minutes.
//!
//! Usage: cargo run --release --features webcam,audio --example check_corpus -- <dir> [out.json]
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn collect_hydra_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_hydra_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "hydra") {
            out.push(path);
        }
    }
}

/// Groups similar errors together: "Function not found: X (...)" errors are
/// keyed by the missing function name; everything else is keyed by the
/// message with the trailing "(line N, position M)" location stripped.
fn bucket_key(err: &str) -> String {
    if let Some(rest) = err.strip_prefix("Function not found: ") {
        let name = rest.split(' ').next().unwrap_or(rest);
        return format!("missing function: {name}");
    }
    let re = regex::Regex::new(r" \(line \d+, position \d+\)$").unwrap();
    re.replace(err, "").to_string()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: check_corpus <dir-of-.hydra-files> [out.json]");
        std::process::exit(1);
    };
    let out_path = args.next().unwrap_or_else(|| "check_corpus_failures.json".to_string());

    let mut files = Vec::new();
    collect_hydra_files(Path::new(&dir), &mut files);
    files.sort();
    println!("found {} .hydra file(s)", files.len());

    let mut ok = 0usize;
    let mut buckets: HashMap<String, usize> = HashMap::new();
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    let start = Instant::now();
    for path in &files {
        let Ok(code) = fs::read_to_string(path) else { continue };
        match hydra_rust::eval(&code) {
            Ok(_) => ok += 1,
            Err(e) => {
                *buckets.entry(bucket_key(&e)).or_insert(0) += 1;
                failures.push((path.clone(), e));
            }
        }
    }
    let elapsed = start.elapsed();
    let total = files.len().max(1);

    println!(
        "ok: {ok}  failed: {}  ({:.1}% failed)  elapsed: {:.2?}",
        failures.len(),
        100.0 * failures.len() as f64 / total as f64,
        elapsed
    );
    println!("\ntop failure buckets:");
    let mut sorted: Vec<_> = buckets.into_iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (key, count) in sorted.iter().take(40) {
        println!("  {count:>6}  {key}");
    }

    let dump: Vec<_> = failures
        .iter()
        .map(|(p, e)| serde_json::json!({"file": p, "error": e}))
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&dump) {
        let _ = fs::write(&out_path, json);
        println!("\nfull failure list written to {out_path}");
    }
}
