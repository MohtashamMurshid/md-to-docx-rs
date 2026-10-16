"""Generate standalone benchmark charts, a self-contained viewer and data exports.

Needs matplotlib. Usage: python visualize.py output/current-comparison
"""
import base64
import csv
import html
import json
import sys
import zipfile
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator

OUT = Path(sys.argv[1]).resolve()
DATA = json.loads((OUT / "results.json").read_text())
META = DATA["metadata"]
PROVENANCE = (
    f"{META['cpu']} · {META['logical_cpus']} logical CPUs · {META['ram_bytes'] / 2**30:g} GiB RAM · {META['os']}\n"
    f"Node {META['node']} · {META['rustc'].splitlines()[0]} · {META['date']}"
)
CHECK_COUNT = len(DATA.get("verification", {}).get("passed_checks", []))
MATCHED_XML_CASES = [row["case"] for row in DATA["results"]
    if all(engine in row["engines"] for engine in ("rust-node", "rust-core"))
    and row["engines"]["rust-node"]["output"]["document_xml_sha256"] == row["engines"]["rust-core"]["output"]["document_xml_sha256"]]
XML_CLAIM = f"Native Node and core document XML match exactly for {len(MATCHED_XML_CASES)} checked cases: {', '.join(MATCHED_XML_CASES)}. Highlighting can differ because core uses Syntect."
BG, FG, MUTED, GRID, TS, RUST = "#07090C", "#F3F5F7", "#A5AFBA", "#29313B", "#8994A2", "#58DDB4"
plt.rcParams.update({"font.family": "DejaVu Sans", "svg.fonttype": "none", "text.color": FG,
                     "axes.labelcolor": MUTED, "xtick.color": MUTED, "ytick.color": MUTED})
NAMES = {
    "prose": ("Mixed prose", "3,200 paragraphs · bold, italic, underline, strike, links"),
    "lists": ("Ordered lists", "1,200 lists · four items per list, including one nested"),
    "plain-code": ("Plain TypeScript", "Eight blocks · 640 functions · 55,606 characters"),
    "highlighted-ts": ("Highlighted TypeScript", "Same code · real token colors checked in both outputs"),
    "chart": ("Line chart", "1200 × 800 · identical pixels checked · different PNG writers"),
    "plugin-fallback": ("JavaScript plugin", "200 paragraphs · original TypeScript compatibility path"),
}


def number(value, metric):
    return f"{value:,.1f}" if metric == "memory" or value >= 10 else f"{value:,.2f}"


