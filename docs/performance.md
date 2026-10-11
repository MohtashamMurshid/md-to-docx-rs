# Performance measurements

These numbers come from the original TypeScript 4.0.1 review and the native Rust comparison supplied during the port review. They precede the compatibility fixes in this repository. No new measurements are claimed here.

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
