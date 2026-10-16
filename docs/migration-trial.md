# Neetprep migration trial, 17 October 2026

Neetprep is the selected consumer. It has a real DOCX download action and a Next.js Node server where the native addon can run. The other local consumer, Markdown DOCX Playground, is a browser application and provides useful saved examples. Its browser export would continue to use TypeScript.

The trial is ready in `output/consumer-trial/neetprep`, branch `codex/rust-docx-trial`. It was created from Neetprep's HEAD plus its existing tracked working changes. The original checkout's diff is byte-for-byte unchanged. No production application or npm release was published.

## Sources and options

| Item | Location | How the trial uses it |
| --- | --- | --- |
| Consuming application | `~/Documents/Development/GitHub/neetprep` | Isolated copy under this repository's `output/consumer-trial/neetprep` |
| Existing export action | Neetprep `app/components/CodeComponent.tsx`, `handleDownloadDocx` | Joins assistant message contents with two newlines; calls the converter with no conversion options; downloads `code-assistant-conversation.docx` |
| Saved Markdown | `~/Documents/Development/GitHub/blog/markdown-docx-playground/src/pages/Index.tsx` | Extracts `defaultMarkdown`, `examples.report` and `examples.documentation` directly from the TypeScript AST |
| Playground options | Same `Index.tsx`, export handler | `documentType` and extracted heading `title`; the trial uses document mode for basic/API examples and report mode for the technical report |
| Reference DOCX | `crates/md-to-docx/tests/oracle/fixtures/reference-template.docx` | Reference generation and `body` placeholder replacement using the saved technical report |
| App templates | None in Neetprep's DOCX export action | The reference fixture is separate library coverage, not an invented app template |

The browser workflow used a clearly labeled local chat-response fixture with a heading, JavaScript fence and bullets. It exercised the real download button and server conversion without calling an AI API or using private conversations.

## Trial behavior and rollback

`NEXT_PUBLIC_NATIVE_DOCX=1` sends the existing download action to `/api/docx`. Otherwise it uses the existing browser converter. The route runs on Node, checks the request, returns the DOCX MIME type and filename, and sets `Cache-Control: no-store`.

`MD_TO_DOCX_ENGINE=rust` selects the native candidate. Any other value selects Neetprep's original TypeScript dependency. The server switch takes effect on restart. The browser flag takes effect when building. The defaults preserve the existing converter.

Start the isolated trial from this repository:

```sh
cd output/consumer-trial/neetprep
NEXT_PUBLIC_NATIVE_DOCX=1 MD_TO_DOCX_ENGINE=rust npm run dev -- --port 3187
```

For a server rollback, restart with `MD_TO_DOCX_ENGINE=typescript`. To restore direct browser conversion, set `NEXT_PUBLIC_NATIVE_DOCX=0` and rebuild. The original checkout needs no rollback because it was not changed.

The production build passed. The real browser button downloaded a 7,823-byte DOCX through Rust with HTTP 200, then a 9,866-byte DOCX through the TypeScript rollback with HTTP 200. Malformed JSON, missing/blank Markdown and the one-million-character limit returned the expected 400/413 responses. Evidence and a patch relative to the original working state are under `output/consumer-trial/`.

## The existing app uses v1

Neetprep currently installs `@mohtasham/md-to-docx` 1.0.0. This rewrite targets 4.0.1. A direct migration also upgrades conversion behavior.

Word shows visible differences on the actual browser fixture: v1 centers the heading and prints raw code fences; v4 and Rust left-align the heading, format the code block, change body sizing/spacing and add a page-number footer. Both occupy one page. They are not visually identical. The Rust browser download canonically matches TypeScript 4.0.1 on that exact input, so these differences belong to the version upgrade.

Keep this as an opt-in trial until those v1-to-v4 changes are acceptable. It is not a 100% drop-in migration for Neetprep's current dependency.

## Word and library validation

Six paired documents cover the three saved Playground examples, the repository README, reference generation and template patching. Microsoft Word 16.91 exported all twelve DOCX files. All ten pages per engine have identical pixels at a 1,600-pixel longest edge and identical PDF text. Every canonical package matches. The sample's unreachable remote image produces the same placeholder in both engines. Existing code-block pagination also matches.

The first run caught a multi-heading patch bookmark counter bug. The fix uses a shared heading counter rather than restarting for each distinct title. A new differential regression covers the report structure.

Full-page images, PDFs, contact sheets, DOCX hashes and validation results are in `output/rollout-documents-final/`. The separate comparison against Neetprep's v1 is in `output/consumer-trial/legacy-comparison/`. Word was tested on this host only; LibreOffice was not tested.

The complete corpus now has 1,754 cases: 1,745 canonical matches with PNG bytes, 1,748 with decoded pixels, and zero unexpected differences. All nine reviewed differences remain documented. Local checks passed with 371 Rust tests, 24 Node tests, six comparator tests, formatting, Clippy and consumer type checks. The full Node suite passed on 20.10, 22.14 and 24.18. Typed error context, cancellation reason identity and callback failures have dedicated coverage.

## Candidate and release limits

The local candidate is `output/release/md-to-docx-rs-0.1.0-rc.4.tgz`, for macOS ARM64. The manifest records SHA-256 and npm integrity. It includes the exact tested compatibility tree: TypeScript converter 4.0.1, docx 9.5.0, Sharp 0.35.5 and KaTeX 0.18.2. Bundling matters because a consuming application's package resolution can otherwise select different transitive versions.

Fresh-install smoke checks verify those versions, output families, reference/patch calls, a JavaScript plugin, highlighting, images, typed errors and cancellation on Node 20.10, 22.14 and 24.18. Node 20.9 fails Sharp's ESM syntax; the declared range is now `^20.10.0 || >=22.0.0`.

The [fresh benchmarks](performance.md) report 12.5x for prose, 36.9x for lists, 10.6x for plain code, 5.5x for highlighted code and 117.3x for the chart on this host. Plugin conversions use TypeScript and establish no native speedup. All 35 output checks passed.

Linux, macOS and Windows CI jobs now build, test, pack and install the candidate on Node 20.10, 22 and 24. Those hosted jobs have not run in this session. Produce and verify binaries on the deployment target before rolling this macOS trial into production. The candidate remains private and unpublished.
