//! Slice 4 parity with `tests/captions-cross-references.test.ts`.
//!
//! Expected text, bookmark names, and `SEQ` / `REF` instructions come from
//! `@mohtasham/md-to-docx` `convertMarkdownToBuffer`. ZIP bytes are not
//! compared. [`md_to_docx::convert_markdown`] covers the default preserve
//! path. [`md_to_docx::convert_with_options`] covers sections and
//! [`md_to_docx::options::CaptionOptions`].

#[path = "common/mod.rs"]
mod common;

use common::{Block, Document, Paragraph, Run, Table};
use md_to_docx::options::{
    Alignment, CaptionFailureMode, CaptionOptions, CaptionPlacement, DocumentSection,
    SectionConfig, UpdateFieldsOnOpen,
};
use md_to_docx::{ConvertOptions, MarkdownConversionError};

const ONE_PX_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";

const SEQ_FIGURE: &str = " SEQ MdToDocxFigure \\* ARABIC ";
const SEQ_TABLE: &str = " SEQ MdToDocxTable \\* ARABIC ";

const FIG_OVERVIEW: &str = "mdxref_figoverview_1ec4keq";
const TBL_RESULTS: &str = "mdxref_tblresults_1me3ew5";
const FIG_LATER: &str = "mdxref_figlater_00i3dz5";
const FIG_DUPLICATE: &str = "mdxref_figduplicate_1coa4ym";
const FIG_ABC: &str = "mdxref_figabc_00sob43";
const FIG_FIRST: &str = "mdxref_figfirst_01a1m49";
const FIG_SECOND: &str = "mdxref_figsecond_0l6oik5";
const TBL_SECTION: &str = "mdxref_tblsection_11vrhr0";
const FIG_PROMPT_FREE: &str = "mdxref_figpromptfree_01brqhc";
const FIG_STYLED: &str = "mdxref_figstyled_16tmlwu";

fn ref_field(bookmark: &str) -> String {
    format!(" REF {bookmark} \\h ")
}

fn convert(markdown: &str) -> Document {
    let bytes = md_to_docx::convert_markdown(markdown)
        .unwrap_or_else(|err| panic!("convert_markdown failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_markdown should return a zip package"
    );
    common::project_docx(&bytes)
}

fn captions(options: CaptionOptions) -> ConvertOptions {
    ConvertOptions {
        captions: Some(options),
        ..ConvertOptions::default()
    }
}

fn expect_error(markdown: &str, options: &ConvertOptions) -> MarkdownConversionError {
    md_to_docx::convert_with_options(markdown, options)
        .expect_err("conversion should fail with MarkdownConversionError")
}

fn convert_with(markdown: &str, options: &ConvertOptions) -> Document {
    let bytes = md_to_docx::convert_with_options(markdown, options)
        .unwrap_or_else(|err| panic!("convert_with_options failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_with_options should return a zip package"
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
                "missing run {text:?}\n{}",
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        })
}

#[track_caller]
fn assert_run(
    paragraph: &Paragraph,
    text: &str,
    bold: bool,
    italic: bool,
    code: bool,
    link: Option<&str>,
) {
    let run = run_named(paragraph, text);
    assert_eq!(run.bold, bold, "bold on {text:?}");
    assert_eq!(run.italic, italic, "italic on {text:?}");
    assert_eq!(run.code, code, "code on {text:?}");
    assert_eq!(run.link.as_deref(), link, "link on {text:?}");
}

#[track_caller]
fn only_table(doc: &Document) -> &Table {
    let tables = doc.tables();
    assert_eq!(tables.len(), 1, "expected one table\n{}", doc.render());
    tables[0]
}

#[track_caller]
fn cell<'a>(table: &'a Table, row: usize, column: usize) -> &'a Paragraph {
    table
        .rows
        .get(row)
        .and_then(|cells| cells.get(column))
        .and_then(|cell| cell.paragraphs.first())
        .unwrap_or_else(|| panic!("missing cell {row},{column}\n{table:?}"))
}

fn every_paragraph(doc: &Document) -> Vec<&Paragraph> {
    let mut paragraphs = Vec::new();
    collect_paragraphs(&doc.blocks, &mut paragraphs);
    paragraphs
}

fn collect_paragraphs<'a>(blocks: &'a [Block], out: &mut Vec<&'a Paragraph>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => out.push(paragraph),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in row {
                        out.extend(cell.paragraphs.iter());
                    }
                }
            }
        }
    }
}

