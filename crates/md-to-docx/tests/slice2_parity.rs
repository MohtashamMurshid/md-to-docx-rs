//! Slice 2 parity with the Jest suite.
//!
//! Each test calls [`md_to_docx::convert_markdown`] and compares a projection of
//! the unzipped package: paragraph text, heading bookmarks, page breaks,
//! footnote references, `NOTEREF` instructions, and footnote body text. ZIP
//! bytes are not compared.
//!
//! Expected values come from `@mohtasham/md-to-docx` `convertMarkdownToBuffer`
//! with default options. `convert_markdown` takes only a string, so a Jest
//! case that passes `sections` cannot send that option. Those tests still use
//! the same Markdown strings and say what the combined package would check.

#[path = "common/mod.rs"]
mod common;

use common::{Document, Footnote, Paragraph, Run};

const ONE_PX_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";

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
                "missing run {text:?} in {paragraph:?}\n{}",
                render_hint(paragraph)
            )
        })
}

fn render_hint(paragraph: &Paragraph) -> String {
    paragraph
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

#[track_caller]
fn only_footnote(doc: &Document) -> &Footnote {
    assert_eq!(doc.footnotes.len(), 1, "footnote count\n{}", doc.render());
    &doc.footnotes[0]
}

#[track_caller]
fn footnote_texts(note: &Footnote) -> Vec<&str> {
    note.paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect()
}

// Jest: "renders a basic Markdown footnote as native DOCX footnote markup"
#[test]
fn basic_footnote_markup() {
    let doc = convert("Text with a footnote[^1].\n\n[^1]: Footnote body.");
    assert_texts(&doc, &["Text with a footnote."]);
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.bookmarks, ["mdfootnote_1"]);
    assert_eq!(paragraph.footnote_refs, ["1"]);
    assert!(paragraph.fields.is_empty());
    assert!(!paragraph.text.contains("Footnote body."));

    let note = only_footnote(&doc);
    assert_eq!(note.id, "1");
    assert_eq!(footnote_texts(note), ["Footnote body."]);
}

// Jest: "preserves inline formatting, links, and code inside footnote content"
#[test]
fn footnote_inline_formatting() {
    let doc = convert(
        "Formatted note[^fmt].\n\n[^fmt]: **bold**, *italic*, [link](https://example.com/fn), and `code`.",
    );
    assert_texts(&doc, &["Formatted note."]);
    let note = only_footnote(&doc);
    assert_eq!(note.id, "1");
    assert_eq!(footnote_texts(note), ["bold, italic, link, and code."]);
    let body = &note.paragraphs[0];
    let bold = run_named(body, "bold");
    assert!(bold.bold);
    assert!(!bold.italic);
    let italic = run_named(body, "italic");
    assert!(italic.italic);
    assert!(!italic.bold);
    let link = run_named(body, "link");
    assert_eq!(link.link.as_deref(), Some("https://example.com/fn"));
    assert!(link.underline);
    assert_eq!(link.color.as_deref(), Some("0000FF"));
    assert!(!link.code);
    let code = run_named(body, "code");
    assert!(code.code);
    assert!(!code.bold);
    assert_eq!(code.color.as_deref(), Some("444444"));
    assert!(doc
        .footnote_rels
        .iter()
        .any(|rel| rel.target == "https://example.com/fn"));
}

// Jest: "renders repeated footnote references as valid NOTEREF fields"
#[test]
fn repeated_footnote_noteref() {
    let doc = convert(
        "First[^alpha], second[^beta], first again[^alpha].\n\n[^beta]: Beta body.\n[^alpha]: Alpha body.",
    );
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.text, "First, second, first again1.");
    assert_eq!(paragraph.footnote_refs, ["1", "2"]);
    assert_eq!(paragraph.bookmarks, ["mdfootnote_1", "mdfootnote_2"]);
    assert_eq!(paragraph.fields, [" NOTEREF mdfootnote_1 \\h \\f "]);

    assert_eq!(doc.footnotes.len(), 2, "footnote count\n{}", doc.render());
    assert_eq!(doc.footnotes[0].id, "1");
    assert_eq!(footnote_texts(&doc.footnotes[0]), ["Alpha body."]);
    assert_eq!(doc.footnotes[1].id, "2");
    assert_eq!(footnote_texts(&doc.footnotes[1]), ["Beta body."]);
    assert!(!paragraph.text.contains("Alpha body."));
    assert!(!paragraph.text.contains("Beta body."));
}

// Jest: "keeps unresolved footnote references as literal text"
#[test]
fn unresolved_footnote_stays_literal() {
    let doc = convert("Missing note[^missing] stays visible.");
    assert_texts(&doc, &["Missing note[^missing] stays visible."]);
    let paragraph = &paragraphs(&doc)[0];
    assert!(paragraph.footnote_refs.is_empty());
    assert!(paragraph.bookmarks.is_empty());
    assert!(doc.footnotes.is_empty(), "user footnotes\n{}", doc.render());
}

