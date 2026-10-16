//! Slice 1 parity with `tests/rendering.test.ts`.
//!
//! Each test calls [`md_to_docx::convert_markdown`] and compares a projection of
//! the unzipped package: paragraph text in order, `Heading1`–`Heading6`, bold,
//! italic, underline, strike, code, link targets, table cell text and column
//! count, and list numbering start. ZIP bytes are not compared.
//!
//! Expected text comes from `@mohtasham/md-to-docx` `convertMarkdownToBuffer`
//! with default options. `convert_markdown` takes only a string, so Jest cases
//! that pass `style` (heading alignment, `heading6Size`, `fontFamily`,
//! `fontFamilly`, callout color overrides, `codeBlockSize`) cannot send those
//! options. Those tests still use the same Markdown and lock the default
//! projection.

#[path = "common/mod.rs"]
mod common;

use common::{Document, ListInfo, Paragraph, Run, Table};

fn convert(markdown: &str) -> Document {
    let bytes = md_to_docx::convert_markdown(markdown)
        .unwrap_or_else(|err| panic!("convert_markdown failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_markdown should return a zip package"
    );
    common::project_docx(&bytes)
}

#[track_caller]
fn paragraphs(doc: &Document) -> Vec<&Paragraph> {
    let paragraphs = doc.paragraphs();
    assert!(
        !paragraphs.is_empty(),
        "document has no paragraphs\n{}",
        doc.render()
    );
    paragraphs
}

#[track_caller]
fn assert_texts(doc: &Document, expected: &[&str]) {
    assert_eq!(
        doc.paragraph_texts().as_slice(),
        expected,
        "paragraph text\n{}",
        doc.render()
    );
}

#[track_caller]
fn run_named<'a>(paragraph: &'a Paragraph, text: &str) -> &'a Run {
    paragraph
        .runs
        .iter()
        .find(|run| run.text == text)
        .unwrap_or_else(|| {
            panic!(
                "missing run {text:?} in {paragraph:?}\nstyles={:?}",
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<Vec<_>>()
            )
        })
}

#[track_caller]
#[allow(clippy::too_many_arguments)] // Expected run properties form one fixture assertion.
fn assert_run(
    paragraph: &Paragraph,
    text: &str,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
    link: Option<&str>,
) {
    let run = run_named(paragraph, text);
    assert_eq!(run.bold, bold, "bold on {text:?}");
    assert_eq!(run.italic, italic, "italic on {text:?}");
    assert_eq!(run.underline, underline, "underline on {text:?}");
    assert_eq!(run.strike, strike, "strike on {text:?}");
    assert_eq!(run.code, code, "code on {text:?}");
    assert_eq!(run.link.as_deref(), link, "link on {text:?}");
}

#[track_caller]
fn assert_list(paragraph: &Paragraph, ordered: bool, start: u32, level: u32) -> &ListInfo {
    let list = paragraph
        .list
        .as_ref()
        .unwrap_or_else(|| panic!("expected a list on {:?}\n{paragraph:?}", paragraph.text));
    assert_eq!(list.ordered, ordered, "ordered on {:?}", paragraph.text);
    assert_eq!(list.start, start, "numbering start on {:?}", paragraph.text);
    assert_eq!(list.level, level, "list level on {:?}", paragraph.text);
    list
}

#[track_caller]
fn only_table(doc: &Document) -> &Table {
    let tables = doc.tables();
    assert_eq!(tables.len(), 1, "expected one table\n{}", doc.render());
    tables[0]
}

#[track_caller]
fn assert_dxa_width(table: &Table, twips: &str) {
    let width = table
        .width
        .as_deref()
        .unwrap_or_else(|| panic!("missing tblW\n{table:?}"));
    assert!(
        width.chars().all(|ch| ch.is_ascii_digit()),
        "table width {width} should be an integer, not a percent"
    );
    assert_eq!(width, twips);
    assert_eq!(table.width_type.as_deref(), Some("dxa"));
}