fn instructions(doc: &Document, kind: &str) -> Vec<String> {
    every_paragraph(doc)
        .into_iter()
        .flat_map(|paragraph| paragraph.fields.iter().cloned())
        .filter(|field| field.contains(kind))
        .collect()
}

#[track_caller]
fn assert_caption(paragraph: &Paragraph, text: &str, bookmark: &str, seq: &str) {
    assert_eq!(
        paragraph.text,
        text,
        "caption text\n{}",
        paragraph_debug(paragraph)
    );
    assert_eq!(paragraph.style_id.as_deref(), Some("Caption"));
    assert_eq!(paragraph.align.as_deref(), Some("center"));
    assert_eq!(paragraph.bookmarks, vec![bookmark.to_string()]);
    assert_eq!(paragraph.fields, vec![seq.to_string()]);
}

fn paragraph_debug(paragraph: &Paragraph) -> String {
    format!(
        "text={:?} style={:?} align={:?} bookmarks={:?} fields={:?}",
        paragraph.text, paragraph.style_id, paragraph.align, paragraph.bookmarks, paragraph.fields
    )
}

fn assert_word_bookmark(name: &str) {
    let mut chars = name.chars();
    let first = chars.next().unwrap_or_else(|| panic!("empty bookmark"));
    assert!(
        first.is_ascii_alphabetic() || first == '_',
        "bookmark {name} must start with a letter or underscore"
    );
    assert!(
        name.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
        "bookmark {name} has a character Word rejects"
    );
    assert!(
        name.chars().count() <= 40,
        "bookmark {name} is longer than 40"
    );
    assert!(!name.contains('/'));
    assert!(!name.contains('?'));
}

// Jest: "numbers figures and tables independently with native fields and bookmarks"
#[test]
fn numbers_figures_and_tables_independently() {
    let markdown = format!(
        "See [@fig:overview] and [@tbl:results].\n\n\
         ![Architecture overview]({ONE_PX_PNG})\n\n\
         : System *overview* {{#fig:overview}}\n\n\
         | Metric | Value |\n\
         | --- | --- |\n\
         | Score | 42 |\n\n\
         : **Results** summary {{#tbl:results}}"
    );
    let doc = convert(&markdown);

    assert_texts(
        &doc,
        &[
            "See Figure 1 and Table 1.",
            "",
            "Figure 1: System overview",
            "Table 1: Results summary",
        ],
    );
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert!(doc.update_fields, "settings should update fields on open");

    let body = paragraphs(&doc);
    assert_eq!(
        body[0].fields,
        vec![ref_field(FIG_OVERVIEW), ref_field(TBL_RESULTS)]
    );
    assert_caption(
        body[2],
        "Figure 1: System overview",
        FIG_OVERVIEW,
        SEQ_FIGURE,
    );
    assert_run(body[2], "overview", false, true, false, None);
    assert_caption(body[3], "Table 1: Results summary", TBL_RESULTS, SEQ_TABLE);
    assert_run(body[3], "Results", true, false, false, None);

    let table = only_table(&doc);
    assert_eq!(table.columns, 2);
    assert_eq!(cell(table, 0, 0).text, "Metric");
    assert_eq!(cell(table, 0, 1).text, "Value");
    assert_eq!(cell(table, 1, 0).text, "Score");
    assert_eq!(cell(table, 1, 1).text, "42");
    assert_eq!(instructions(&doc, " SEQ "), vec![SEQ_FIGURE, SEQ_TABLE]);
    assert_eq!(
        instructions(&doc, " REF "),
        vec![ref_field(FIG_OVERVIEW), ref_field(TBL_RESULTS)]
    );
}

// Jest: "supports multiple clickable forward references"
#[test]
fn multiple_clickable_forward_references() {
    let markdown = format!(
        "First [@fig:later], then [@fig:later] again.\n\n\
         ![Forward reference]({ONE_PX_PNG})\n\n\
         : Defined later {{#fig:later}}"
    );
    let doc = convert(&markdown);

    assert_texts(
        &doc,
        &[
            "First Figure 1, then Figure 1 again.",
            "",
            "Figure 1: Defined later",
        ],
    );
    let body = paragraphs(&doc);
    let cached: Vec<_> = body[0]
        .runs
        .iter()
        .filter(|run| run.text == "Figure 1")
        .collect();
    assert_eq!(cached.len(), 2, "two cached Figure 1 results");
    assert_caption(body[2], "Figure 1: Defined later", FIG_LATER, SEQ_FIGURE);
    let refs = instructions(&doc, " REF ");
    assert_eq!(refs, vec![ref_field(FIG_LATER), ref_field(FIG_LATER)]);
    assert!(refs.iter().all(|field| field.contains("\\h")));
}

