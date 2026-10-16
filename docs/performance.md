# Performance measurements

The current comparison below measures the code after the compatibility fixes. The older Linux measurements remain in the later sections for reference; their fixtures and host differ.

## Current same-input comparison, 17 October 2026

Apple M3, eight logical CPUs, 16 GiB RAM, macOS ARM64, Node v24.18.0 and rustc 1.99.0 release. These measurements include the uncommitted compatibility and error-contract changes on base commit `582e9a8`. The benchmark records the dirty working-tree state. The 4.0.1 compatibility package uses docx 9.5.0, lowlight 3.3.0, Sharp 0.35.5 and KaTeX 0.18.2.

Each engine receives the same saved Markdown and options. A fresh process performs two warmups followed by 12 measured conversions. Engines run sequentially in rotated order. Medians exclude imports and file I/O. Engines are sampled in batches within each workload, so this run does not control for thermal or clock drift between those batches. Node syntax highlighting uses lowlight through the native callback bridge. Core Rust uses Syntect, so its highlighting time measures a different tokenizer.

| Workload | TypeScript ms | Rust Node ms | Rust core ms | TS / Rust Node |
| --- | ---: | ---: | ---: | ---: |
| prose | 696.95 | 55.83 | 55.60 | 12.5x |
| lists | 919.33 | 24.92 | 25.09 | 36.9x |
| plain-code | 22.35 | 2.11 | 2.04 | 10.6x |
| highlighted-ts | 173.64 | 31.40 | 81.29 | 5.5x |
| chart | 253.18 | 2.16 | 2.07 | 117.3x |
| plugin-fallback | 46.11 | 42.05 | n/a | Same TS implementation |

The no-op JavaScript plugin uses the original TypeScript converter. Its overlapping timing ranges establish no native speedup. The chart has identical decoded pixels but uses a different PNG encoder. These are warmed, single-host conversion measurements, not startup or HTTP latency.

Peak memory comes from a separate fresh process converting once without warmup. macOS `/usr/bin/time -l` includes runtime and imports in the process high-water.

| Workload | TypeScript peak MiB | Rust Node peak MiB | Rust core peak MiB |
| --- | ---: | ---: | ---: |
| prose | 540.7 | 132.6 | 50.9 |
| lists | 509.4 | 102.7 | 24.2 |
| plain-code | 131.4 | 85.7 | 5.9 |
| highlighted-ts | 331.7 | 132.3 | 35.9 |
| chart | 263.5 | 88.7 | 9.0 |
| plugin-fallback | 168.1 | 158.2 | n/a |

All 35 output checks passed, including ZIP/XML integrity, content counts, actual colored highlighting, chart dimensions and exact decoded chart pixels. Native Node highlighting matches the TypeScript text, run count and color set. Core highlighting remains a different implementation. Literal inline HTML is now preserved in both converters.

Raw samples, fixture hashes, package versions, import times and first-conversion timings are in [the recorded results](benchmarks/2026-10-17.json). Reproduce with `scripts/benchmark/compare.py output/current-comparison` and `scripts/benchmark/validate_outputs.py output/current-comparison`. Word checks on smaller consumer documents are recorded separately in [the migration trial](migration-trial.md).

## Earlier macOS comparison, 12 October 2026

Apple M3, eight logical CPUs, 16 GiB RAM, macOS ARM, Node v24.18.0 and rustc 1.99.0 release. The conversion code is commit `582e9a8`. TypeScript is `@mohtasham/md-to-docx` 4.0.1 with `docx` 9.5.0, using the same dependency installation as the addon's compatibility path. Patched Sharp and KaTeX overrides are installed; these workloads do not exercise them.

Each engine receives the same saved Markdown and options. A fresh process performs two warmups and 12 timed conversions. Engines run sequentially with rotated order. Medians exclude imports, input/output I/O and copying the returned Blob. TypeScript and the addon stop when `convertMarkdownToDocx` returns a Blob; core Rust stops when `convert_with_options` returns its bytes. There is no forced GC between calls.

| Workload | Input characters | TypeScript ms | Rust Node ms | Rust core ms | TS / Rust Node |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3,200 mixed-inline paragraphs | 384,808 | 671.49 | 41.13 | 40.90 | 16.3x |
| 1,200 ordered lists | 94,888 | 900.26 | 22.47 | 22.52 | 40.1x |
| Eight plain TypeScript blocks, 640 functions | 55,606 | 22.21 | 1.99 | 1.99 | 11.2x |
| Same code with real highlighting | 55,606 | 176.23 | 83.97 | 83.58 | 2.1x |
| Line chart, 1200 by 800 | 145 | 255.67 | 2.01 | 1.99 | 127.2x |
| No-op JavaScript AST plugin, 200 paragraphs | 23,808 | 45.80 | 42.90 | n/a | Same TS implementation |