def chart(engine, metric):
    rows = [r for r in DATA["results"] if engine in r["engines"]]
    fig, axes = plt.subplots(3, 2, figsize=(16, 11.2), facecolor=BG)
    fig.subplots_adjust(left=.098, right=.947, top=.75, bottom=.16, hspace=1.00, wspace=.49)
    for ax in axes.flat:
        ax.set_facecolor(BG)
    binding = "Rust Node addon" if engine == "rust-node" else "Rust library"
    title = "Conversion time" if metric == "time" else "Peak process memory"
    fig.text(.055, .939, "md-to-docx  /  FRESH MEASUREMENTS", color=MUTED, fontsize=12, weight="bold")
    fig.text(.055, .891, title, fontsize=30, weight="bold")
    fig.text(.055, .850, f"TypeScript 4.0.1 vs {binding}   ·   same inputs and options   ·   lower is better", color=MUTED, fontsize=13)
    unit = "ms" if metric == "time" else "MiB"
    for i, row in enumerate(rows):
        ax = axes.flat[i]
        name = row["case"]
        ts, rust = row["engines"]["typescript"], row["engines"][engine]
        values = [ts["median_ms"], rust["median_ms"]] if metric == "time" else [ts["cold"]["peak_rss_mib"], rust["cold"]["peak_rss_mib"]]
        largest_sample = max(ts["max_ms"], rust["max_ms"]) if metric == "time" else max(values)
        limit = largest_sample * 1.35
        ax.set_xlim(0, limit)
        ax.set_ylim(-.55, 1.55)
        ax.invert_yaxis()
        ax.barh([0, 1], values, height=.36, color=[TS, RUST], zorder=3)
        if metric == "time":
            bounds = [(ts["min_ms"], ts["max_ms"]), (rust["min_ms"], rust["max_ms"])]
            for y, (a, b) in enumerate(bounds):
                ax.plot([a, b], [y, y], color=FG, linewidth=1.2, zorder=4)
                ax.vlines([a, b], y-.075, y+.075, color=FG, linewidth=1, zorder=4)
        for y, value in enumerate(values):
            label_at = max(value, bounds[y][1]) if metric == "time" else value
            ax.text(label_at + limit*.027, y, number(value, metric), va="center", fontsize=13, weight="bold", color=FG)
        ax.set_yticks([0, 1], ["TS 4.0.1", "Rust Node" if engine == "rust-node" else "Rust core"], fontsize=11)
        ax.tick_params(axis="y", length=0, pad=8)
        ax.tick_params(axis="x", length=0, labelsize=10, pad=5)
        ax.xaxis.set_major_locator(MaxNLocator(nbins=3, integer=True))
        ax.grid(axis="x", color=GRID, linewidth=.65, zorder=0)
        for spine in ax.spines.values():
            spine.set_visible(False)
        ax.axvline(0, color=MUTED, linewidth=.6)
        ax.text(0, 1.34, NAMES[name][0], transform=ax.transAxes, fontsize=17, weight="bold")
        ax.text(0, 1.16, NAMES[name][1], transform=ax.transAxes, color=MUTED, fontsize=9.2)
        ax.text(1.0, -.23, unit, transform=ax.transAxes, color=MUTED, fontsize=10, ha="right")
        if name == "plugin-fallback":
            label = "SAME TS PATH"
        elif metric == "time":
            label = f"{ts['median_ms']/rust['median_ms']:.1f}× faster"
        else:
            label = f"{(1-values[1]/values[0])*100:.0f}% less"
        ax.text(1, 1.34, label, transform=ax.transAxes, ha="right", color=RUST, fontsize=12, weight="bold")
    for ax in list(axes.flat)[len(rows):]:
        ax.axis("off")
        ax.text(0, .90, "Native library measurement", transform=ax.transAxes, fontsize=17, weight="bold")
        ax.text(0, .63, "Rust runs without Node.\nJavaScript plugins use the Node package's TS fallback.\nThat case is shown in the Node comparison.", transform=ax.transAxes, color=MUTED, fontsize=11, linespacing=1.8)
    method = "Median of 12 timed conversions after 2 warmups. Whiskers show min–max. Separate linear scale in each panel." if metric == "time" else "Fresh process, one conversion, no warmup. macOS process high-water RSS. Imports and runtime are included."
    fig.text(.055, .098, method, fontsize=10.5, color=MUTED)
    meta = DATA["metadata"]
    fig.text(.055, .067, PROVENANCE, fontsize=10.5, color=MUTED)
    fig.text(.055, .035, "This chart has identical pixels but different PNG encoding. Syntax tokenizers differ. Plugin fallback gets no native speedup. Word rendering was not tested.", fontsize=9.5, color=MUTED)
    base = OUT / f"{metric}-{'node' if engine == 'rust-node' else 'core'}"
    fig.savefig(base.with_suffix(".png"), dpi=140, facecolor=BG)
    fig.savefig(base.with_suffix(".svg"), facecolor=BG)
    plt.close(fig)