// Jest: "continues automatic numbering across document sections"
#[test]
fn numbering_continues_across_sections() {
    let first = format!("![First]({ONE_PX_PNG})\n\n: First section {{#fig:first}}");
    let second = format!(
        "Reference [@fig:first].\n\n\
         ![Second]({ONE_PX_PNG})\n\n\
         : Second section {{#fig:second}}\n\n\
         | A |\n\
         | --- |\n\
         | B |\n\n\
         : Section table {{#tbl:section}}"
    );
    let doc = convert_with(
        "",
        &ConvertOptions {
            sections: Some(vec![
                DocumentSection {
                    markdown: first,
                    config: SectionConfig::default(),
                },
                DocumentSection {
                    markdown: second,
                    config: SectionConfig::default(),
                },
            ]),
            ..ConvertOptions::default()
        },
    );

    assert_texts(
        &doc,
        &[
            "",
            "Figure 1: First section",
            "",
            "Reference Figure 1.",
            "",
            "Figure 2: Second section",
            "Table 1: Section table",
        ],
    );
    assert_eq!(doc.drawing_count, 2, "drawings\n{}", doc.render());
    assert!(doc.update_fields, "settings should update fields on open");
    let body = paragraphs(&doc);
    assert_caption(body[1], "Figure 1: First section", FIG_FIRST, SEQ_FIGURE);
    assert_eq!(body[3].fields, vec![ref_field(FIG_FIRST)]);
    assert_caption(body[5], "Figure 2: Second section", FIG_SECOND, SEQ_FIGURE);
    assert_caption(body[6], "Table 1: Section table", TBL_SECTION, SEQ_TABLE);
    assert_eq!(
        instructions(&doc, " SEQ "),
        vec![SEQ_FIGURE, SEQ_FIGURE, SEQ_TABLE]
    );
    let table = only_table(&doc);
    assert_eq!(table.columns, 1);
    assert_eq!(cell(table, 0, 0).text, "A");
    assert_eq!(cell(table, 1, 0).text, "B");
}

// Jest: "keeps the first duplicate ID and preserves later duplicate syntax by default"
#[test]
fn first_duplicate_id_wins_and_later_syntax_is_preserved() {
    let markdown = format!(
        "![First]({ONE_PX_PNG})\n\n\
         : First caption {{#fig:duplicate}}\n\n\
         ![Second]({ONE_PX_PNG})\n\n\
         : Duplicate caption {{#fig:duplicate}}\n\n\
         See [@fig:duplicate]."
    );
    let doc = convert(&markdown);

    assert_texts(
        &doc,
        &[
            "",
            "Figure 1: First caption",
            "",
            ": Duplicate caption {#fig:duplicate}",
            "See Figure 1.",
        ],
    );
    let body = paragraphs(&doc);
    assert_caption(
        body[1],
        "Figure 1: First caption",
        FIG_DUPLICATE,
        SEQ_FIGURE,
    );
    assert!(body[3].fields.is_empty(), "later duplicate stays literal");
    assert!(body[3].bookmarks.is_empty());
    assert_eq!(body[3].align.as_deref(), Some("left"));
    assert_eq!(body[4].fields, vec![ref_field(FIG_DUPLICATE)]);
    assert_eq!(instructions(&doc, " SEQ "), vec![SEQ_FIGURE.to_string()]);
    assert!(doc.update_fields);
}

