//! Measure a saved fixture. Input reads and output writes are outside the clock.
use std::error::Error;
use std::time::Instant;

use md_to_docx::convert_with_options;
use serde_json::json;

fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        return Err("use a release build".into());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("compare_fixture INPUT OPTIONS OUTPUT WARMUPS RUNS".into());
    }
    let markdown = std::fs::read_to_string(&args[0])?;
    let options =
        md_to_docx::options_json::load_options_json(&std::fs::read_to_string(&args[1])?, &args[1])?
            .convert;
    let warmups: usize = args[3].parse()?;
    let runs: usize = args[4].parse()?;
    if runs == 0 {
        return Err("RUNS must be positive".into());
    }
    let mut warmup_ms = Vec::new();
    for _ in 0..warmups {
        let start = Instant::now();
        let bytes = convert_with_options(&markdown, &options)?;
        warmup_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&bytes);
    }
    let mut samples_ms = Vec::new();
    let mut bytes = Vec::new();
    for _ in 0..runs {
        drop(bytes);
        let start = Instant::now();
        bytes = convert_with_options(&markdown, &options)?;
        samples_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    std::fs::write(&args[2], &bytes)?;
    println!(
        "{}",
        json!({
            "engine": "rust-core", "input_bytes": markdown.len(),
            "input_chars": markdown.chars().count(), "docx_bytes": bytes.len(),
            "warmup_ms": warmup_ms, "samples_ms": samples_ms,
        })
    );
    Ok(())
}