// Jest: "keeps nested footnote references inside footnote content as literal text"
#[test]
fn nested_footnote_stays_literal() {
    let doc = convert(
        "Outer note[^outer].\n\n[^outer]: Nested reference [^inner] stays visible.\n[^inner]: Inner body.",
    );
    assert_texts(&doc, &["Outer note."]);
    let note = only_footnote(&doc);
    assert_eq!(note.id, "1");
    assert_eq!(
        footnote_texts(note),
        ["Nested reference [^inner] stays visible."]
    );
    assert!(!footnote_texts(note).join("\n").contains("Inner body."));
}

// Jest: "keeps footnote IDs unique across sections"
//
// `convert_markdown` cannot take `options.sections`. The Jest fixture is two
// section strings packed together, and that package uses footnote ids 1 and 2.
// Each string on its own is id 1. The combined uniqueness check cannot run
// through this API.
#[test]
fn footnote_ids_unique_across_sections() {
    let first = convert("Section one[^one].\n\n[^one]: First section footnote.");
    assert_eq!(paragraphs(&first)[0].footnote_refs, ["1"]);
    assert_eq!(paragraphs(&first)[0].bookmarks, ["mdfootnote_1"]);
    assert_eq!(
        footnote_texts(only_footnote(&first)),
        ["First section footnote."]
    );

    let second = convert("Section two[^two].\n\n[^two]: Second section footnote.");
    assert_eq!(paragraphs(&second)[0].footnote_refs, ["1"]);
    assert_eq!(paragraphs(&second)[0].bookmarks, ["mdfootnote_1"]);
    assert_eq!(
        footnote_texts(only_footnote(&second)),
        ["Second section footnote."]
    );
}

// Slice 2 plan, no Jest name: tables inside footnote bodies flatten to " | " separated text.
#[test]
fn footnote_table_flattens_to_text() {
    let doc = convert("See table[^t].\n\n[^t]: | A | B |\n    | --- | --- |\n    | 1 | 2 |");
    assert!(doc.tables().is_empty(), "body tables\n{}", doc.render());
    let note = only_footnote(&doc);
    assert_eq!(footnote_texts(note), ["A | B", "1 | 2"]);
}

// Slice 2 plan, no Jest name: unresolved footnote image tokens become `mdImageN`
// relationships in `word/_rels/footnotes.xml.rels` (`src/packageRepairs.ts`).
#[test]
fn footnote_image_relationship() {
    let markdown = format!("See image[^img].\n\n[^img]: ![dot]({ONE_PX_PNG})");
    let doc = convert(&markdown);
    assert_eq!(doc.drawing_count, 0, "body drawings\n{}", doc.render());
    let note = only_footnote(&doc);
    assert_eq!(note.drawing_count, 1);
    assert_eq!(note.embeds, ["mdImage1"]);
    let rel = doc
        .footnote_rels
        .iter()
        .find(|rel| rel.id == "mdImage1")
        .unwrap_or_else(|| panic!("missing mdImage1\n{}", doc.render()));
    assert!(
        rel.rel_type.ends_with("/image"),
        "rel type {}",
        rel.rel_type
    );
    assert!(rel.target.starts_with("media/"), "target {}", rel.target);
    assert!(rel.target.ends_with(".png"), "target {}", rel.target);
    assert!(
        doc.media_parts
            .iter()
            .any(|part| part == &format!("word/{}", rel.target)),
        "media parts {:?}\n{}",
        doc.media_parts,
        doc.render()
    );
    assert!(note.embeds.iter().all(|embed| !embed.starts_with("rId{")));
}

// Jest: "emits a TOC field and an explicit page break"
//
// The `[TOC]` field is slice 3. This test locks the page break and the two
// heading bookmarks around it. "supports custom TOC title and depth filtering"
// needs `toc` options and is not asserted here.
#[test]
fn explicit_page_break() {
    let doc =
        convert("[TOC]\n\n# Section 1\n\nContent.\n\n\\pagebreak\n\n# Section 2\n\nMore content.");
    let paras = paragraphs(&doc);
    let break_at = paras
        .iter()
        .position(|paragraph| paragraph.page_break)
        .unwrap_or_else(|| panic!("missing page break\n{}", doc.render()));
    assert!(paras[break_at].text.is_empty());
    assert_eq!(
        paras
            .iter()
            .filter(|paragraph| paragraph.page_break)
            .count(),
        1
    );

    let headings: Vec<_> = paras
        .iter()
        .filter(|paragraph| paragraph.heading.as_deref() == Some("Heading1"))
        .collect();
    assert_eq!(headings.len(), 2, "headings\n{}", doc.render());
    assert_eq!(headings[0].text, "Section 1");
    assert_eq!(headings[0].bookmarks, ["_Toc_Section_1_1"]);
    assert_eq!(headings[1].text, "Section 2");
    assert_eq!(headings[1].bookmarks, ["_Toc_Section_2_2"]);

    let content_at = paras
        .iter()
        .position(|paragraph| paragraph.text == "Content.")
        .unwrap();
    let more_at = paras
        .iter()
        .position(|paragraph| paragraph.text == "More content.")
        .unwrap();
    let section2_at = paras
        .iter()
        .position(|paragraph| {
            paragraph.text == "Section 2" && paragraph.heading.as_deref() == Some("Heading1")
        })
        .unwrap();
    assert!(content_at < break_at);
    assert!(break_at < section2_at);
    assert!(section2_at < more_at);
}