// Preserve default for Jest: "can fail on duplicate, malformed, and unresolved identifiers".
// The throw path is `duplicate_malformed_and_unresolved_identifiers_fail`.
#[test]
fn duplicate_malformed_and_unresolved_stay_literal_by_default() {
    let duplicate = format!(
        "![One]({ONE_PX_PNG})\n\n\
         : One {{#fig:same}}\n\n\
         ![Two]({ONE_PX_PNG})\n\n\
         : Two {{#fig:same}}"
    );
    let duplicate_doc = convert_with(&duplicate, &ConvertOptions::default());
    assert_texts(
        &duplicate_doc,
        &["", "Figure 1: One", "", ": Two {#fig:same}"],
    );
    assert_eq!(
        instructions(&duplicate_doc, " SEQ "),
        vec![SEQ_FIGURE.to_string()]
    );
    assert!(instructions(&duplicate_doc, " REF ").is_empty());

    let malformed = format!("![Bad]({ONE_PX_PNG})\n\n: Bad ID {{#tbl:not-a-figure}}");
    let malformed_doc = convert_with(&malformed, &ConvertOptions::default());
    assert_texts(&malformed_doc, &["", ": Bad ID {#tbl:not-a-figure}"]);
    assert!(instructions(&malformed_doc, " SEQ ").is_empty());
    assert!(instructions(&malformed_doc, " REF ").is_empty());
    assert_eq!(malformed_doc.drawing_count, 1);

    let unresolved = convert_with("Missing [@fig:unknown].", &ConvertOptions::default());
    assert_texts(&unresolved, &["Missing [@fig:unknown]."]);
    assert!(instructions(&unresolved, " REF ").is_empty());
    assert!(instructions(&unresolved, " SEQ ").is_empty());
}

// Jest: "preserves unresolved references as literal text by default"
#[test]
fn unresolved_references_stay_literal_by_default() {
    let doc = convert("Missing [@tbl:nope] remains visible.");

    assert_texts(&doc, &["Missing [@tbl:nope] remains visible."]);
    assert!(paragraphs(&doc)[0].fields.is_empty());
    assert!(instructions(&doc, " REF ").is_empty());
    assert!(instructions(&doc, " SEQ ").is_empty());
    assert!(!doc.update_fields);
}

// Jest: "sanitizes unsafe IDs into stable collision-resistant Word bookmarks"
#[test]
fn unsafe_ids_become_stable_bookmarks() {
    let markdown = format!(
        "See [@fig:a/b?c].\n\n\
         ![Unsafe ID]({ONE_PX_PNG})\n\n\
         : Unsafe identifier {{#fig:a/b?c}}"
    );
    let first = convert(&markdown);
    let second = convert(&markdown);

    assert_texts(
        &first,
        &["See Figure 1.", "", "Figure 1: Unsafe identifier"],
    );
    let body = paragraphs(&first);
    assert_caption(body[2], "Figure 1: Unsafe identifier", FIG_ABC, SEQ_FIGURE);
    assert_word_bookmark(FIG_ABC);
    assert_eq!(body[0].fields, vec![ref_field(FIG_ABC)]);
    assert!(body[0].fields[0].contains(FIG_ABC));
    assert!(body[0].fields[0].contains("\\h"));
    assert_eq!(
        paragraphs(&second)[2].bookmarks,
        paragraphs(&first)[2].bookmarks
    );
}

// Jest: "does not reinterpret ordinary images, tables, links, or inline code"
//
// The Jest file also checks `descr="Accessible ordinary image"`. The projector
// does not read drawing `descr`, so this test locks the drawing, the table,
// the hyperlink, and the code run that must stay `[@fig:literal]`.
#[test]
fn ordinary_images_tables_links_and_inline_code_stay_literal() {
    let markdown = format!(
        "![Accessible ordinary image]({ONE_PX_PNG})\n\n\
         | A | B |\n\
         | --- | --- |\n\
         | 1 | 2 |\n\n\
         [External link](https://example.com) and `[@fig:literal]`."
    );
    let doc = convert(&markdown);

    assert_texts(&doc, &["", "External link and [@fig:literal]."]);
    let body = paragraphs(&doc);
    assert!(body[0].fields.is_empty());
    assert!(body[1].fields.is_empty());
    assert_run(
        body[1],
        "External link",
        false,
        false,
        false,
        Some("https://example.com"),
    );
    assert_run(body[1], "[@fig:literal]", false, false, true, None);
    assert!(instructions(&doc, " SEQ ").is_empty());
    assert!(instructions(&doc, " REF ").is_empty());
    assert!(!doc.update_fields);
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());

    let table = only_table(&doc);
    assert_eq!(table.columns, 2);
    assert_eq!(cell(table, 0, 0).text, "A");
    assert_eq!(cell(table, 0, 1).text, "B");
    assert_eq!(cell(table, 1, 0).text, "1");
    assert_eq!(cell(table, 1, 1).text, "2");
}

