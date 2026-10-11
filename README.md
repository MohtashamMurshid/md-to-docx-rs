# md-to-docx-rs

Rust port of [@mohtasham/md-to-docx](https://github.com/MohtashamMurshid/md-to-docx). It parses Markdown, writes WordprocessingML, and returns a ZIP-based `.docx` package.

The workspace contains a Rust library, a CLI, and a Node-API addon. It is still a port in progress. Check the [compatibility notes](docs/compatibility.md) before migrating an application that uses plugins, renderer callbacks, reference documents, or patches.

## Build

Install Rust through rustup. The repository pins its toolchain in `rust-toolchain.toml`, currently `1.99.0`.

```sh
cargo build --workspace --release
```

The CLI is `target/release/md-to-docx`. Cargo also builds the native Node addon for the current OS and architecture.

## Rust library

Use the library as a path dependency while the workspace packages remain unpublished.

```toml
[dependencies]
md-to-docx = { path = "path/to/md-to-docx-rs/crates/md-to-docx" }
```

```rust
use md_to_docx::convert_markdown;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = convert_markdown("# Report\n\nHello **Word**.")?;
    std::fs::write("report.docx", bytes)?;
    Ok(())
}
```

`convert_with_options` accepts `ConvertOptions`. `convert_with_reference` uses a reference package, and `patch_markdown` replaces named placeholders in an existing package. These Rust calls are synchronous and return `Result<Vec<u8>, MarkdownConversionError>`.

## CLI

```sh
target/release/md-to-docx report.md report.docx
target/release/md-to-docx report.md report.docx --options options.json
cat report.md | target/release/md-to-docx - report.docx
target/release/md-to-docx --batch markdown/ documents/
target/release/md-to-docx report.md report.docx --watch
target/release/md-to-docx report.md report.docx --reference template.docx
target/release/md-to-docx template.docx patched.docx --patches patches.json
```

A patch file maps placeholder names to Markdown strings. For example, `{"summary": "## Summary\n\nRevenue increased."}` replaces `{{summary}}`.

Use `--help` for flags and combinations. Binary stdout uses `-` as the output path; status messages then go to stderr. Batch mode preserves input directories, continues after individual failures, and returns a failing exit code if any conversion fails.

## Node addon

Requires Node.js 20.9 or newer.

Install the compatibility dependency, then build the native binary for the current OS and architecture:

```sh
npm ci --prefix crates/md-to-docx-napi
npm run build --prefix crates/md-to-docx-napi
```

```sh
node --input-type=module <<'JS'
import { writeFile } from "node:fs/promises";
import { convertMarkdownToBuffer } from "./crates/md-to-docx-napi/index.js";

const bytes = await convertMarkdownToBuffer("# Report\n\nHello **Word**.");
await writeFile("report.docx", bytes);
JS
```

`convertMarkdownToDocx` returns a `Blob`; the ArrayBuffer and Buffer variants return their named types. Native conversions run off the event loop. JavaScript image, chart and Mermaid callbacks are supported by the native bridge.

Existing JavaScript plugins and other APIs that require remark or `docx` objects use the pinned TypeScript compatibility path. The browser conditional export also uses the original browser implementation. These paths are not native Rust and are not covered by the native performance figures. See [compatibility](docs/compatibility.md) for dispatch rules and remaining differences.

## Performance and checks

The original native release measurements were 46.07 ms for 3,200 paragraphs and 22.26 ms for 1,200 ordered lists. The corresponding TypeScript 4.0.1 medians were 1,249.75 ms and 1,749.40 ms. These are historical measurements before the compatibility fixes. They do not measure the Node addon.

Read [performance](docs/performance.md) for inputs, memory measurements, exclusions, and encoder differences. Read [development](docs/development.md) for tests and the TypeScript oracle prerequisites.