#[track_caller]
fn cell(table: &Table, row: usize, column: usize) -> &Paragraph {
    table
        .rows
        .get(row)
        .and_then(|cells| cells.get(column))
        .and_then(|cell| cell.paragraphs.first())
        .unwrap_or_else(|| panic!("missing cell {row},{column}\n{table:?}"))
}

// Jest: "applies level-specific heading alignments and falls back to headingAlignment"
//
// The Jest call also sets heading1Alignment CENTER, heading2Alignment RIGHT, and
// headingAlignment JUSTIFIED. convert_markdown cannot take those options, so this
// checks the default rendering of that Markdown: Heading1–Heading5 and the
// default half-point sizes 32, 28, 24, 20, 16.
#[test]
fn level_specific_heading_alignments() {
    let doc = convert(
        "# H1 centered
## H2 right
### H3 default-from-fallback
#### H4 default-from-fallback
##### H5 default-from-fallback",
    );
    let expected = [
        ("Heading1", "H1 centered", 32_u32),
        ("Heading2", "H2 right", 28),
        ("Heading3", "H3 default-from-fallback", 24),
        ("Heading4", "H4 default-from-fallback", 20),
        ("Heading5", "H5 default-from-fallback", 16),
    ];
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs.len(), expected.len(), "{}", doc.render());
    for (paragraph, (style, text, size)) in paragraphs.iter().zip(expected) {
        assert_eq!(paragraph.heading.as_deref(), Some(style), "{text}");
        assert_eq!(paragraph.text, text);
        assert_eq!(run_named(paragraph, text).size, Some(size), "{text}");
    }
}

// Jest: "renders H6 headings with level-specific styling"
//
// heading6Alignment RIGHT and heading6Size 18 are style options. The default
// H6 run is Heading6 at 12 half-points.
#[test]
fn h6_level_specific_styling() {
    let doc = convert("###### H6 custom");
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.heading.as_deref(), Some("Heading6"));
    assert_eq!(paragraph.text, "H6 custom");
    assert_eq!(run_named(paragraph, "H6 custom").size, Some(12));
}

// Jest: "renders heading inline formatting, links, and code from the model"
#[test]
fn heading_inline_formatting_links_and_code() {
    let doc = convert("# Head **bold** and [lnk](https://example.com/h) and `code`");
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.heading.as_deref(), Some("Heading1"));
    assert_eq!(paragraph.text, "Head bold and lnk and code");
    assert!(!paragraph.text.contains("[lnk]"));
    assert!(!paragraph.text.contains("`code`"));
    assert_run(paragraph, "bold", true, false, false, false, false, None);
    assert_run(
        paragraph,
        "lnk",
        false,
        false,
        true,
        false,
        false,
        Some("https://example.com/h"),
    );
    let code = run_named(paragraph, "code");
    assert!(code.code);
    assert_eq!(code.color.as_deref(), Some("444444"));
    assert!(!code.text.contains('`'));
}

// Jest: "applies paragraph and blockquote alignments"
//
// paragraphAlignment JUSTIFIED and blockquoteAlignment CENTER are style options.
// Default conversion still keeps the three paragraphs and italicizes the quote.
#[test]
fn paragraph_and_blockquote_alignment() {
    let doc = convert(
        "Left paragraph here.

> A blockquote that should be centered.

Another paragraph with justified alignment.",
    );
    assert_texts(
        &doc,
        &[
            "Left paragraph here.",
            "A blockquote that should be centered.",
            "Another paragraph with justified alignment.",
        ],
    );
    let quote = &paragraphs(&doc)[1];
    assert_run(
        quote,
        "A blockquote that should be centered.",
        false,
        true,
        false,
        false,
        false,
        None,
    );
}