#[track_caller]
fn assert_look(paragraph: &Paragraph, text: &str, italic: bool, size: u32) {
    let run = run_named(paragraph, text);
    assert_eq!(run.italic, italic, "italic on {text:?}");
    assert_eq!(run.size, Some(size), "size on {text:?}");
}

// Jest: "can suppress Word's automatic field-update prompt"
#[test]
fn suppress_field_update_on_open() {
    let markdown = format!(
        "See [@fig:prompt-free].\n\n\
         ![Prompt-free figure]({ONE_PX_PNG})\n\n\
         : Prompt-free caption {{#fig:prompt-free}}"
    );
    let doc = convert_with(
        &markdown,
        &captions(CaptionOptions {
            update_fields_on_open: Some(UpdateFieldsOnOpen::Bool(false)),
            ..CaptionOptions::default()
        }),
    );

    assert!(!doc.update_fields, "settings should omit w:updateFields");
    assert_texts(
        &doc,
        &["See Figure 1.", "", "Figure 1: Prompt-free caption"],
    );
    let body = paragraphs(&doc);
    assert_eq!(body[0].fields, vec![ref_field(FIG_PROMPT_FREE)]);
    assert_caption(
        body[2],
        "Figure 1: Prompt-free caption",
        FIG_PROMPT_FREE,
        SEQ_FIGURE,
    );
    assert_eq!(
        instructions(&doc, " REF "),
        vec![ref_field(FIG_PROMPT_FREE)]
    );
}

// Jest: "validates the automatic field-update option"
#[test]
fn invalid_update_fields_on_open_is_rejected() {
    let err = expect_error(
        "Text",
        &captions(CaptionOptions {
            update_fields_on_open: Some(UpdateFieldsOnOpen::Invalid),
            ..CaptionOptions::default()
        }),
    );
    assert_eq!(
        err.to_string(),
        "Invalid captions.updateFieldsOnOpen: Must be a boolean"
    );
}

// Jest: "can fail on duplicate, malformed, and unresolved identifiers"
#[test]
fn duplicate_malformed_and_unresolved_identifiers_fail() {
    let throw = captions(CaptionOptions {
        failure_mode: Some(CaptionFailureMode::Throw),
        ..CaptionOptions::default()
    });
    let duplicate = format!(
        "![One]({ONE_PX_PNG})\n\n\
         : One {{#fig:same}}\n\n\
         ![Two]({ONE_PX_PNG})\n\n\
         : Two {{#fig:same}}"
    );
    let err = expect_error(&duplicate, &throw);
    assert_eq!(err.to_string(), "Duplicate caption identifier: fig:same");

    let malformed = format!("![Bad]({ONE_PX_PNG})\n\n: Bad ID {{#tbl:not-a-figure}}");
    let err = expect_error(&malformed, &throw);
    assert_eq!(err.to_string(), "Figure captions require a fig: identifier");

    let err = expect_error("Missing [@fig:unknown].", &throw);
    assert_eq!(err.to_string(), "Unresolved cross-reference: fig:unknown");
}

// Jest: "applies typed labels, placement, and caption styling"
#[test]
fn typed_labels_placement_and_caption_styling() {
    let markdown = format!("![Styled]({ONE_PX_PNG})\n\n: Styled caption {{#fig:styled}}");
    let doc = convert_with(
        &markdown,
        &captions(CaptionOptions {
            figure_label: Some("Illustration".to_string()),
            figure_placement: Some(CaptionPlacement::Above),
            alignment: Some(Alignment::Left),
            italic: Some(true),
            size: Some(20),
            ..CaptionOptions::default()
        }),
    );

    assert_texts(&doc, &["Illustration 1: Styled caption", ""]);
    let body = paragraphs(&doc);
    assert_eq!(
        body[0].text,
        "Illustration 1: Styled caption",
        "caption text\n{}",
        paragraph_debug(body[0])
    );
    assert_eq!(body[0].style_id.as_deref(), Some("Caption"));
    assert_eq!(body[0].align.as_deref(), Some("left"));
    assert_eq!(body[0].bookmarks, vec![FIG_STYLED.to_string()]);
    assert_eq!(body[0].fields, vec![SEQ_FIGURE.to_string()]);
    assert_look(body[0], "Illustration ", true, 20);
    assert_look(body[0], ": ", true, 20);
    assert_look(body[0], "Styled caption", true, 20);
    assert!(body[1].text.is_empty(), "the drawing follows the caption");
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
}
