"""Run identical saved inputs sequentially, preserving every timing and DOCX.

macOS usage: python3 scripts/benchmark/compare.py output/current-comparison
Prepare fixtures with the release `benchmark --fixtures` example and build
`compare_fixture` and the Node addon first. Timings exclude process startup.
"""
import datetime
import hashlib
import json
import platform
import re
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]).resolve()
FIXTURES = OUT / "fixtures"
RUNS, WARMUPS = 12, 2
CASES = ["prose", "lists", "plain-code", "highlighted-ts", "chart", "plugin-fallback"]


def capture(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def command(engine, case, output, warmups, runs):
    args = [str(FIXTURES / f"{case}.md"), str(FIXTURES / f"{case}.options.json"), str(output), str(warmups), str(runs)]
    if engine == "rust-core":
        return [str(ROOT / "target/release/examples/compare_fixture"), *args]
    return ["node", "--expose-gc", str(ROOT / "scripts/benchmark/node.mjs"), engine, *args]


def run(engine, case, memory=False):
    label = f"{case}-{engine}" + ("-memory" if memory else "")
    args = command(engine, case, OUT / "docx" / f"{label}.docx", 0 if memory else WARMUPS, 1 if memory else RUNS)
    if memory:
        args = ["/usr/bin/time", "-l", *args]
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True)
    (OUT / "raw" / f"{label}.stdout.json").write_text(result.stdout)
    (OUT / "raw" / f"{label}.stderr.txt").write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(f"{label} failed: {result.stderr}")
    data = json.loads(result.stdout.strip().splitlines()[-1])
    if memory:
        match = re.search(r"(\d+)\s+maximum resident set size", result.stderr)
        if not match:
            raise RuntimeError("macOS /usr/bin/time peak RSS was not returned")
        data["peak_rss_bytes"] = int(match.group(1))
        data["peak_rss_mib"] = int(match.group(1)) / 2**20
    else:
        samples = data["samples_ms"]
        data.update(median_ms=statistics.median(samples), min_ms=min(samples), max_ms=max(samples))
    print(f"{label}: {data.get('median_ms', data['samples_ms'][0]):.2f} ms", flush=True)
    return data


def main():
    if sys.platform != "darwin":
        raise RuntimeError("this runner currently reads macOS time -l memory units")
    for path in [OUT / "raw", OUT / "docx"]:
        path.mkdir(parents=True, exist_ok=True)
    # A real no-op JavaScript AST plugin forces the documented compatibility path.
    prose = (FIXTURES / "prose.md").read_text()
    (FIXTURES / "plugin-fallback.md").write_text("\n\n".join(prose.split("\n\n")[:200]))
    (FIXTURES / "plugin-fallback.options.json").write_text('{"benchmarkPlugin":true}\n')
    metadata = {
        "date": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "git_commit": capture("git", "rev-parse", "HEAD"),
        "git_dirty": bool(capture("git", "status", "--porcelain")),
        "node_highlighter": "lowlight via callback bridge",
        "core_highlighter": "Syntect",
        "cpu": capture("sysctl", "-n", "machdep.cpu.brand_string"),
        "logical_cpus": int(capture("sysctl", "-n", "hw.logicalcpu")),
        "ram_bytes": int(capture("sysctl", "-n", "hw.memsize")),
        "os": platform.platform(), "node": capture("node", "--version"),
        "rustc": capture("rustc", "--version", "-v"),
        "npm": json.loads(capture("npm", "ls", "--prefix", "crates/md-to-docx-napi", "@mohtasham/md-to-docx", "docx", "lowlight", "remark", "sharp", "katex", "--depth=3", "--json")),
        "warmups": WARMUPS, "runs": RUNS, "historical_fixtures": False,
        "method": "Fresh process per engine/case; sequential engines with rotated order; median of 12 calls after two warmups; no forced GC between calls; input/import/output I/O excluded.",
        "memory_method": "Separate fresh process, zero warmups, one conversion; macOS time -l maximum resident set size in bytes, entire process including imports; no ZIP inspection in that process.",
        "timed_api": {"typescript": "convertMarkdownToDocx -> Blob", "rust-node": "convertMarkdownToDocx -> Blob", "rust-core": "convert_with_options -> Vec<u8>"},
    }
    selected = set(sys.argv[2:]) or set(CASES)
    if not selected.issubset(CASES):
        raise RuntimeError("unknown case filter")
    previous = json.loads((OUT / "results.json").read_text()) if (OUT / "results.json").exists() and selected != set(CASES) else {"results": []}
    by_case = {item["case"]: item for item in previous["results"]}
    metadata["fixture_version"] = 2
    for index, case in enumerate(CASES):
        if case not in selected:
            continue
        engines = ["typescript", "rust-node", "rust-core"] if case != "plugin-fallback" else ["typescript", "rust-node"]
        offset = index % len(engines)
        engines = engines[offset:] + engines[:offset]
        data = {"case": case, "input_sha256": hashlib.sha256((FIXTURES / f"{case}.md").read_bytes()).hexdigest(),
                "options_sha256": hashlib.sha256((FIXTURES / f"{case}.options.json").read_bytes()).hexdigest(), "engines": {}}
        for engine in engines:
            timing = run(engine, case)
            memory = run(engine, case, memory=True)
            data["engines"][engine] = {**timing, "cold": memory}
        ts = data["engines"]["typescript"]["median_ms"]
        for engine, measurement in data["engines"].items():
            measurement["typescript_time_divided_by_this"] = ts / measurement["median_ms"]
        by_case[case] = data
        results = [by_case[name] for name in CASES if name in by_case]
        (OUT / "results.json").write_text(json.dumps({"metadata": metadata, "results": results}, indent=2) + "\n")
    probe = OUT / "compatibility-probe"
    probe.mkdir(exist_ok=True)
    (probe / "html-underline.md").write_text("HTML underline: <u>word</u>")
    (probe / "options.json").write_text("{}")
    for engine in ["typescript", "rust-node"]:
        args = ["node", "--expose-gc", str(ROOT / "scripts/benchmark/node.mjs"), engine,
                str(probe / "html-underline.md"), str(probe / "options.json"), str(probe / f"{engine}.docx"), "0", "1"]
        result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
        (probe / f"{engine}.json").write_text(result.stdout)


if __name__ == "__main__":
    main()