// Jest: "preserves inline formatting, links, and code inside blockquotes"
#[test]
fn blockquote_inline_formatting() {
    let doc = convert("> Quote with **bold**, [lnk](https://example.com/q) and `code`.");
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.text, "Quote with bold, lnk and code.");
    assert_run(
        paragraph,
        "Quote with ",
        false,
        true,
        false,
        false,
        false,
        None,
    );
    assert_run(paragraph, "bold", true, true, false, false, false, None);
    assert_run(paragraph, ", ", false, true, false, false, false, None);
    assert_run(
        paragraph,
        "lnk",
        false,
        true,
        true,
        false,
        false,
        Some("https://example.com/q"),
    );
    assert_run(paragraph, " and ", false, true, false, false, false, None);
    assert_run(paragraph, "code", false, false, false, false, true, None);
    assert_run(paragraph, ".", false, true, false, false, false, None);
    assert!(!paragraph.text.contains("**bold**"));
    assert!(!paragraph.text.contains("[lnk]"));
    assert!(!paragraph.text.contains("`code`"));
}

// Jest: "renders multi-paragraph blockquotes as separate quoted paragraphs"
#[test]
fn multi_paragraph_blockquotes() {
    let doc = convert("> First quoted paragraph.\n>\n> Second quoted paragraph.");
    assert_texts(
        &doc,
        &["First quoted paragraph.", "Second quoted paragraph."],
    );
    for paragraph in paragraphs(&doc) {
        assert!(!paragraph.text.contains('\n'));
        assert!(paragraph.runs.iter().any(|run| run.italic));
    }
}

// Jest: "renders lists inside blockquotes instead of dropping them"
#[test]
fn lists_inside_blockquotes() {
    let doc = convert(
        "> Quoted intro
>
> - list item 1
> - list item 2",
    );
    assert_texts(&doc, &["Quoted intro", "list item 1", "list item 2"]);
    assert!(paragraphs(&doc)[0].runs.iter().any(|run| run.italic));
    assert_list(paragraphs(&doc)[1], false, 1, 0);
    assert_list(paragraphs(&doc)[2], false, 1, 0);
}

// Jest: "renders nested blockquotes with all nested text preserved"
#[test]
fn nested_blockquotes() {
    let doc = convert(
        "> Outer quote
>
> > Inner quote",
    );
    assert_texts(&doc, &["Outer quote", "Inner quote"]);
    assert_eq!(paragraphs(&doc).len(), 2);
    for paragraph in paragraphs(&doc) {
        assert!(paragraph
            .runs
            .iter()
            .all(|run| run.italic || run.text.is_empty()));
    }
}

