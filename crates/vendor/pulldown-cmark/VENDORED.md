# Vendored pulldown-cmark 0.13.4

This copy comes from the crates.io source of `pulldown-cmark` 0.13.4. See
`LICENSE` for its MIT license. Binary targets, examples, tests and the build
script were removed. The workspace selects it through `[patch.crates-io]`.

The local parser changes match the remark extensions used by TypeScript 4.0.1:

- Single `~` follows emphasis flanking rules, including intraword strike.
- Inline math pairs dollar runs of the same length. Runs of other lengths
  and backslashes inside math remain data. Surrounding whitespace and brace
  nesting do not prevent a dollar delimiter from matching.
- Footnote labels cannot contain whitespace. An invalid footnote definition
  cannot interrupt the preceding footnote paragraph.
- Emphasis consumes at most two delimiters before checking the rule of three
  again against the remaining lengths. This preserves literal delimiters in
  inputs such as `***bi****i*`.

The core crate handles standalone math fences and literal-autolink precedence
before feeding events into its document model. Tests in
`crates/md-to-docx/tests/expanded_parity.rs` cover these interactions. The full
CommonMark and GFM differential corpora exercise the vendored parser alongside
those changes. Keep these notes and regression results current when upgrading.
