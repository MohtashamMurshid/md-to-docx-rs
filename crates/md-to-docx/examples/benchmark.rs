//! Reproducible native conversion benchmarks. Run in release mode:
//! cargo run --locked --release -p md-to-docx --example benchmark
//!
//! These generated fixtures are NEW, not the unavailable historical fixtures.
//! Export them with --fixtures DIR for measurements by another implementation.
//! --case NAME chooses a case; --runs N overrides its sample count.
//! --memory converts once without warmup. Run a fresh process for each case.
//! Linux reports RSS at return and process high-water BEFORE unzip/validation.
//! Other platforms emit null memory readings. No Node addon is measured.
use std::collections::BTreeMap;
use std::error::Error;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::time::Instant;

use md_to_docx::options::{ChartRenderingOptions, CodeHighlightingOptions, OptionBool};
use md_to_docx::{convert_with_options, ConvertOptions};
use serde_json::{json, Value};

struct Case {
    name: &'static str,
    markdown: String,
    options: ConvertOptions,
    runs: usize,
}

fn cases() -> Vec<Case> {
    let prose = (0..3200).map(|i| format!(
        "Paragraph {i}: **bold**, *italic*, <u>underline</u>, ~~strike~~ and [a link](https://example.com/{}) with ordinary prose.", i % 25
    )).collect::<Vec<_>>().join("\n\n");
    let lists = (0..1200)
        .map(|i| {
            format!(
                "1. List {i} first item\n2. Second item\n   1. Nested ordered item\n3. Third item"
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let code = (0..8).map(|block| {
        let lines = (0..80).map(|i| format!(
            "export function compute{block}_{i}(value: number): string {{ return `result: ${{value + {i}}}`; }}"
        )).collect::<Vec<_>>().join("\n");
        format!("```ts\n{lines}\n```")
    }).collect::<Vec<_>>().join("\n\n");
    let highlighted = ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(true)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let chart = ConvertOptions {
        chart_rendering: Some(ChartRenderingOptions {
            enabled: Some(OptionBool::Bool(true)),
            ..Default::default()
        }),
        ..Default::default()
    };
    vec![
        Case { name: "prose", markdown: prose, options: ConvertOptions::default(), runs: 4 },
        Case { name: "lists", markdown: lists, options: ConvertOptions::default(), runs: 3 },
        Case { name: "plain-code", markdown: code.clone(), options: ConvertOptions::default(), runs: 6 },
        Case { name: "highlighted-ts", markdown: code, options: highlighted, runs: 6 },
        Case { name: "chart", markdown: "```chart\n{\"type\":\"line\",\"data\":{\"labels\":[\"A\",\"B\",\"C\",\"D\",\"E\"],\"datasets\":[{\"label\":\"Series\",\"data\":[1,4,2,6,3]}]},\"width\":1200,\"height\":800}\n```".into(), options: chart, runs: 3 },
    ]
}

fn memory_kib() -> Value {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return Value::Null;
    };
    let reading = |key: &str| {
        status.lines().find_map(|line| {
            line.strip_prefix(key)?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    };
    json!({"rss_kib": reading("VmRSS:"), "high_water_kib": reading("VmHWM:")})
}

fn inspect(case: &Case, bytes: &[u8]) -> Result<Value, Box<dyn Error>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut sizes = BTreeMap::new();
    let mut document = String::new();
    let mut chart_dimensions = None;
    for i in 0..archive.len() {
        let mut part = archive.by_index(i)?;
        let name = part.name()?.into_owned();
        sizes.insert(name.clone(), part.size());
        if name == "word/document.xml" {
            part.read_to_string(&mut document)?;
        } else if case.name == "chart" && name.starts_with("word/media/") {
            let mut image_bytes = Vec::new();
            part.read_to_end(&mut image_bytes)?;
            let png = image::load_from_memory(&image_bytes)?;
            assert_eq!((png.width(), png.height()), (1200, 800));
            chart_dimensions = Some([png.width(), png.height()]);
        }
    }
    let colored_code_runs = document.matches("<w:color w:val=").count();
    if case.name == "highlighted-ts" {
        let spans = md_to_docx::highlight::highlight_code(
            "export function f(x: number): string { return `value ${x + 1}`; }",
            "ts",
        );
        assert!(
            spans.iter().any(|span| span.color.is_some()),
            "TypeScript grammar did not color tokens"
        );
        assert!(document.contains("D73A49"), "DOCX has no keyword color");
    }
    if case.name == "chart" {
        assert!(chart_dimensions.is_some(), "chart PNG missing");
    }
    Ok(
        json!({"parts_uncompressed_bytes": sizes, "color_elements": colored_code_runs, "chart_dimensions": chart_dimensions}),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut case_name = None;
    let mut runs_override = None;
    let mut fixtures: Option<PathBuf> = None;
    let mut memory = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--case" => case_name = Some(args.next().ok_or("--case needs a name")?),
            "--runs" => {
                runs_override = Some(
                    args.next()
                        .ok_or("--runs needs a number")?
                        .parse::<usize>()?,
                )
            }
            "--fixtures" => {
                fixtures = Some(args.next().ok_or("--fixtures needs a directory")?.into())
            }
            "--memory" => memory = true,
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if runs_override == Some(0) {
        return Err("--runs must be positive".into());
    }
    if memory && case_name.is_none() {
        return Err("--memory requires --case so each process measures one workload".into());
    }
    if cfg!(debug_assertions) {
        return Err("use cargo run --release for benchmarks".into());
    }
    let selected = cases()
        .into_iter()
        .filter(|case| case_name.as_ref().is_none_or(|name| name == case.name))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err("unknown case; use prose, lists, plain-code, highlighted-ts or chart".into());
    }
    if let Some(path) = &fixtures {
        std::fs::create_dir_all(path)?;
    }
    for case in selected {
        if let Some(path) = &fixtures {
            std::fs::write(path.join(format!("{}.md", case.name)), &case.markdown)?;
            let options = match case.name {
                "highlighted-ts" => json!({"codeHighlighting": {"enabled": true}}),
                "chart" => json!({"chartRendering": {"enabled": true}}),
                _ => json!({}),
            };
            std::fs::write(
                path.join(format!("{}.options.json", case.name)),
                serde_json::to_vec_pretty(&options)?,
            )?;
        }
        let warmup = if memory {
            None
        } else {
            let started = Instant::now();
            let _ = convert_with_options(&case.markdown, &case.options)?;
            Some(started.elapsed().as_secs_f64() * 1000.0)
        };
        let mut samples = Vec::new();
        let mut bytes = Vec::new();
        for _ in 0..if memory {
            1
        } else {
            runs_override.unwrap_or(case.runs)
        } {
            drop(bytes);
            let started = Instant::now();
            bytes = convert_with_options(&case.markdown, &case.options)?;
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let process_memory = if memory { memory_kib() } else { Value::Null };
        let output = inspect(&case, &bytes)?;
        samples.sort_by(f64::total_cmp);
        let middle = samples.len() / 2;
        let median = if samples.len() % 2 == 0 {
            (samples[middle - 1] + samples[middle]) / 2.0
        } else {
            samples[middle]
        };
        println!(
            "{}",
            json!({
                "fixture_version": 1, "historical_fixture": false, "case": case.name,
                "input_chars": case.markdown.chars().count(), "input_bytes": case.markdown.len(),
                "warmup_ms": warmup, "runs": samples.len(), "median_ms": median,
                "min_ms": samples[0], "max_ms": samples[samples.len() - 1], "samples_ms": samples,
                "docx_bytes": bytes.len(), "memory_before_inspection": process_memory, "output": output
            })
        );
    }
    Ok(())
}