def exports():
    with (OUT / "results.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["case", "engine", "input_chars", "median_ms", "min_ms", "max_ms", "ts_time_divided_by_this", "cold_peak_rss_mib", "docx_bytes", "document_xml_bytes", "input_sha256"])
        for row in DATA["results"]:
            for engine, values in row["engines"].items():
                writer.writerow([row["case"], engine, values["input_chars"], values["median_ms"], values["min_ms"], values["max_ms"], values["typescript_time_divided_by_this"], values["cold"]["peak_rss_mib"], values["docx_bytes"], values["output"]["document_xml_bytes"], row["input_sha256"]])
    lines = ["# Current TypeScript versus Rust comparison", "", f"Measured at {META['date']} with commit `{META['git_commit']}`. Dirty working tree: {META.get('git_dirty', 'not recorded')}.", "",
             PROVENANCE.replace("\n", " · "), "",
             "Same saved Markdown and options for each implementation. Two warmups then 12 timed calls in a fresh process for each engine and case. Engines ran sequentially, with rotated order. Samples include ordinary garbage collection; no forced GC between calls. Imports, fixture I/O, Blob-to-buffer copies and output writes are outside the conversion clock.", "",
             "TypeScript and the Rust Node addon call `convertMarkdownToDocx` and stop at Blob return. The core Rust call stops at `Vec<u8>` return. Peak memory uses a separate fresh process with one conversion, no warmup, measured with macOS `/usr/bin/time -l`. It includes imports and runtime allocation, without ZIP inspection. MiB means 1,048,576 bytes. Peak memory is not RSS at return.", "",
             "The TypeScript package is 4.0.1 with docx 9.5.0. Both package routes use the same installed dependencies. Sharp 0.35.5 and KaTeX 0.18.2 are pinned patched overrides; these workloads do not invoke Sharp or math. Full versions are in results.json.", "",
             "| Case | TypeScript ms | Rust Node ms | Rust core ms | Node ratio |", "| --- | ---: | ---: | ---: | ---: |"]
    for row in DATA["results"]:
        engines = row["engines"]
        ts, node = engines["typescript"], engines["rust-node"]
        core = f"{engines['rust-core']['median_ms']:.2f}" if "rust-core" in engines else "n/a"
        ratio = f"{node['typescript_time_divided_by_this']:.2f}×" if row["case"] != "plugin-fallback" else "same TS implementation"
        lines.append(f"| {NAMES[row['case']][0]} | {ts['median_ms']:.2f} | {node['median_ms']:.2f} | {core} | {ratio} |")
    lines += ["", f"All {CHECK_COUNT} output checks pass. ZIP integrity and all XML parts were checked. Prose and list counts match. Both code outputs preserve 640 functions and actual coloring is checked. Concatenated Word text matches between implementations after normalizing highlighted TypeScript's nonbreaking spaces. {XML_CLAIM}", "",
              "The prose fixture uses shared `++underline++` syntax. The separate raw HTML probe in results.json records the actual behavior of the measured implementation.", "",
              "The 1200 × 800 chart has exactly matching decoded RGBA pixels, including plotted content; its PNG encoding differs. The large chart speedup compares those full implementations. Syntect and lowlight tokenize and color code differently. Highlighted TypeScript changes some spaces to NBSP; Rust keeps ordinary spaces. No Word/LibreOffice pagination, browser DOM, remote image or callback benchmark was performed.", "",
              "The JavaScript plugin case uses an executed no-op AST transform on 200 paragraphs. The Rust Node package routes it to TypeScript. Its timing ranges overlap the direct TypeScript result; do not advertise a plugin speedup.", "",
              "These are new fixture version 2 inputs, not the old Linux fixtures. Raw samples, input hashes, options, individual DOCX outputs and memory logs are included. All values are specific to this machine and these workloads.", "",
              "Rerun from the repository root:", "", "```sh", "npm ci --prefix crates/md-to-docx-napi", "npm run build --prefix crates/md-to-docx-napi", "cargo build --locked --release -p md-to-docx --example benchmark --example compare_fixture", "target/release/examples/benchmark --runs 1 --fixtures output/current-comparison/fixtures", "python3 scripts/benchmark/compare.py output/current-comparison", "python3 scripts/benchmark/validate_outputs.py output/current-comparison", "# Install matplotlib in your Python environment to regenerate the graphs.", "python3 scripts/benchmark/visualize.py output/current-comparison", "```", ""]
    (OUT / "methodology.md").write_text("\n".join(lines))


def data_image(path):
    return "data:image/png;base64," + base64.b64encode(path.read_bytes()).decode()


def viewer():
    # Standard plot artifacts remain available as PNG/SVG, independent of this viewer.
    images = {f"{metric}-{engine}": data_image(OUT / f"{metric}-{engine}.png") for metric in ["time", "memory"] for engine in ["node", "core"]}
    # Only reencode for compact embedding; decoded pixels stay identical.
    from PIL import Image
    for engine in ["typescript", "rust-node"]:
        original = Image.open(OUT / f"chart-{engine}.png")
        original.save(OUT / f"chart-{engine}-preview.png", optimize=True)
        assert original.tobytes() == Image.open(OUT / f"chart-{engine}-preview.png").tobytes()
    chart_ts, chart_rust = data_image(OUT / "chart-typescript-preview.png"), data_image(OUT / "chart-rust-node-preview.png")
    rows = []
    for row in DATA["results"]:
        e = row["engines"]
        cells = [NAMES[row["case"]][0], f"{e['typescript']['median_ms']:.2f}", f"{e['rust-node']['median_ms']:.2f}", f"{e['rust-core']['median_ms']:.2f}" if "rust-core" in e else "TS fallback"]
        rows.append("<tr>" + "".join(f"<td>{html.escape(x)}</td>" for x in cells) + "</tr>")
    page = '''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>md-to-docx measured comparison</title><style>
html,body{margin:0;background:transparent;color:var(--foreground,#eef2f6);font-family:var(--font-sans,system-ui)}
.controls{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:12px}.controls button{border:1px solid var(--border,#424b55);border-radius:8px;background:transparent;color:inherit;padding:9px 12px;cursor:pointer;font:inherit;font-size:13px}.controls button.active{background:#58ddb4;color:#07090c;border-color:#58ddb4;font-weight:650}
.chart-panel{background:#07090c;color:#f3f5f7;padding:20px 16px;border-radius:var(--radius,12px)}.plot-meta{color:#a5afba;font-size:12px;line-height:1.6;margin-bottom:18px}.plots{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:24px 16px}.case h3{font-size:16px;letter-spacing:-.3px;margin:0 0 6px}.case .ratio{color:#58ddb4;font-size:12px;margin-bottom:4px}.case .subtitle{font-size:11px;color:#a5afba;line-height:1.5;min-height:33px}.case svg{display:block;width:100%;height:135px;overflow:visible}.graph-foot{font-size:11px;line-height:1.6;color:#a5afba;margin-top:18px}.explain{font-size:13px;line-height:1.6;color:var(--muted-foreground,#a5afba);margin:12px 0}details{margin-top:16px}summary{cursor:pointer;font-size:14px;font-weight:600}table{width:100%;border-collapse:collapse;margin-top:12px;font-size:12px;font-variant-numeric:tabular-nums}td,th{padding:8px 5px;text-align:right;border-bottom:1px solid var(--border,#303942)}td:first-child,th:first-child{text-align:left}.samples{margin-top:12px;font-size:12px;line-height:1.5}.small{font-size:12px;color:var(--muted-foreground,#a5afba)}.outputs{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:12px}.outputs figure{margin:0}.outputs img{width:100%;display:block;background:white}.outputs figcaption{margin:6px 0;font-size:12px}pre{white-space:pre-wrap;word-break:break-word;font-size:12px;max-height:240px;overflow:auto} @media(max-width:600px){.plots,.outputs{grid-template-columns:1fr}.case .subtitle{min-height:0}table{font-size:11px}}
</style></head><body><div class="controls"><button class="active" data-metric="time">Conversion time</button><button data-metric="memory">Peak memory</button><button class="active" data-engine="node">Node addon</button><button data-engine="core">Rust library</button></div>
<div class="chart-panel"><div class="plot-meta" id="plot-meta"></div><div class="plots" id="plots"></div><div class="graph-foot">PROVENANCE_LABEL<br>This chart has identical decoded pixels but different PNG encoding. Each panel has its own linear scale.</div></div>
<div class="explain">Same saved inputs and options. Timings exclude imports and file I/O. Memory includes the whole process. The Node addon uses TypeScript for JavaScript plugins. Each graph panel has its own linear scale.</div>
<details><summary>Inspect all measured medians</summary><table><thead><tr><th>Workload</th><th>TypeScript ms</th><th>Rust Node ms</th><th>Rust core ms</th></tr></thead><tbody>ROWS</tbody></table><div class="small">Two warmups, then 12 calls per engine and case. Whiskers show the full sample range.</div></details>
<details><summary>Inspect the actual chart images</summary><div class="outputs"><figure><img src="CHART_TS" alt="Chart PNG extracted from TypeScript DOCX"><figcaption>TypeScript 4.0.1 · extracted DOCX image</figcaption></figure><figure><img src="CHART_RUST" alt="Chart PNG extracted from Rust DOCX"><figcaption>Rust Node addon · extracted DOCX image</figcaption></figure></div><p class="small">Both are 1200 × 800, with identical decoded pixels. PNG encoding differs; the chart speedup includes encoding.</p></details>
<details><summary>What the output checks establish</summary><div class="samples">CHECK_COUNT_LABEL checks passed. All ZIP entries and XML parts parse. The prose and list counts match. Both highlighted outputs preserve all 640 functions and contain token colors. XML_CLAIM_LABEL<br><br>Core highlighting uses Syntect; Node uses lowlight callbacks. Text checks normalize nonbreaking spaces when comparing core output. The prose benchmark uses shared ++underline++ syntax; raw HTML behavior is recorded in the separate probe. These checks do not establish identical styling, pagination or Word rendering.</div></details>
<details><summary>Raw samples and memory readings</summary><pre id="raw"></pre></details>
<script>const raw=RAWDATA; const names=NAMES; let metric='time',engine='node';
function n(v){return metric==='memory'||v>=10?v.toFixed(1):v.toFixed(2)}
function render(){const key='rust-'+engine,unit=metric==='time'?'ms':'MiB';document.querySelectorAll('[data-metric]').forEach(b=>b.classList.toggle('active',b.dataset.metric===metric));document.querySelectorAll('[data-engine]').forEach(b=>b.classList.toggle('active',b.dataset.engine===engine));
document.getElementById('plot-meta').textContent=metric==='time'?'Median conversion time in ms. Two warmups, then 12 calls. Whiskers show min–max. Lower is better.':'Peak process memory in MiB. Separate fresh process, one conversion, no warmup. Runtime and imports included. Lower is better.';
document.getElementById('plots').innerHTML=raw.results.filter(r=>r.engines[key]).map(r=>{const a=r.engines.typescript,b=r.engines[key],values=metric==='time'?[a.median_ms,b.median_ms]:[a.cold.peak_rss_mib,b.cold.peak_rss_mib],max=(metric==='time'?Math.max(a.max_ms,b.max_ms):Math.max(...values))*1.35,scale=v=>76+v/max*216; const ratio=r.case==='plugin-fallback'?'Same TypeScript fallback':metric==='time'?(values[0]/values[1]).toFixed(1)+'× faster':Math.round((1-values[1]/values[0])*100)+'% lower peak';
let svg='<svg role="img" aria-label="'+names[r.case][0]+' measured comparison" viewBox="0 0 342 135">';[0,max/2,max].forEach(v=>{let x=scale(v);svg+='<line x1="'+x+'" y1="14" x2="'+x+'" y2="99" stroke="#29313b"/><text x="'+x+'" y="121" fill="#a5afba" font-size="10" text-anchor="middle">'+(v===0?'0':v.toFixed(0))+'</text>';});
[a,b].forEach((d,i)=>{let y=29+i*44,color=i?'#58ddb4':'#8994a2',v=values[i],label=metric==='time'?Math.max(v,d.max_ms):v;svg+='<text x="68" y="'+(y+4)+'" fill="#a5afba" font-size="11" text-anchor="end">'+(i?(engine==='node'?'Rust Node':'Rust core'):'TS 4.0.1')+'</text><rect x="76" y="'+(y-8)+'" width="'+(scale(v)-76)+'" height="16" fill="'+color+'"/><text x="'+(scale(label)+7)+'" y="'+(y+4)+'" fill="#f3f5f7" font-size="12" font-weight="600">'+n(v)+'</text>';if(metric==='time'){let x1=scale(d.min_ms),x2=scale(d.max_ms);svg+='<path d="M'+x1+' '+y+'H'+x2+' M'+x1+' '+(y-4)+'V'+(y+4)+' M'+x2+' '+(y-4)+'V'+(y+4)+'" stroke="#f3f5f7" fill="none"/>';}});svg+='</svg>';return '<section class="case"><h3>'+names[r.case][0]+'</h3><div class="ratio">'+ratio+'</div><div class="subtitle">'+names[r.case][1]+'</div>'+svg+'</section>';}).join('');}
document.querySelectorAll('button').forEach(b=>b.onclick=()=>{if(b.dataset.metric)metric=b.dataset.metric;if(b.dataset.engine)engine=b.dataset.engine;render();});document.getElementById('raw').textContent=JSON.stringify(raw,null,2);render();console.log('Benchmark rows:',raw.results.length,'output checks:',raw.verification.passed_checks.length);</script></body></html>'''
    page = page.replace("PROVENANCE_LABEL", html.escape(PROVENANCE).replace("\n", "<br>")).replace("XML_CLAIM_LABEL", html.escape(XML_CLAIM)).replace("CHECK_COUNT_LABEL", str(CHECK_COUNT)).replace("ROWS", "".join(rows)).replace("CHART_TS", chart_ts).replace("CHART_RUST", chart_rust).replace("RAWDATA", json.dumps(DATA).replace("</", "<\\/")).replace("NAMES",json.dumps(NAMES))
    (OUT / "comparison.html").write_text(page)


def main():
    for engine in ["rust-node", "rust-core"]:
        for metric in ["time", "memory"]:
            chart(engine, metric)
    exports()
    viewer()
    with zipfile.ZipFile(OUT / "comparison-files.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for path in OUT.rglob("*"):
            if path.is_file() and path.suffix not in [".zip", ".html"]:
                archive.write(path, path.relative_to(OUT))
    print("Saved four PNG/SVG graphs, self-contained HTML, CSV, methodology and comparison-files.zip")


if __name__ == "__main__":
    main()