// Jest: "emits a TOC field and an explicit page break"
//
// `classifyHtmlNode` treats an HTML block whose text contains "pagebreak" as
// the same page break `\pagebreak` becomes in that Jest fixture.
#[test]
fn html_pagebreak_block() {
    let doc = convert("Before.\n\n<div class=\"pagebreak\"></div>\n\nAfter.");
    assert_texts(&doc, &["Before.", "", "After."]);
    let paras = paragraphs(&doc);
    assert!(!paras[0].page_break);
    assert!(paras[1].page_break);
    assert!(!paras[2].page_break);
}

// Jest: "renders the full comment text, not just the first character"
#[test]
fn comment_full_text() {
    let doc = convert("# Title\n\n<!-- COMMENT: hello world this is a note -->\n\nBody.");
    assert_texts(
        &doc,
        &["Title", "Comment: hello world this is a note", "Body."],
    );
    let paras = paragraphs(&doc);
    assert_eq!(paras[0].heading.as_deref(), Some("Heading1"));
    assert_eq!(paras[0].bookmarks, ["_Toc_Title_1"]);
    let comment = run_named(&paras[1], "Comment: hello world this is a note");
    assert!(comment.italic);
    assert!(!comment.bold);
    assert_eq!(comment.color.as_deref(), Some("666666"));
    assert!(!run_named(&paras[2], "Body.").italic);
}

// Jest: "renders multi-word comments inside blockquotes"
#[test]
fn comment_inside_blockquote() {
    let doc = convert("> quoted\n>\n> <!-- COMMENT: reviewer feedback here -->");
    assert_texts(&doc, &["quoted", "Comment: reviewer feedback here"]);
    let paras = paragraphs(&doc);
    let quoted = run_named(&paras[0], "quoted");
    assert!(quoted.italic);
    assert_eq!(quoted.color.as_deref(), Some("000000"));
    let comment = run_named(&paras[1], "Comment: reviewer feedback here");
    assert!(comment.italic);
    assert_eq!(comment.color.as_deref(), Some("666666"));
}

// Jest: "does not treat non-comment HTML containing COMMENT: as a comment"
#[test]
fn non_comment_html_is_not_a_comment() {
    let doc = convert("Before.\n\n<div>COMMENT: just markup text</div>\n\nAfter.");
    assert_texts(&doc, &["Before.", "After."]);
    assert!(doc
        .paragraph_texts()
        .iter()
        .all(|text| !text.contains("Comment:")));
    assert!(doc
        .paragraph_texts()
        .iter()
        .all(|text| !text.contains("just markup text")));
}

// Jest: "generates deterministic unique bookmark names across sections"
//
// The Jest fixture splits `# Repeat` across `options.sections`. One document
// uses the same counter, so the names are still `_Toc_Repeat_1` and
// `_Toc_Repeat_2`. The second conversion locks `sanitizeForBookmarkId` on
// punctuation, which that counter feeds.
#[test]
fn deterministic_heading_bookmarks() {
    let doc = convert("# Repeat\n\n# Repeat");
    let paras = paragraphs(&doc);
    assert_eq!(paras.len(), 2);
    assert_eq!(paras[0].text, "Repeat");
    assert_eq!(paras[1].text, "Repeat");
    assert_eq!(paras[0].heading.as_deref(), Some("Heading1"));
    assert_eq!(paras[1].heading.as_deref(), Some("Heading1"));
    assert_eq!(paras[0].bookmarks, ["_Toc_Repeat_1"]);
    assert_eq!(paras[1].bookmarks, ["_Toc_Repeat_2"]);

    let punct = convert("# Hello, World! (v2)");
    let heading = &paragraphs(&punct)[0];
    assert_eq!(heading.text, "Hello, World! (v2)");
    assert_eq!(heading.bookmarks, ["_Toc_Hello_World_v2_1"]);
}

// Jest: "resolves forward, duplicate, Unicode and cross-section heading links"
// (`tests/feature-completeness.test.ts`)
//
// That fixture's hyperlink anchors cross `options.sections`, which
// `convert_markdown` cannot take. These are the bookmark names for the same
// headings in one document: Alpha, 重复, Alpha again.
#[test]
fn unicode_and_duplicate_heading_bookmarks() {
    let doc = convert("# Alpha\n\n# 重复\n\n# Alpha");
    let paras = paragraphs(&doc);
    assert_eq!(paras.len(), 3);
    assert_eq!(paras[0].text, "Alpha");
    assert_eq!(paras[1].text, "重复");
    assert_eq!(paras[2].text, "Alpha");
    assert!(paras
        .iter()
        .all(|paragraph| paragraph.heading.as_deref() == Some("Heading1")));
    assert_eq!(paras[0].bookmarks, ["_Toc_Alpha_1"]);
    assert_eq!(paras[1].bookmarks, ["_Toc___2"]);
    assert_eq!(paras[2].bookmarks, ["_Toc_Alpha_3"]);
}
