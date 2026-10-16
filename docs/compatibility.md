# Compatibility with TypeScript 4.0.1

This is a close replacement for the Node API, with documented differences. It is not a proven 100% drop-in replacement. Ordinary Node conversions use Rust; APIs that need JavaScript's remark AST or `docx` objects use the pinned TypeScript package.

## Which implementation runs

| Request | Implementation |
| --- | --- |
| Ordinary Node conversion, reference generation and patching | Native Rust, off the event loop |
| JavaScript image resolver, chart renderer and Mermaid renderer | Native conversion with callbacks dispatched to JavaScript |
| Node syntax highlighting | Native conversion with lowlight tokenization dispatched to JavaScript |
| JavaScript plugins, function/RegExp text replacements, or warning callback | TypeScript 4.0.1 compatibility path |
| Styled patch objects and string/promise reference inputs | TypeScript compatibility path |
| `parseToDocxOptions` | TypeScript compatibility path, returning actual `docx` options |
| Browser conditional export | TypeScript browser implementation |
| Rust library and CLI | Native Rust, with Syntect highlighting and Rust callbacks |

The browser export is not WebAssembly. JavaScript plugins do not use the native conversion benchmark path. The dispatch rules are in `crates/md-to-docx-napi/index.js`.

## Measured package parity

On 2026-10-17, the differential corpus compared 1,756 inputs through TypeScript 4.0.1 and the native Node API.

| Corpus | Matching canonical packages |
| --- | ---: |
| CommonMark examples | 652 / 652 |
| GFM examples | 677 / 677 |
| Feature, validation, reference, patch and seeded random cases | 418 / 427 |
| Total with exact embedded media bytes | 1,747 / 1,756, or 99.49% |
| Total comparing decoded PNG RGBA pixels | 1,750 / 1,756, or 99.66% |

Comparison covers every package part. It resolves relationship IDs and media filenames, ignores XML property ordering where appropriate, and removes generated timestamps. Explicitly supplied timestamps, XML namespaces and text spaces remain significant. ZIP compression and entry ordering are not compared. Error cases compare success versus failure, error class, message and JSON context values, including number and boolean types. Dedicated contract tests cover 81 error and boundary inputs plus cancellation reason identity and asynchronous chart failures.

The nine strict differences are reviewed in `scripts/parity/known-differences.json`:

- Three cases contain differently encoded PNGs. SVG rasterization and all tested chart types have identical decoded pixels in these cases.
- Three invalid-chart cases contain different JSON parser diagnostics. TypeScript uses V8's version-dependent message; Rust uses its native diagnostic. Both reject the same definitions.
- An empty replacement search string makes TypeScript loop until its process exhausts the heap. Native Rust treats the empty pattern as a no-op.
- A footnote reference inside a heading makes TypeScript emit a `bookmarkUniqueNumericId` element containing a function string. Rust emits a footnote reference and bookmark.
- A bare `#` link makes TypeScript emit a dangling relationship ID. Rust emits an empty internal anchor.

The verifier checks decoded PNG equality for encoder exceptions and permits only the reviewed difference classes for the other cases. It reports zero unexpected differences. These numbers describe this corpus, not a percentage of all possible Markdown or API behavior.

## Behavior covered by the fixes

The native writer now matches default package parts, styles, numbering, settings, section properties, text-run boundaries, bookmarks, fields and resource relationships across the corpus. Long punctuation runs in autolinks are scanned once, protected URL tokens are restored by index, and indented display math retains its list container. Coverage includes rich tables and spans, headers and footers, TOC variants, metadata, captions, footnotes, image budgets, reference documents, patches and 150 seeded mixed documents.

The parser preserves literal inline HTML, code-span source text, GFM autolinks and task lists, and the `++underline++` extension. Its vendored pulldown-cmark changes cover intraword strike, dollar delimiter runs, footnote labels and partially consumed emphasis delimiters. See `crates/vendor/pulldown-cmark/VENDORED.md`.

Node highlighting uses the compatibility package's lowlight tokenizer through the callback bridge. Token boundaries, colors and nonbreaking-space substitutions match the tested TypeScript output. Rust and CLI highlighting use Syntect, so their token boundaries and colors can differ.

Reference and patch conversion retain conversion settings. Patches honor custom delimiters, preserve metadata, import required definitions, renumber bookmarks and request TOC field updates. Generated documents can be reused as references without duplicate relationship IDs.

The Node DOCX, ArrayBuffer and Buffer families return `Blob`, `ArrayBuffer` and Node `Buffer`, including reference and patch functions. Callback conversions use dedicated threads so callbacks can await filesystem work without exhausting the libuv pool.

## Remaining limits

The Rust model and callback types differ from JavaScript mdast. Existing JavaScript plugins use the compatibility path. Node errors preserve typed JSON context values. Rust callers retain the string-map accessor and can also read typed context through `context_json()`. This covers the tested contracts, not every possible user-thrown error.

Cancelling a native request rejects its JavaScript promise and discards the result. An already-aborted request never starts. Cancellation preserves the exact `signal.reason` object in `error.context.reason`. Cancellation does not forcibly interrupt synchronous Rust computation or arbitrary user callbacks.

Math supports the original renderer's TeX subset. Word may need to update TOC, caption and cross-reference fields. Six consumer/reference document pairs were exported by Microsoft Word 16.91. All ten pages per engine have identical rasterized pixels and text after the patch-bookmark fix. This checks those documents on this reader and host; LibreOffice, other fonts and other operating systems were not tested. See [the migration trial](migration-trial.md).

Browser consumers use the original browser implementation and its existing restrictions. Native binaries depend on OS and architecture; the local candidate is for macOS ARM64. CI now defines Linux, macOS and Windows checks on Node 20.10, 22 and 24, and the first hosted run passed the full Linux parity job and eight package jobs. Windows Node 20.10 exposed a wildcard test-discovery issue; the runner now passes explicit filenames. See [PR #1](https://github.com/MohtashamMurshid/md-to-docx-rs/pull/1) for current check status. Node 20.9 fails Sharp's ESM import syntax, so the package now declares `^20.10.0 || >=22.0.0`.

Historical benchmark differences involving stripped `<u>` tags and highlighted-code ordinary spaces were corrected. A fresh 12-sample comparison with output checks is recorded in [performance](performance.md).