// Jest: "starts quoted lists at quote-local list depth"
#[test]
fn quoted_lists_at_local_depth() {
    let doc = convert(
        "- outer
  - > - quoted item",
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(
        paragraphs
            .iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect::<Vec<_>>(),
        vec!["outer", "", "quoted item"]
    );
    assert_list(paragraphs[0], false, 1, 0);
    assert_list(paragraphs[1], false, 1, 1);
    assert_list(paragraphs[2], false, 1, 0);
}

// Jest: "renders GitHub-style note callouts with variant styling"
#[test]
fn github_callout_note() {
    assert_github_callout("NOTE", "Note");
}

// Jest: "renders GitHub-style tip callouts with variant styling"
#[test]
fn github_callout_tip() {
    assert_github_callout("TIP", "Tip");
}

// Jest: "renders GitHub-style important callouts with variant styling"
#[test]
fn github_callout_important() {
    assert_github_callout("IMPORTANT", "Important");
}

// Jest: "renders GitHub-style warning callouts with variant styling"
#[test]
fn github_callout_warning() {
    assert_github_callout("WARNING", "Warning");
}

// Jest: "renders GitHub-style caution callouts with variant styling"
#[test]
fn github_callout_caution() {
    assert_github_callout("CAUTION", "Caution");
}

#[track_caller]
fn assert_github_callout(marker: &str, label: &str) {
    let doc = convert(&format!("> [!{marker}]\n> {label} body."));
    let body = format!("{label} body.");
    assert_texts(&doc, &[label, body.as_str()]);
    let paragraphs = paragraphs(&doc);
    assert_run(paragraphs[0], label, true, false, false, false, false, None);
    assert!(!paragraphs[1].text.contains(&format!("[!{marker}]")));
    assert!(!paragraphs[0].text.contains(&format!("[!{marker}]")));
}

// Jest: "keeps unsupported GitHub-style callout markers as normal blockquotes"
#[test]
fn unsupported_callout_stays_a_quote() {
    let doc = convert(
        "> [!INFO]
> Unsupported info callout.",
    );
    assert_texts(&doc, &["[!INFO]\nUnsupported info callout."]);
    let paragraph = &paragraphs(&doc)[0];
    assert!(paragraph.text.contains("[!INFO]"));
    assert!(paragraph
        .runs
        .iter()
        .filter(|run| !run.text.trim().is_empty())
        .all(|run| run.italic));
}

// Jest: "preserves inline formatting and nested paragraphs inside callouts"
#[test]
fn callout_inline_formatting_and_nested_paragraphs() {
    let doc = convert(
        "> [!NOTE]
> First **bold** paragraph with [lnk](https://example.com/callout) and `code`.
>
> Second paragraph.",
    );
    assert_texts(
        &doc,
        &[
            "Note",
            "First bold paragraph with lnk and code.",
            "Second paragraph.",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_run(
        paragraphs[0],
        "Note",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    assert_run(
        paragraphs[1],
        "bold",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    assert_run(
        paragraphs[1],
        "lnk",
        false,
        false,
        true,
        false,
        false,
        Some("https://example.com/callout"),
    );
    assert_run(
        paragraphs[1],
        "code",
        false,
        false,
        false,
        false,
        true,
        None,
    );
    assert!(!doc
        .paragraph_texts()
        .iter()
        .any(|text| text.contains("[!NOTE]")));
}

// Jest: "emits bold, italic, underline, strikethrough, and inline code runs"
#[test]
fn bold_italic_underline_strike_and_inline_code() {
    let doc = convert(
        "This paragraph has **bold**, *italic*, ++underline++, ~~strikethrough~~, ***bold italic***, and `inline code`.",
    );
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(
        paragraph.text,
        "This paragraph has bold, italic, underline, strikethrough, bold italic, and inline code."
    );
    assert_run(paragraph, "bold", true, false, false, false, false, None);
    assert_run(paragraph, "italic", false, true, false, false, false, None);
    assert_run(
        paragraph,
        "underline",
        false,
        false,
        true,
        false,
        false,
        None,
    );
    assert_run(
        paragraph,
        "strikethrough",
        false,
        false,
        false,
        true,
        false,
        None,
    );
    assert_run(
        paragraph,
        "bold italic",
        true,
        true,
        false,
        false,
        false,
        None,
    );
    let code = run_named(paragraph, "inline code");
    assert!(code.code);
    assert!(!code.bold);
    assert!(!code.italic);
    assert_eq!(code.color.as_deref(), Some("444444"));
    assert_eq!(code.size, Some(20));
}

// Jest: "renders bullet and numbered lists with numbering references"
#[test]
fn bullet_and_numbered_lists() {
    let doc = convert(
        "## Bullets
- First bullet
- Second bullet with **bold**

## Numbers
1. First numbered
2. Second numbered

Interrupting paragraph.

1. Should restart
2. Second item of new list",
    );
    assert_texts(
        &doc,
        &[
            "Bullets",
            "First bullet",
            "Second bullet with bold",
            "Numbers",
            "First numbered",
            "Second numbered",
            "Interrupting paragraph.",
            "Should restart",
            "Second item of new list",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].heading.as_deref(), Some("Heading2"));
    assert_eq!(paragraphs[3].heading.as_deref(), Some("Heading2"));
    assert_list(paragraphs[1], false, 1, 0);
    assert_list(paragraphs[2], false, 1, 0);
    assert_run(
        paragraphs[2],
        "bold",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    let first = assert_list(paragraphs[4], true, 1, 0);
    let first_id = first.num_id.clone();
    assert_eq!(assert_list(paragraphs[5], true, 1, 0).num_id, first_id);
    let restarted = assert_list(paragraphs[7], true, 1, 0);
    assert_ne!(
        restarted.num_id, first_id,
        "a new ordered list should restart instead of continuing the previous numbering"
    );
    assert_eq!(
        assert_list(paragraphs[8], true, 1, 0).num_id,
        restarted.num_id
    );
}

// Jest: "renders checked, unchecked, and nested GFM task items"
#[test]
fn task_lists() {
    let doc = convert(
        "- [ ] Pending **bold** work
- [x] Done *today*
- [X] Uppercase checked
  - [ ] Nested `code` task",
    );
    assert_texts(
        &doc,
        &[
            "☐ Pending bold work",
            "☒ Done today",
            "☒ Uppercase checked",
            "☐ Nested code task",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_list(paragraphs[0], false, 1, 0);
    assert_list(paragraphs[1], false, 1, 0);
    assert_list(paragraphs[2], false, 1, 0);
    assert_list(paragraphs[3], false, 1, 1);
    assert_run(
        paragraphs[0],
        "bold",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    assert_run(
        paragraphs[1],
        "today",
        false,
        true,
        false,
        false,
        false,
        None,
    );
    let code = run_named(paragraphs[3], "code");
    assert!(code.code);
    assert_eq!(code.color.as_deref(), Some("444444"));
    assert_eq!(
        doc.paragraph_texts()
            .iter()
            .filter(|text| text.contains('☐'))
            .count(),
        2
    );
    assert_eq!(
        doc.paragraph_texts()
            .iter()
            .filter(|text| text.contains('☒'))
            .count(),
        2
    );
}

// Jest: "keeps ordinary ordered and unordered list rendering unchanged"
#[test]
fn ordinary_lists_have_no_task_markers() {
    let doc = convert(
        "- ordinary bullet
- **formatted** bullet

1. ordinary number
2. second number",
    );
    assert_texts(
        &doc,
        &[
            "ordinary bullet",
            "formatted bullet",
            "ordinary number",
            "second number",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_list(paragraphs[0], false, 1, 0);
    assert_list(paragraphs[1], false, 1, 0);
    assert_list(paragraphs[2], true, 1, 0);
    assert_list(paragraphs[3], true, 1, 0);
    assert_run(
        paragraphs[1],
        "formatted",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    let rendered = doc.render();
    assert!(!rendered.contains('☐'));
    assert!(!rendered.contains('☒'));
}

// Jest: "preserves zero and non-one starts at nested levels"
// tests/feature-completeness.test.ts, covering the slice 1 "ordered start" case.
//
// Numbering start is the level's w:startOverride when Word would use one, and
// otherwise w:start on the abstract numbering level. The Markdown markers are
// 5, 3, and 0. The TypeScript docx serializer writes startOverride 1 for a
// zero start, and so does this port.
#[test]
fn ordered_list_numbering_start() {
    let doc = convert(
        "5. outer

   3. inner

End.

0. zero",
    );
    assert_texts(&doc, &["outer", "inner", "End.", "zero"]);
    let paragraphs = paragraphs(&doc);
    let outer = assert_list(paragraphs[0], true, 5, 0);
    let inner = assert_list(paragraphs[1], true, 3, 1);
    assert!(paragraphs[2].list.is_none());
    let zero = assert_list(paragraphs[3], true, 1, 0);
    assert_ne!(outer.num_id, inner.num_id);
    assert_ne!(inner.num_id, zero.num_id);
    assert_ne!(outer.num_id, zero.num_id);
}

// Jest: "renders --- as a native Word horizontal rule"
// Jest: "renders *** as a native Word horizontal rule"
// Jest: "renders ___ as a native Word horizontal rule"
#[test]
fn thematic_break_markers() {
    for marker in ["---", "***", "___"] {
        let doc = convert(&format!("Before\n\n{marker}\n\nAfter"));
        assert_texts(&doc, &["Before", "", "After"]);
        let paragraphs = paragraphs(&doc);
        assert!(!paragraphs[0].rule, "{marker}");
        assert!(paragraphs[1].rule, "{marker}");
        assert!(!paragraphs[2].rule, "{marker}");
    }
}

// Jest: "renders thematic breaks inside blockquotes and list items"
#[test]
fn thematic_breaks_inside_quote_and_list_item() {
    let doc = convert(
        "> Quoted before
>
> ***
>
> Quoted after

- List item

  ___",
    );
    assert_texts(
        &doc,
        &["Quoted before", "", "Quoted after", "List item", ""],
    );
    let paragraphs = paragraphs(&doc);
    assert!(paragraphs[0].runs.iter().any(|run| run.italic));
    assert!(paragraphs[1].rule);
    assert!(paragraphs[2].runs.iter().any(|run| run.italic));
    assert_list(paragraphs[3], false, 1, 0);
    assert!(paragraphs[4].rule);
    assert_eq!(
        paragraphs
            .iter()
            .filter(|paragraph| paragraph.list.is_some())
            .count(),
        1
    );
}

// Jest: "renders fenced code block content with code formatting"
//
// Highlighting stays off, so run colors are the language label 666666 and the
// plain code color 444444. Leading indent spaces are non-breaking spaces.
// Default label size is 18 and default code-line size is 20 (the Jest call's
// codeBlockSize: 20 is a style option).
#[test]
fn fenced_code_without_token_colors() {
    let doc = convert(
        "Intro paragraph.

```typescript
function greet(name: string): string {
  return `Hello, ${name}!`;
}
```

Outro paragraph.",
    );
    let code = "typescript\nfunction greet(name: string): string {\n\u{00a0}\u{00a0}return `Hello, ${name}!`;\n}";
    assert_texts(&doc, &["Intro paragraph.", code, "Outro paragraph."]);
    assert_eq!(doc.drawing_count, 0);
    let paragraph = &paragraphs(&doc)[1];
    let label = run_named(paragraph, "typescript");
    assert!(label.bold);
    assert!(label.code);
    assert_eq!(label.color.as_deref(), Some("666666"));
    assert_eq!(label.size, Some(18));
    let body = run_named(paragraph, "function greet(name: string): string {");
    assert!(body.code);
    assert!(!body.bold);
    assert_eq!(body.color.as_deref(), Some("444444"));
    assert_eq!(body.size, Some(20));
    let indented = run_named(paragraph, "\u{00a0}\u{00a0}return `Hello, ${name}!`;");
    assert!(indented.code);
    assert_eq!(indented.color.as_deref(), Some("444444"));
    assert_eq!(indented.size, Some(20));
    let close = run_named(paragraph, "}");
    assert!(close.code);
    assert_eq!(close.color.as_deref(), Some("444444"));
    for run in &paragraph.runs {
        if let Some(color) = &run.color {
            assert!(
                color == "444444" || color == "666666",
                "fenced code picked up a token color {color} on {:?}",
                run.text
            );
        }
    }
}

// Jest: "renders headers, data rows, and column alignment markers"
#[test]
fn table_headers_and_column_alignment() {
    let doc = convert(
        "| Left | Center | Right |
|:-----|:------:|------:|
| a    | b      | c     |
| d    | e      | f     |",
    );
    let table = only_table(&doc);
    assert_eq!(table.columns, 3);
    assert_eq!(table.rows.len(), 3);
    assert_dxa_width(table, "9746");
    let cells = [
        ["Left", "Center", "Right"],
        ["a", "b", "c"],
        ["d", "e", "f"],
    ];
    let aligns = ["left", "center", "right"];
    for (row_index, row) in cells.iter().enumerate() {
        for (column, text) in row.iter().enumerate() {
            let paragraph = cell(table, row_index, column);
            assert_eq!(paragraph.text, *text);
            assert_eq!(paragraph.align.as_deref(), Some(aligns[column]));
        }
    }
    assert!(run_named(cell(table, 0, 0), "Left").bold);
    assert!(run_named(cell(table, 0, 1), "Center").bold);
    assert!(run_named(cell(table, 0, 2), "Right").bold);
    assert!(!run_named(cell(table, 1, 0), "a").bold);
}

// Jest: "preserves inline formatting in cells and headers (issue #35)"
#[test]
fn table_inline_formatting() {
    let doc = convert(
        "| **Bold Header** | *Italic Header* | `Code Header` |
|-----------------|-----------------|-----------------|
| **Bold text**   | *Italic text*   | ~~Deprecated~~  |
| `inline code` | [link](https://example.com) | ***bold italic*** |
| ++underline++   | normal          | text            |",
    );
    let table = only_table(&doc);
    assert_eq!(table.columns, 3);
    assert_eq!(table.rows.len(), 4);
    assert_dxa_width(table, "9746");
    assert_eq!(cell(table, 0, 0).text, "Bold Header");
    assert_eq!(cell(table, 0, 1).text, "Italic Header");
    assert_eq!(cell(table, 0, 2).text, "Code Header");
    assert_run(
        cell(table, 0, 0),
        "Bold Header",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    assert_run(
        cell(table, 0, 1),
        "Italic Header",
        true,
        true,
        false,
        false,
        false,
        None,
    );
    let code_header = run_named(cell(table, 0, 2), "Code Header");
    assert!(code_header.code);
    assert!(!code_header.bold);
    assert_run(
        cell(table, 1, 0),
        "Bold text",
        true,
        false,
        false,
        false,
        false,
        None,
    );
    assert_run(
        cell(table, 1, 1),
        "Italic text",
        false,
        true,
        false,
        false,
        false,
        None,
    );
    assert_run(
        cell(table, 1, 2),
        "Deprecated",
        false,
        false,
        false,
        true,
        false,
        None,
    );
    assert!(run_named(cell(table, 2, 0), "inline code").code);
    assert_run(
        cell(table, 2, 1),
        "link",
        false,
        false,
        true,
        false,
        false,
        Some("https://example.com"),
    );
    assert_run(
        cell(table, 2, 2),
        "bold italic",
        true,
        true,
        false,
        false,
        false,
        None,
    );
    assert_run(
        cell(table, 3, 0),
        "underline",
        false,
        false,
        true,
        false,
        false,
        None,
    );
    assert_eq!(cell(table, 3, 1).text, "normal");
    assert_eq!(cell(table, 3, 2).text, "text");
}

// Jest: "emits an integer table width that Word 2007 accepts (no percentage form)"
#[test]
fn table_integer_dxa_width() {
    let doc = convert(
        "| A | B |
|---|---|
| 1 | 2 |",
    );
    let table = only_table(&doc);
    assert_eq!(table.columns, 2);
    assert_eq!(cell(table, 0, 0).text, "A");
    assert_eq!(cell(table, 0, 1).text, "B");
    assert_eq!(cell(table, 1, 0).text, "1");
    assert_eq!(cell(table, 1, 1).text, "2");
    assert_dxa_width(table, "9746");
    let width = table.width.as_deref().unwrap();
    assert!(!width.contains('%'));
}

// Jest: "applies fontFamily to run properties"
//
// style.fontFamily "Trebuchet MS" cannot be passed to convert_markdown. This
// locks the fixture paragraph from that Jest test. The custom font name is not
// asserted.
#[test]
fn font_family_on_runs() {
    let doc = convert("Paragraph with custom font.");
    assert_texts(&doc, &["Paragraph with custom font."]);
}

// Jest: "supports the deprecated fontFamilly alias"
//
// style.fontFamilly "Arial" cannot be passed to convert_markdown. This locks
// the fixture paragraph. The alias font name is not asserted.
#[test]
fn font_familly_alias() {
    let doc = convert("Paragraph with alias font.");
    assert_texts(&doc, &["Paragraph with alias font."]);
}