The plugin timing ranges overlap. It runs the original TypeScript implementation through the compatibility path and establishes no native speedup. The chart has matching decoded RGBA pixels, with different PNG encoders. Highlighting compares Syntect with lowlight, including their different tokenization and run counts.

Peak memory was measured separately in a fresh process with one conversion and no warmup. macOS `/usr/bin/time -l` measures the entire process high-water, including imports and runtime; ZIP inspection happens in another process. These are MiB, not decimal MB or RSS at return.

| Workload | TypeScript peak MiB | Rust Node peak MiB | Rust core peak MiB |
| --- | ---: | ---: | ---: |
| Prose | 558.5 | 98.9 | 48.0 |
| Lists | 508.8 | 72.5 | 23.8 |
| Plain code | 131.2 | 54.0 | 5.9 |
| Highlighted code | 330.5 | 84.2 | 37.2 |
| Chart | 263.3 | 56.6 | 8.7 |

All 35 output checks passed: ZIP integrity, parsing all XML parts, expected prose/list/function counts, actual highlighting, chart dimensions and exact decoded chart pixels. Core Rust and native Node document XML match exactly for the five native workloads. Concatenated Word text matches TypeScript after normalizing highlighted code's nonbreaking spaces. This does not establish identical formatting, numbering behavior, wrapping or pagination.

The comparison exposed two differences. TypeScript emits `<u>` tags as literal text while Rust removes the tags and keeps plain text; the prose measurements were rerun with shared `++underline++` syntax. TypeScript changes some highlighted-code spaces to NBSP while Rust keeps ordinary spaces. This line chart has identical decoded pixels, while its PNG encoding differs. Word/LibreOffice rendering, browser DOM execution, remote images and user renderer callbacks were not benchmarked.

The fixture generator is version 2. It replaces the earlier generated HTML underline with shared underline syntax. These fixtures still differ from the original Linux inputs. Run both implementations against the new fixtures rather than pairing these numbers with the old baseline.

