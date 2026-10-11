# Compatibility with TypeScript 4.0.1

This project implements conversion in Rust. Matching function names and producing a valid `.docx` do not establish full compatibility with the TypeScript library.

## Implemented document paths

| Area | Rust implementation | Qualification |
| --- | --- | --- |
| Paragraphs, headings, inline formatting, links | Implemented | XML and ZIP bytes differ from the TypeScript output. |
| Ordered and unordered lists | Implemented | Numbering output retains separate sequences and nine levels per definition. |
| Tables, including spans and nesting | Implemented | Projection tests inspect supported structures. |
| Footnotes, math, captions and cross-references | Implemented | TeX rendering supports a subset; Word field updates depend on the reader. |
| Sections, headers, footers, TOC and styles | Implemented | Field TOCs may need an update in Word. |
| Images and built-in charts | Implemented | Image loading has limits; remote loading is opt-in. Chart pixels and PNG encoding need not match the original. |
| Syntax highlighting | Syntect-based | Grammar coverage, span boundaries and colors differ from lowlight. See the historical TypeScript gap below. |
| Reference document generation and patching | Implemented | The live TypeScript oracle needs external dependencies and fixtures. Rendering compatibility requires additional checks. |
| CLI batch and watch | Implemented | Check option routing and asset paths before migration. |
| Rust plugins | Callback types exist | AST and block hooks require the fixes described below. |

The table describes implementation coverage, not a guarantee that every TypeScript option has identical behavior.

## Gaps found before the compatibility fixes

The original review found these issues. This list is a record of the starting point; update each item after integration and verification.

- The Node wrapper returned `Buffer` for the DOCX and ArrayBuffer variants. The public TypeScript API distinguishes `Blob`, `ArrayBuffer` and `Buffer`.
- Native Node conversion ran on the JavaScript thread. An `async` wrapper alone did not move CPU work off the event loop.
- The CLI computed `--base-dir` but did not apply it during conversion.
- The default Syntect syntax set had no TypeScript grammar. Both `ts` and `typescript` fell back to plain code, even with highlighting enabled.
- A Rust AST transform callback took no AST argument. It could run but could not edit the document through that callback API.
- CLI and Node option parsing duplicated substantial code.
- Reference and patch oracle tests relied on a TypeScript repository layout and dependencies absent from this checkout.

## Differences that remain relevant when migrating

The JavaScript wrapper serializes options as JSON and rejects functions. TypeScript plugins, image resolvers, chart renderers and Mermaid renderers supplied as JavaScript callbacks therefore cannot cross this bridge. Native Rust callback support does not make JavaScript callbacks available.

The addon is for Node. Browser imports and browser download behavior are not supplied by the native crate. A browser application needs a separate integration.

Rust highlighting uses Syntect; the original library uses lowlight. Even once both recognize TypeScript, token segmentation and generated OOXML can differ. Re-run highlighted benchmarks after grammar changes.

Reference packages and patching need special attention because they edit existing XML, relationships and media. Passing projection tests establishes the fields those tests inspect. It does not establish identical pagination, font substitution, chart appearance or rendering in Word and LibreOffice.

Before switching an application, compare representative documents with its actual options, inspect output in its target Word reader, and test return types and callback dependencies at the JavaScript call sites.
