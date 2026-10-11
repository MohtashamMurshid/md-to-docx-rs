# Development and verification

The workspace separates the native library, CLI and Node binding. `rust-toolchain.toml`, `Cargo.lock` and both npm lockfiles pin the build and test dependencies.

## Full checks

Rust, Node.js 20.9 or newer and Python 3 are required. The existing parity projector uses Python's ZIP reader.

```sh
./scripts/validate.sh
```

The script installs the reference oracle, checks formatting and all-target Clippy with warnings denied, runs workspace tests, builds the Node addon and runs its integration tests and TypeScript consumer check. CI runs the same script and verifies benchmark output semantics.

Individual checks:

```sh
npm ci --prefix crates/md-to-docx/tests/oracle
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
npm ci --prefix crates/md-to-docx-napi
npm run build --prefix crates/md-to-docx-napi
npm test --prefix crates/md-to-docx-napi
npm run test:types --prefix crates/md-to-docx-napi
```

## Reference oracle

`crates/md-to-docx/tests/oracle/run.mjs` imports pinned `@mohtasham/md-to-docx` 4.0.1, `docx` 9.5.0 and JSZip 3.10.1 from its own npm installation. The oracle keeps the 4.0.1 application version while overriding Sharp and KaTeX to patched versions. The fixture is checked into `tests/oracle/fixtures/`. Cargo alone does not install npm dependencies; run the install command above or the validation script first.

The fixture originates from the original MIT-licensed project. Tests inspect selected document properties, relationships, styles and media. They do not compare complete ZIP byte streams or render documents in Word.

## Node checks

Integration tests cover output types, reference and patch families, error context, cancellation, event-loop responsiveness, callback images/charts/Mermaid, concurrent asynchronous image resolvers and the JavaScript plugin/text-replacement compatibility path.

The package's browser condition forwards to the original browser entry point. This does not turn the addon into WebAssembly.

## Reproducible native benchmark

```sh
cargo run --locked --release -p md-to-docx --example benchmark
cargo run --locked --release -p md-to-docx --example benchmark -- --fixtures /tmp/md-fixtures
cargo run --locked --release -p md-to-docx --example benchmark -- --memory --case prose
```

The harness generates prose, ordered lists, plain and highlighted TypeScript, and a chart. It reports warmup, timed samples, medians, ranges, package sizes and uncompressed part sizes as JSON. It checks that highlighting creates colored runs and the chart has the requested dimensions. Fixture export includes the inputs and options so another implementation can use them.

These are new generated fixtures, explicitly marked as different from the historical inputs in [performance.md](performance.md). Do not compare their timings with the old tables or reuse old speedup ratios. Run both implementations against the exported fixtures on the same host.

Memory mode runs one conversion, retains the bytes and reads Linux `/proc/self/status` before unpacking. Other platforms report null for these Linux measurements. Run a fresh process per case. Keep native and Node measurements separate, and distinguish RSS at return from process high-water.

## Verification after the compatibility fixes

The full validation script passed locally: 355 Rust tests (including 46 live TypeScript oracle tests), 19 Node integration tests, the TypeScript consumer check, formatting, and all-target Clippy with warnings denied. Both npm installations reported zero audit vulnerabilities.

A release benchmark smoke run passed all five output checks, including colored TypeScript and a 1200 × 800 chart. It used one measured iteration on macOS ARM, so it establishes harness operation rather than a new performance comparison. CI is configured to run these checks on Linux; its hosted result was not observed during this review.

No Word or LibreOffice visual comparison was performed. Browser tests checked export routing rather than DOM execution.
