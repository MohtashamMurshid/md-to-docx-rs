# Development and verification

## Workspace

| Crate | Purpose |
| --- | --- |
| `md-to-docx` | Markdown parsing, typed options, OOXML output, image and chart handling, reference packages and patching |
| `md-to-docx-cli` | `md-to-docx` executable, JSON options, batch conversion and watch mode |
| `md-to-docx-napi` | Native Node addon and JavaScript wrapper |

The toolchain and dependency versions are pinned by `rust-toolchain.toml` and `Cargo.lock`.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins
```

The third command runs unit and binary tests. It does not run the integration parity tests. Run those separately once their prerequisites are available.

```sh
cargo test -p md-to-docx --tests
```

Some parity helpers use `python3` to unzip generated packages. Tests inspect selected XML structures and media; they do not compare entire ZIP byte streams or render documents in Word.

## TypeScript oracle

The slice-8 reference and patch tests run a live TypeScript oracle. In the original checkout layout, `slice8_oracle.mjs` imports `../../../../dist/index.js` relative to the test script and resolves `docx`, `jszip` and TypeScript reference fixtures outside this Rust workspace. A standalone checkout cannot satisfy that layout by running Cargo alone.

Before claiming reference or patch parity, provide the original TypeScript build, its locked npm dependencies and its `tests/fixtures/reference-template.docx`, or update the tests to use an explicit oracle path. Run the suite and record the actual result. Missing oracle dependencies are a test setup failure, not evidence of matching conversion behavior.

The first review reported 114 workspace unit tests passing, parity slices 1 through 7 passing, and eight slice-9 tests passing. Slice 8 had 35 failures because its JavaScript dependencies were unavailable. Those are historical results, not the current branch's test status.

## Node verification

Build the release addon before exercising the wrapper. Verify `Blob`, `ArrayBuffer` and `Buffer` contracts independently, including reference and patch functions. Test a conversion large enough to check that timers continue running while native work is in progress. Function-valued JavaScript options need explicit bridge support; JSON serialization cannot transport callbacks.

## Benchmark work still needed

Commit fixture generation and benchmark entry points, with the exact options beside each fixture. Record toolchain, host, warmup count, timed samples, output size and memory collection method. For native and Node runs, report separate results.

Check the output before comparing time. A highlighted fence must produce colored runs, lists must preserve start and nesting behavior, and charts must embed an image with the requested dimensions. Run memory measurements in fresh processes and distinguish RSS at return from process high-water.

The [performance notes](performance.md) preserve the original measurements and their limits. Future compatibility changes need fresh numbers rather than carrying those results forward.
