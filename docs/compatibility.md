# Compatibility with TypeScript 4.0.1

The package uses Rust for ordinary Node conversions and a pinned TypeScript compatibility path for APIs that depend on JavaScript's remark AST or `docx` objects. Matching signatures does not promise identical pagination, chart pixels, or ZIP bytes.

## Which implementation runs

| Request | Implementation |
| --- | --- |
| Ordinary Node conversion, reference generation and patching | Native Rust, off the event loop |
| JavaScript image resolver, chart renderer and Mermaid renderer | Native conversion with callbacks dispatched to JavaScript |
| JavaScript plugins, function/RegExp text replacements, or warning callback | TypeScript 4.0.1 compatibility path |
| Styled patch objects and string/promise reference inputs | TypeScript compatibility path |
| `parseToDocxOptions` | TypeScript compatibility path, returning actual `docx` options |
| Browser conditional export | TypeScript browser implementation |
| Rust library and CLI | Native Rust; JavaScript functions cannot be loaded from CLI JSON |

The browser export is not a Rust WebAssembly build. Plugin-bearing conversions do not inherit the native Rust benchmark results. Inspect `crates/md-to-docx-napi/index.js` for the dispatch rules.

## Fixed during this review

- The Node DOCX, ArrayBuffer and Buffer families now return `Blob`, `ArrayBuffer` and Node `Buffer`, including reference and patch functions.
- Native Node work runs away from the event loop. Callback conversions use dedicated threads so callbacks awaiting filesystem work do not exhaust the libuv pool.
- Image configuration reaches document conversion, reference generation and patching. The CLI resolves local images from the input directory and honors `--base-dir`. Image budgets restart for each conversion and remain shared across sections within it.
- Bundled TypeScript and TSX grammars recognize `ts`, `typescript` and related aliases. Tests inspect colored Word runs, not just the highlighting option.
- Native Rust AST transforms receive a mutable `model::Document`. Custom block handlers consume `Block::Custom` nodes inserted by those transforms. This Rust model is distinct from JavaScript mdast; existing JavaScript plugins use the compatibility path.
- The CLI and Node binding use a shared JSON decoder.
- The reference/patch oracle has pinned npm dependencies and a checked-in fixture. It no longer needs an adjacent TypeScript checkout.
- Reference and patch calls apply their conversion settings rather than discarding them. Reference layout/header/footer flags are honored; patch limits also apply after plugin expansion.
- Strict image alt-text settings survive option decoding. Custom chart renderers receive the resolved dimensions, including configured limits.
- Internal regexes are cached, and accessibility metadata is written before ZIP packing instead of unpacking and recompressing it.

## Document coverage

| Area | Qualification |
| --- | --- |
| Paragraphs, headings, links, formatting and lists | Semantic projection tests; OOXML and ZIP bytes can differ. |
| Tables, spans and nested tables | Supported structures have regression tests. |
| Math, footnotes, captions and cross-references | Math supports a TeX subset; field updates depend on the document reader. |
| Sections, headers, footers, TOC and styles | Field TOCs may need updating in Word. |
| Images and charts | Bounded image loading; remote images opt in. Chart appearance and PNG encoding can differ. |
| Highlighting | Syntect instead of lowlight. Token boundaries and theme scope mappings can differ. |
| Reference generation and patching | Live oracle tests inspect selected XML and package structures. |

## Remaining limits

A native request cancelled through `AbortSignal` rejects its JavaScript promise and discards the result. An already-aborted request never starts. Cancelling a running request does not forcibly interrupt synchronous Rust computation or arbitrary user callbacks.

Rust calls are synchronous. Their trusted callback types operate on Rust values; the core crate cannot execute JavaScript plugins. The Node compatibility path preserves those JavaScript APIs at the cost of using the original implementation.

Browser consumers use the original browser implementation and its existing restrictions, including server-style remote image fetching. Native binaries are specific to OS and architecture; this repository builds one for the current machine rather than distributing prebuilt binaries for every target.

Projection tests do not establish identical pagination, font substitution, or rendering in Word and LibreOffice. Compare representative documents in the readers your application supports before migrating. No Word or LibreOffice visual comparison was performed during this review.
