# Development and verification

The workspace separates the native library, CLI and Node binding. `rust-toolchain.toml`, `Cargo.lock` and both npm lockfiles pin the build and test dependencies.

## Full checks

Rust and Python 3 are required. Node supports `^20.10.0 || >=22.0.0`. The existing parity projector uses Python's ZIP reader.

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

Fixture version 2 uses `++underline++`, which both implementations support. Older benchmark runs recorded a raw `<u>` difference; the parser now preserves those tags as literal text like TypeScript. The fixture version is recorded in the benchmark JSON.

Memory mode runs one conversion, retains the bytes and reads Linux `/proc/self/status` before unpacking. Other platforms report null for these Linux measurements. Run a fresh process per case. Keep native and Node measurements separate, and distinguish RSS at return from process high-water.

## Same-input TypeScript comparison

The comparison scripts run the original TypeScript 4.0.1 package, the native Node addon and the Rust library against the same saved fixtures. Pillow is required for output image inspection. The current memory runner uses macOS `/usr/bin/time -l`; do not run it on Linux and interpret its units as macOS bytes.

```sh
npm ci --prefix crates/md-to-docx-napi
npm run build --prefix crates/md-to-docx-napi
cargo build --locked --release -p md-to-docx --example benchmark --example compare_fixture
target/release/examples/benchmark --runs 1 --fixtures output/current-comparison/fixtures
python3 scripts/benchmark/compare.py output/current-comparison
python3 scripts/benchmark/validate_outputs.py output/current-comparison
```

Use a Python environment with Matplotlib and Pillow to render the graphs:

```sh
python3 scripts/benchmark/visualize.py output/current-comparison
```

Outputs include every timed sample, fresh-process memory logs, package versions, input hashes, individual DOCX files, CSV data, PNG/SVG graphs and a self-contained HTML viewer. The runner records two warmups and 12 measured calls per engine and case, plus a no-op JavaScript plugin case that checks the TypeScript fallback. An additional small fixture checks raw HTML underline handling.

Verification checks ZIP integrity, XML parsing, content counts, token colors, image dimensions, exact chart pixels, shared prose formatting counts and matching native Node/core document XML for workloads without highlighting. Historical benchmark text comparison normalizes highlighted-code NBSP substitutions. The full-package differential check below compares those characters directly. These checks do not replace visual verification in Word.

## Full-package differential corpus

The 1,754-case corpus includes 652 CommonMark examples, 677 GFM examples, feature and invalid-option inputs, reference/patch documents, and 150 seeded mixed documents. Each engine runs in a restartable subprocess with a 256 MiB JavaScript heap and a ten-second per-case timeout. Process failures are recorded in the index and do not abort the remaining corpus. Native memory is not included in the JavaScript heap cap.

```sh
npm run build --prefix crates/md-to-docx-napi
node scripts/parity/generate.mjs output/parity-final
python3 -I scripts/parity/verify.py output/parity-final
```

Pillow is required for decoded PNG verification. The verifier permits only the reviewed differences in `scripts/parity/known-differences.json` and fails on any additional difference. A successful verifier run does not mean every case is identical.

For strict differences, individual cases and the comparator's own tests:

```sh
python3 -I scripts/parity/compare.py output/parity-final
python3 -I scripts/parity/compare.py output/parity-final '^y-'
PARITY_MEDIA_MODE=pixels python3 -I scripts/parity/compare.py output/parity-final
python3 -I -m unittest discover -s scripts/parity -p 'test_*.py'
```

`compare.py` returns exit status 1 when it finds any difference, including a reviewed one. Set `PARITY_TRACE=1` when generating to log each case and engine. The generator also accepts a case-name regex as its second argument after the output directory.

## Verification on 2026-10-17

The final checks passed locally on macOS ARM with Node 24.18.0:

- 371 Rust tests, including ten expanded-corpus regressions and the 46-test reference/patch suite with a live TypeScript oracle.
- 24 Node tests and the TypeScript consumer check.
- Workspace formatting, all-target Clippy with warnings denied, and `git diff --check`.
- Six comparator tests covering spaces, namespace identity, prefix aliases, explicit timestamps, error classes and typed error context.
- Full-package parity of 1,745/1,754 with media bytes, or 1,748/1,754 with decoded PNG pixels. The verifier found zero unexpected differences.

The recorded counts, environment and corpus hashes are in [the parity report](parity/2026-10-17.json).

The empty-pattern process abort belongs to the TypeScript oracle. Native Rust completes that input. The worker isolation records the abort and continues to the next case.

Six DOCX pairs were exported with Word 16.91, with identical text and rasterized pixels on all ten pages per engine. The Neetprep trial passed a production build, a browser download through Rust, and a second browser download through its TypeScript rollback route. The chat response was a local browser fixture, so the check used no model API. A fresh benchmark comparison and its limits are recorded in [performance](performance.md). Hosted Linux/macOS/Windows jobs are configured but were not run in this session. LibreOffice was not tested.

## Release candidate and consumer trial

Build the addon before packing. The candidate includes a binary for the current OS and architecture and the exact compatibility dependency tree. It stays private and is not published by this command.

```sh
npm run build --prefix crates/md-to-docx-napi
node scripts/release/pack.mjs 0.1.0-rc.4
```

Install the resulting tarball into a consuming application, then run `node scripts/release/smoke.mjs /absolute/path/to/application`. This verifies dependency versions, output families, templates, plugins, highlighting, images, typed errors and cancellation using the installed package.

For saved Playground examples and Word rendering on macOS:

```sh
node scripts/rollout/documents.mjs /absolute/path/to/markdown-docx-playground/src/pages/Index.tsx output/rollout-documents
python3 scripts/rollout/render-word.py output/rollout-documents
```

The reader check requires installed Microsoft Word, Poppler and Pillow. It writes PDFs, full-page PNGs, contact sheets, input hashes and a JSON report under the chosen output directory. It serializes Word exports across processes. Review the full-page PNGs after running it. See [the migration trial](migration-trial.md) for the selected application, source locations, switches and rollback evidence.