The [recorded results](benchmarks/2026-10-12.json) contain raw samples, input hashes, package versions and output statistics. Graphs, individual DOCX outputs and the methodology are in `output/current-comparison/`. Reproduction uses `scripts/benchmark/compare.py`, `validate_outputs.py` and `visualize.py`, documented in [development](development.md#same-input-typescript-comparison).

## Historical Linux comparison

## Method

Both reports used Linux x86_64, four Intel Xeon vCPUs and 16,014 MB RAM. TypeScript used Node.js 22.14.0. Rust used rustc 1.99.0 and a Cargo release build. The TypeScript report locked `docx` 9.5.0, `jszip` 3.10.1, `pako` 1.0.11, `remark` 15.0.1, `unified` 11.0.5, `lowlight` 3.3.0 and `sharp` 0.34.5.

TypeScript timed `convertMarkdownToDocx` until its `Blob` returned. Rust timed `convert_markdown` or `convert_with_options` until the `Vec<u8>` returned. The Rust run did not use the Node addon. Each process ran one warmup conversion before timing the same fixture repeatedly. The table reports medians, with the two middle samples averaged for an even run count.

The benchmark harnesses lived outside the repositories. Their source is not committed here, so matching character counts alone is insufficient to reproduce the exact fixtures. Preserve the original strings and option values in a committed harness before treating these as reproducible release benchmarks.

## Conversion time

| Input | Runs | Input characters | TypeScript median, ms | Rust median, ms | Ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3,200 mixed-inline paragraphs | 4 | 340,008 | 1,249.75 | 46.07 | 27.1x |
| 1,200 short ordered lists | 3 | 69,958 | 1,749.40 | 22.26 | 78.6x |
| Eight plain TypeScript blocks, 80 functions each | 6 | 81,214 | 119.85 | 6.71 | 17.9x |
| Line chart, 1200 by 800 | 3 | 253 | 414.36 | 4.13 | 100.3x |

Each prose paragraph includes bold, italic, underline, strike and an HTTPS link. Each list block has three items, with an indented ordered item inside the second. The chart is a `chart` fence with width and height in JSON; chart rendering is enabled and no asset is fetched.

The chart ratio includes different PNG encoders. It is an end-to-end result for those implementations, not an isolated comparison of languages or equivalent checksum work.

## Output size

| Input | TypeScript DOCX bytes | Rust DOCX bytes |
| --- | ---: | ---: |
| 3,200 paragraphs | 172,229 | 52,174 |
| 1,200 ordered lists | 54,054 | 47,126 |
| Eight plain TypeScript blocks | 25,803 | 20,784 |
| Line chart, 1200 by 800 | 29,558 | 17,682 |

Smaller ZIP output does not mean less document content. The prose Rust `word/document.xml` was 6,059,344 bytes, similar to scaling the TypeScript 800-paragraph part to the larger input. Rust deduplicated hyperlinks to 25 relationships for distinct URLs; its relationships part was 4,834 bytes.

The list document parts were 1,604,208 bytes in Rust and 1,606,319 bytes in TypeScript. Numbering parts were 2,142,393 and 2,380,242 bytes. Rust still emitted 1,202 abstract numbering definitions and 10,818 level entries. The measured gain did not depend on removing that nine-level numbering structure.

## Memory

Memory came from separate fresh processes converting once and retaining the output bytes. Rust read `VmRSS` and `VmHWM` from `/proc/self/status` before unzipping the result. Rust values are kibibytes divided by 1024. TypeScript figures are the original review's process RSS at return and include Node.

| Input | TypeScript RSS at return, MB | Rust RSS at return, MB | Rust high-water, MB |
| --- | ---: | ---: | ---: |
| 3,200 paragraphs | 514 | 21.9 | 39.0 |
| 1,200 ordered lists | 419 | 14.1 | 23.9 |
| Eight plain TypeScript blocks | Not recorded | 6.9 | 11.2 |
| Line chart, 1200 by 800 | Not recorded | 7.1 | 9.4 |

RSS at return is not peak memory. Use the high-water column when discussing Rust peak usage. There is no corresponding TypeScript memory figure for plain code or the chart in the supplied review.

## Why highlighted TypeScript was excluded

The original highlighted TypeScript run took 513.45 ms in TypeScript and 7.15 ms in Rust. That pair is not comparable. Rust's default Syntect set did not contain a TypeScript grammar. Requesting highlighting changed the code-block border but left the code uncolored, with `word/document.xml` unchanged at 1,412,054 bytes.

TypeScript used lowlight and produced 5,012,309 bytes of document XML. Changing the Rust fence to `js` enabled actual tokenization and took 121.08 ms over six measured runs, with 8,221,654 bytes of document XML. The different language and tokenizer make that a separate result, not a replacement for the missing TypeScript comparison.

Grammar fixes invalidate the old 7.15 ms result as a measure of current highlighted conversion. New runs must check that output contains colored spans and preserves the source text.

## What changed in the implementation

Rust writes OOXML directly rather than creating the TypeScript `docx` object tree. The original list bottleneck was `docx`'s repeated whole-document regex replacement for each concrete numbering ID. Direct numbering output avoids that replacement loop.

The Rust chart path uses `image` to deflate PNG scanlines. Its 1200 by 800 PNG was 45,679 bytes. The TypeScript rasterizer wrote stored, uncompressed IDAT data and used a bit-at-a-time CRC32 loop, which accounted for 31.4 percent of its chart CPU profile. Both end-to-end figures include their encoder choices.

Highlighting creates more runs and XML in both implementations. Its performance needs a genuine highlighted workload after grammar changes.

## Limits of the comparison

These are warmed medians on one host. They do not measure Node addon overhead, event-loop responsiveness, browser use, remote image fetching or every supported option. Different ZIP bytes and plausible XML sizes do not prove identical output. Retain correctness checks alongside future benchmark runs and visually inspect representative documents.

## Changes after these measurements

The compatibility work added real TypeScript/TSX grammars, wired image settings through all conversion paths, and implemented mutable native plugin transforms. The old uncolored TypeScript result no longer describes current behavior. Reference XML operations now reuse fixed regexes and bound the cache for internal generated patterns. Proofing language and RTL properties are applied before ZIP creation, avoiding a second unzip/repack for those options.

Node conversions now run off the event loop; image/chart/Mermaid callbacks can call JavaScript. Existing JavaScript plugins, warning hooks and function/RegExp text replacements use the TypeScript compatibility path. Browser exports use that implementation too. Measure these paths separately; native speedups do not apply to them.

The committed [benchmark harness](development.md#reproducible-native-benchmark) uses new generated fixtures and verifies actual colored TypeScript runs and chart dimensions. Its inputs differ from the historical fixtures, so fresh results require a new same-input TypeScript comparison. No new speedup ratios are claimed here.


## Node highlighting after the parity review

The native Node path now calls the pinned lowlight tokenizer through the JavaScript callback bridge. The Rust library and CLI still use Syntect. The parity fixes also changed package defaults, run splitting and parser behavior. The current same-input measurements above cover this implementation. Historical timing ratios describe earlier code and fixtures.
