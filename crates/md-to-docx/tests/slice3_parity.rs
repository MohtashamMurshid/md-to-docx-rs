//! Slice 3 parity with the Jest suite.
//!
//! Style, section, header, footer, and TOC cases call
//! [`md_to_docx::convert_with_options`] with [`md_to_docx::ConvertOptions`].
//! [`md_to_docx::convert_markdown`] is the no-option wrapper and covers the
//! default page geometry only.
//!
//! Field names follow `src/options.rs`. Heading level overrides are
//! `heading_alignments` and `heading_size`, index 0 for H1 through index 5
//! for H6. A cleared header or footer slot is [`HeaderFooterSlot::Remove`].
//! `title_page` is `Option<bool>`.
//!
//! These Jest cases need options `StyleOverrides` does not have, so this file
//! does not send them:
//! - "applies configured GitHub-style callout colors" needs callout colors
//! - "applies configurable inline code styling" needs inline code size, color,
//!   and background
//! - "honors configurable tableLayout option" needs `tableLayout`
//! - the blockquote half of "applies paragraph and blockquote alignments"
//!   needs `blockquoteAlignment`
//! - "rejects invalid section type", "rejects invalid page orientation",
//!   "rejects invalid header alignment", and "rejects invalid titlePage type"
//!   send `"BAD_TYPE"`, `"SIDEWAYS"`, `"MIDDLE"`, and `"yes"`. `SectionBreak`,
//!   `Orientation`, `Alignment`, and `Option<bool>` cannot carry those values.

#[path = "common/mod.rs"]
mod common;

use common::{Document, Paragraph, Run, SectionProps, Story};
use md_to_docx::options::{
    Alignment, DocumentSection, HeaderFooterContent, HeaderFooterGroup, HeaderFooterSlot,
    MarginInput, Orientation, PageInput, PageNumberDisplay, PageNumberFormat, PageNumberSeparator,
    PageNumberingInput, SectionBreak, SectionConfig, SizeInput, StyleOverrides, TocMode,
    TocOverrides,
};
use md_to_docx::{ConvertOptions, MarkdownConversionError};

const ONE_PX_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";

fn convert(markdown: &str, options: &ConvertOptions) -> Document {
    let bytes = md_to_docx::convert_with_options(markdown, options)
        .unwrap_or_else(|err| panic!("convert_with_options failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_with_options should return a zip package"
    );
    common::project_docx(&bytes)
}

fn convert_default(markdown: &str) -> Document {
    let bytes = md_to_docx::convert_markdown(markdown)
        .unwrap_or_else(|err| panic!("convert_markdown failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_markdown should return a zip package"
    );
    common::project_docx(&bytes)
}

fn expect_error(markdown: &str, options: &ConvertOptions) -> MarkdownConversionError {
    md_to_docx::convert_with_options(markdown, options)
        .expect_err("conversion should fail with MarkdownConversionError")
}

fn section(markdown: &str, config: SectionConfig) -> DocumentSection {
    DocumentSection {
        markdown: markdown.to_string(),
        config,
    }
}

fn heading_align(levels: &[(usize, Alignment)]) -> [Option<Alignment>; 6] {
    let mut alignments = [None; 6];
    for &(index, alignment) in levels {
        alignments[index] = Some(alignment);
    }
    alignments
}

fn heading_size(levels: &[(usize, u32)]) -> [Option<u32>; 6] {
    let mut sizes = [None; 6];
    for &(index, size) in levels {
        sizes[index] = Some(size);
    }
    sizes
}

fn content(slot: HeaderFooterContent) -> Option<HeaderFooterSlot> {
    Some(HeaderFooterSlot::Content(slot))
}

fn removed() -> Option<HeaderFooterSlot> {
    Some(HeaderFooterSlot::Remove)
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
        .unwrap_or_else(|| panic!("missing run {text:?}\n{}", paragraph.text))
}

#[track_caller]
fn assert_default_page(section: &SectionProps) {
    assert_eq!(section.page_width, Some(11906));
    assert_eq!(section.page_height, Some(16838));
    assert_eq!(section.orientation.as_deref(), Some("portrait"));
    assert_eq!(section.margin_top, Some(1440));
    assert_eq!(section.margin_right, Some(1080));
    assert_eq!(section.margin_bottom, Some(1440));
    assert_eq!(section.margin_left, Some(1080));
    assert_eq!(section.margin_header, Some(708));
    assert_eq!(section.margin_footer, Some(708));
    assert_eq!(section.gutter, Some(0));
}

#[track_caller]
fn story<'a>(doc: &'a Document, file: &str) -> &'a Story {
    doc.stories
        .iter()
        .find(|story| story.path == format!("word/{file}"))
        .unwrap_or_else(|| panic!("missing word/{file}\n{}", doc.render()))
}

fn story_paragraphs(story: &Story) -> Vec<&Paragraph> {
    let mut paragraphs = Vec::new();
    for block in &story.blocks {
        match block {
            common::Block::Paragraph(paragraph) => paragraphs.push(paragraph),
            common::Block::Table(table) => {
                for row in &table.rows {
                    for cell in row {
                        paragraphs.extend(cell.paragraphs.iter());
                    }
                }
            }
        }
    }
    paragraphs
}

fn story_fields(story: &Story) -> Vec<String> {
    story_paragraphs(story)
        .into_iter()
        .flat_map(|paragraph| paragraph.fields.iter().cloned())
        .collect()
}

fn body_fields(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.fields.iter().cloned())
        .collect()
}

fn body_field_chars(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.field_chars.iter().cloned())
        .collect()
}

// Default page geometry from `sectionBuilder.ts`. `convert_markdown` passes
// no options. Jest section fixtures rely on these twips when a section omits
// page size, and on a centered PAGE footer.
#[test]
fn default_page_geometry_and_page_footer() {
    let doc = convert_default("# Hello\n\nBody.");
    assert_texts(&doc, &["Hello", "Body."]);
    assert_eq!(doc.sections.len(), 1, "sections\n{}", doc.render());
    let section = &doc.sections[0];
    assert_default_page(section);
    assert!(!section.title_page);
    assert_eq!(section.footer_refs.len(), 1);
    assert_eq!(section.footer_refs[0].kind, "default");
    assert!(section.header_refs.is_empty());
    assert!(!doc.update_fields);

    let footer = story(&doc, "footer1.xml");
    let paragraphs = story_paragraphs(footer);
    assert_eq!(paragraphs.len(), 1, "footer\n{}", doc.render());
    assert_eq!(paragraphs[0].align.as_deref(), Some("center"));
    assert_eq!(paragraphs[0].fields, ["PAGE"]);
    assert!(paragraphs[0].text.is_empty());
}

// Jest: "applies level-specific heading alignments and falls back to headingAlignment"
#[test]
fn heading_alignment_fallback() {
    let doc = convert(
        "# H1 centered\n## H2 right\n### H3 default-from-fallback\n#### H4 default-from-fallback\n##### H5 default-from-fallback",
        &ConvertOptions {
            style: Some(StyleOverrides {
                heading_alignment: Some(Alignment::Justified),
                heading_alignments: heading_align(&[
                    (0, Alignment::Center),
                    (1, Alignment::Right),
                ]),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let paragraphs = paragraphs(&doc);
    let expected = [
        (
            "Heading1",
            "H1 centered",
            "center",
            32,
            "_Toc_H1_centered_1",
        ),
        ("Heading2", "H2 right", "right", 28, "_Toc_H2_right_2"),
        (
            "Heading3",
            "H3 default-from-fallback",
            "both",
            24,
            "_Toc_H3_defaultfromfallback_3",
        ),
        (
            "Heading4",
            "H4 default-from-fallback",
            "both",
            20,
            "_Toc_H4_defaultfromfallback_4",
        ),
        (
            "Heading5",
            "H5 default-from-fallback",
            "both",
            16,
            "_Toc_H5_defaultfromfallback_5",
        ),
    ];
    assert_eq!(
        paragraphs.len(),
        expected.len(),
        "headings\n{}",
        doc.render()
    );
    for (paragraph, (style, text, align, size, bookmark)) in paragraphs.iter().zip(expected) {
        assert_eq!(paragraph.heading.as_deref(), Some(style));
        assert_eq!(paragraph.text, text);
        assert_eq!(paragraph.align.as_deref(), Some(align));
        assert_eq!(run_named(paragraph, text).size, Some(size));
        assert_eq!(paragraph.bookmarks, [bookmark]);
    }
}

// Jest: "renders H6 headings with level-specific styling"
#[test]
fn heading6_size_and_alignment() {
    let doc = convert(
        "###### H6 custom",
        &ConvertOptions {
            style: Some(StyleOverrides {
                heading_alignments: heading_align(&[(5, Alignment::Right)]),
                heading_size: heading_size(&[(5, 18)]),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs.len(), 1);
    assert_eq!(paragraphs[0].heading.as_deref(), Some("Heading6"));
    assert_eq!(paragraphs[0].text, "H6 custom");
    assert_eq!(paragraphs[0].align.as_deref(), Some("right"));
    assert_eq!(run_named(paragraphs[0], "H6 custom").size, Some(18));
    assert_eq!(paragraphs[0].bookmarks, ["_Toc_H6_custom_1"]);
}

// Jest: "applies paragraph and blockquote alignments"
//
// `StyleOverrides` has `paragraph_alignment` and no blockquote alignment.
// With only paragraph alignment set, the quote keeps the default of no `w:jc`.
#[test]
fn paragraph_and_blockquote_alignment() {
    let doc = convert(
        "Left paragraph here.\n\n> A blockquote that should be centered.\n\nAnother paragraph with justified alignment.",
        &ConvertOptions {
            style: Some(StyleOverrides {
                paragraph_alignment: Some(Alignment::Justified),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert_texts(
        &doc,
        &[
            "Left paragraph here.",
            "A blockquote that should be centered.",
            "Another paragraph with justified alignment.",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].align.as_deref(), Some("both"));
    assert_eq!(paragraphs[1].align, None);
    assert_eq!(paragraphs[2].align.as_deref(), Some("both"));
    assert_eq!(
        run_named(paragraphs[0], "Left paragraph here.").size,
        Some(24)
    );
}

// Jest: "renders fenced code block content with code formatting"
//
// `code_block_size: 20` makes the language label and the code lines both
// half-point size 20. Highlighting stays off.
#[test]
fn fenced_code_block_size() {
    let doc = convert(
        "Intro paragraph.\n\n```typescript\nfunction greet(name: string): string {\n  return `Hello, ${name}!`;\n}\n```\n\nOutro paragraph.",
        &ConvertOptions {
            style: Some(StyleOverrides {
                code_block_size: Some(20),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let code = "typescript\nfunction greet(name: string): string {\n\u{00a0}\u{00a0}return `Hello, ${name}!`;\n}";
    assert_texts(&doc, &["Intro paragraph.", code, "Outro paragraph."]);
    let paragraph = &paragraphs(&doc)[1];
    let label = run_named(paragraph, "typescript");
    assert!(label.bold);
    assert!(label.code);
    assert_eq!(label.size, Some(20));
    assert_eq!(label.color.as_deref(), Some("666666"));
    assert_eq!(label.font.as_deref(), Some("Courier New"));
    assert_eq!(label.fill.as_deref(), Some("F5F5F5"));
    for text in [
        "function greet(name: string): string {",
        "\u{00a0}\u{00a0}return `Hello, ${name}!`;",
        "}",
    ] {
        let run = run_named(paragraph, text);
        assert!(run.code);
        assert_eq!(run.size, Some(20));
        assert_eq!(run.color.as_deref(), Some("444444"));
        assert_eq!(run.fill.as_deref(), Some("F5F5F5"));
    }
}

// Jest: "scales table width to the configured page size and margins"
#[test]
fn table_width_follows_page_size() {
    let markdown = "| A | B |\n|---|---|\n| 1 | 2 |";
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![section(
                markdown,
                SectionConfig {
                    page: Some(PageInput {
                        size: Some(SizeInput {
                            width: Some(12240),
                            height: Some(15840),
                            ..SizeInput::default()
                        }),
                        margin: Some(MarginInput {
                            left: Some(1440),
                            right: Some(1440),
                            ..MarginInput::default()
                        }),
                    }),
                    ..SectionConfig::default()
                },
            )]),
            ..ConvertOptions::default()
        },
    );
    let table = &doc.tables()[0];
    assert_eq!(table.width.as_deref(), Some("9360"));
    assert_eq!(table.width_type.as_deref(), Some("dxa"));
    assert_eq!(table.layout.as_deref(), Some("autofit"));
    assert_eq!(doc.sections.len(), 1);
    let section = &doc.sections[0];
    assert_eq!(section.page_width, Some(12240));
    assert_eq!(section.page_height, Some(15840));
    assert_eq!(section.orientation.as_deref(), Some("portrait"));
    assert_eq!(section.margin_left, Some(1440));
    assert_eq!(section.margin_right, Some(1440));
    assert_eq!(section.margin_top, Some(1440));
    assert_eq!(section.margin_bottom, Some(1440));
}

// Jest: "applies fontFamily to run properties"
#[test]
fn font_family_on_runs() {
    let doc = convert(
        "Paragraph with custom font.",
        &ConvertOptions {
            style: Some(StyleOverrides {
                font_family: Some("Trebuchet MS".to_string()),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(paragraph.text, "Paragraph with custom font.");
    assert_eq!(
        run_named(paragraph, "Paragraph with custom font.")
            .font
            .as_deref(),
        Some("Trebuchet MS")
    );
}

// Jest: "supports the deprecated fontFamilly alias"
#[test]
fn font_familly_alias() {
    let doc = convert(
        "Paragraph with alias font.",
        &ConvertOptions {
            style: Some(StyleOverrides {
                font_familly: Some("Arial".to_string()),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let paragraph = &paragraphs(&doc)[0];
    assert_eq!(
        run_named(paragraph, "Paragraph with alias font.")
            .font
            .as_deref(),
        Some("Arial")
    );
}

// Jest: "throws MarkdownConversionError for whitespace-only fontFamily"
#[test]
fn whitespace_font_family_is_rejected() {
    let err = expect_error(
        "Hello",
        &ConvertOptions {
            style: Some(StyleOverrides {
                font_family: Some("   ".to_string()),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(
        err.to_string(),
        "Invalid fontFamily: Must be a non-empty string"
    );
}

// Jest: "builds distinct section properties, footers, and page numbering"
#[test]
fn distinct_section_footers_and_page_numbering() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            template: Some(SectionConfig {
                page_numbering: Some(PageNumberingInput {
                    display: Some(PageNumberDisplay::Current),
                    alignment: Some(Alignment::Right),
                    ..PageNumberingInput::default()
                }),
                ..SectionConfig::default()
            }),
            sections: Some(vec![
                section(
                    "# Cover\n\nCustom cover content.",
                    SectionConfig {
                        footers: Some(HeaderFooterGroup {
                            default: removed(),
                            ..HeaderFooterGroup::default()
                        }),
                        page_numbering: Some(PageNumberingInput {
                            display: Some(PageNumberDisplay::None),
                            ..PageNumberingInput::default()
                        }),
                        style: Some(StyleOverrides {
                            paragraph_alignment: Some(Alignment::Center),
                            ..StyleOverrides::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
                section(
                    "# Body\n\nMain content starts here.",
                    SectionConfig {
                        footers: Some(HeaderFooterGroup {
                            default: content(HeaderFooterContent {
                                text: Some("Page".to_string()),
                                page_number_display: Some(
                                    PageNumberDisplay::CurrentAndSectionTotal,
                                ),
                                alignment: Some(Alignment::Right),
                                ..HeaderFooterContent::default()
                            }),
                            ..HeaderFooterGroup::default()
                        }),
                        page_numbering: Some(PageNumberingInput {
                            start: Some(1),
                            format: Some(PageNumberFormat::Decimal),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    assert_texts(
        &doc,
        &[
            "Cover",
            "Custom cover content.",
            "Body",
            "Main content starts here.",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[1].align.as_deref(), Some("center"));
    assert_eq!(paragraphs[3].align.as_deref(), Some("left"));
    assert_eq!(doc.sections.len(), 2, "sections\n{}", doc.render());
    assert!(doc.sections[0].footer_refs.is_empty());
    assert_default_page(&doc.sections[0]);
    assert_eq!(doc.sections[1].footer_refs.len(), 1);
    assert_eq!(doc.sections[1].page_start, Some(1));
    assert_eq!(doc.sections[1].page_format.as_deref(), Some("decimal"));

    let footer = story(&doc, "footer1.xml");
    let footer_paragraphs = story_paragraphs(footer);
    assert_eq!(footer_paragraphs[0].align.as_deref(), Some("right"));
    assert_eq!(
        footer_paragraphs
            .iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Page  / "]
    );
    assert_eq!(story_fields(footer), ["PAGE", "SECTIONPAGES"]);
}

// Jest: "creates independent numbered-list references across sections"
#[test]
fn independent_numbered_lists_across_sections() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![
                section(
                    "1. First section item\n2. Second item",
                    SectionConfig::default(),
                ),
                section(
                    "1. New section item\n2. Another section item",
                    SectionConfig::default(),
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    assert_texts(
        &doc,
        &[
            "First section item",
            "Second item",
            "New section item",
            "Another section item",
        ],
    );
    let paragraphs = paragraphs(&doc);
    let first = paragraphs[0].list.as_ref().expect("first list");
    let second = paragraphs[1].list.as_ref().expect("second list");
    let third = paragraphs[2].list.as_ref().expect("third list");
    let fourth = paragraphs[3].list.as_ref().expect("fourth list");
    assert!(first.ordered && first.start == 1 && first.level == 0);
    assert_eq!(second.num_id, first.num_id);
    assert_eq!(second.start, 1);
    assert!(third.ordered && third.start == 1);
    assert_ne!(third.num_id, first.num_id);
    assert_eq!(fourth.num_id, third.num_id);
    assert_eq!(doc.sections.len(), 2);
}

// Jest: "applies template and section-level advanced properties, including section types and numbering variants"
#[test]
fn advanced_section_properties() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            template: Some(SectionConfig {
                page: Some(PageInput {
                    margin: Some(MarginInput {
                        top: Some(1600),
                        left: Some(1200),
                        ..MarginInput::default()
                    }),
                    ..PageInput::default()
                }),
                headers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        text: Some("Template Header".to_string()),
                        alignment: Some(Alignment::Left),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                page_numbering: Some(PageNumberingInput {
                    display: Some(PageNumberDisplay::Current),
                    alignment: Some(Alignment::Center),
                    ..PageNumberingInput::default()
                }),
                ..SectionConfig::default()
            }),
            sections: Some(vec![
                section(
                    "# Intro\n\nTemplate should apply here.",
                    SectionConfig {
                        title_page: Some(true),
                        break_type: Some(SectionBreak::OddPage),
                        page: Some(PageInput {
                            size: Some(SizeInput {
                                orientation: Some(Orientation::Landscape),
                                ..SizeInput::default()
                            }),
                            ..PageInput::default()
                        }),
                        headers: Some(HeaderFooterGroup {
                            first: content(HeaderFooterContent {
                                text: Some("First Page Header".to_string()),
                                alignment: Some(Alignment::Right),
                                ..HeaderFooterContent::default()
                            }),
                            ..HeaderFooterGroup::default()
                        }),
                        footers: Some(HeaderFooterGroup {
                            default: content(HeaderFooterContent {
                                text: Some("Custom Footer".to_string()),
                                page_number_display: Some(PageNumberDisplay::CurrentAndTotal),
                                alignment: Some(Alignment::Right),
                                ..HeaderFooterContent::default()
                            }),
                            ..HeaderFooterGroup::default()
                        }),
                        page_numbering: Some(PageNumberingInput {
                            start: Some(5),
                            format: Some(PageNumberFormat::UpperRoman),
                            separator: Some(PageNumberSeparator::Colon),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
                section(
                    "# B\n\nTwo",
                    SectionConfig {
                        break_type: Some(SectionBreak::Continuous),
                        page_numbering: Some(PageNumberingInput {
                            start: Some(1),
                            format: Some(PageNumberFormat::LowerRoman),
                            separator: Some(PageNumberSeparator::Period),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
                section(
                    "# C\n\nThree",
                    SectionConfig {
                        break_type: Some(SectionBreak::EvenPage),
                        page_numbering: Some(PageNumberingInput {
                            start: Some(1),
                            format: Some(PageNumberFormat::UpperLetter),
                            separator: Some(PageNumberSeparator::Hyphen),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    assert_texts(
        &doc,
        &[
            "Intro",
            "Template should apply here.",
            "B",
            "Two",
            "C",
            "Three",
        ],
    );
    assert_eq!(doc.sections.len(), 3, "sections\n{}", doc.render());

    let first = &doc.sections[0];
    assert_eq!(first.break_type.as_deref(), Some("oddPage"));
    assert!(first.title_page);
    assert_eq!(first.page_width, Some(16838));
    assert_eq!(first.page_height, Some(11906));
    assert_eq!(first.orientation.as_deref(), Some("landscape"));
    assert_eq!(first.margin_top, Some(1600));
    assert_eq!(first.margin_left, Some(1200));
    assert_eq!(first.margin_right, Some(1080));
    assert_eq!(first.margin_bottom, Some(1440));
    assert_eq!(first.page_start, Some(5));
    assert_eq!(first.page_format.as_deref(), Some("upperRoman"));
    assert_eq!(first.page_separator.as_deref(), Some("colon"));
    assert_eq!(
        first
            .header_refs
            .iter()
            .map(|reference| reference.kind.as_str())
            .collect::<Vec<_>>(),
        ["default", "first"]
    );
    assert_eq!(first.footer_refs.len(), 1);

    let second = &doc.sections[1];
    assert_eq!(second.break_type.as_deref(), Some("continuous"));
    assert!(!second.title_page);
    assert_eq!(second.page_start, Some(1));
    assert_eq!(second.page_format.as_deref(), Some("lowerRoman"));
    assert_eq!(second.page_separator.as_deref(), Some("period"));
    assert_eq!(second.margin_top, Some(1600));
    assert_eq!(second.margin_left, Some(1200));

    let third = &doc.sections[2];
    assert_eq!(third.break_type.as_deref(), Some("evenPage"));
    assert_eq!(third.page_start, Some(1));
    assert_eq!(third.page_format.as_deref(), Some("upperLetter"));
    assert_eq!(third.page_separator.as_deref(), Some("hyphen"));

    for (file, text) in [
        ("header1.xml", "Template Header"),
        ("header3.xml", "Template Header"),
        ("header4.xml", "Template Header"),
    ] {
        let paragraphs = story_paragraphs(story(&doc, file));
        assert_eq!(paragraphs[0].text, text);
        assert_eq!(paragraphs[0].align.as_deref(), Some("left"));
    }
    let first_page = story_paragraphs(story(&doc, "header2.xml"));
    assert_eq!(first_page[0].text, "First Page Header");
    assert_eq!(first_page[0].align.as_deref(), Some("right"));

    let custom = story_paragraphs(story(&doc, "footer1.xml"));
    assert_eq!(custom[0].align.as_deref(), Some("right"));
    assert!(custom
        .iter()
        .any(|paragraph| paragraph.text.contains("Custom Footer")));
    assert!(custom
        .iter()
        .any(|paragraph| paragraph.text.contains(" / ")));
    assert_eq!(
        story_fields(story(&doc, "footer1.xml")),
        ["PAGE", "NUMPAGES"]
    );
    for file in ["footer2.xml", "footer3.xml"] {
        let paragraphs = story_paragraphs(story(&doc, file));
        assert_eq!(paragraphs[0].align.as_deref(), Some("center"));
        assert_eq!(story_fields(story(&doc, file)), ["PAGE"]);
        assert!(paragraphs[0].text.is_empty());
    }
}

// Jest: "supports per-section style overrides in full conversion"
#[test]
fn per_section_paragraph_size() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            style: Some(StyleOverrides {
                paragraph_size: Some(24),
                ..StyleOverrides::default()
            }),
            sections: Some(vec![
                section(
                    "# Section One\n\nSmaller text section.",
                    SectionConfig {
                        style: Some(StyleOverrides {
                            paragraph_size: Some(20),
                            ..StyleOverrides::default()
                        }),
                        page_numbering: Some(PageNumberingInput {
                            display: Some(PageNumberDisplay::None),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
                section(
                    "# Section Two\n\nLarger text section.",
                    SectionConfig {
                        style: Some(StyleOverrides {
                            paragraph_size: Some(30),
                            ..StyleOverrides::default()
                        }),
                        page_numbering: Some(PageNumberingInput {
                            start: Some(1),
                            ..PageNumberingInput::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    assert_texts(
        &doc,
        &[
            "Section One",
            "Smaller text section.",
            "Section Two",
            "Larger text section.",
        ],
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].heading.as_deref(), Some("Heading1"));
    assert_eq!(run_named(paragraphs[0], "Section One").size, Some(32));
    assert_eq!(
        run_named(paragraphs[1], "Smaller text section.").size,
        Some(20)
    );
    assert_eq!(run_named(paragraphs[2], "Section Two").size, Some(32));
    assert_eq!(
        run_named(paragraphs[3], "Larger text section.").size,
        Some(30)
    );
    assert_eq!(paragraphs[0].bookmarks, ["_Toc_Section_One_1"]);
    assert_eq!(paragraphs[2].bookmarks, ["_Toc_Section_Two_2"]);
    assert!(doc.sections[0].footer_refs.is_empty());
    assert_eq!(doc.sections[1].footer_refs.len(), 1);
    assert_eq!(doc.sections[1].page_start, Some(1));
    assert!(doc.sections[1].page_format.is_none());
}

// Plan: precedence is global style, then template.style, then the section style.
// Top-level markdown is ignored once `sections` is set.
#[test]
fn style_precedence_global_template_section() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            style: Some(StyleOverrides {
                paragraph_alignment: Some(Alignment::Left),
                paragraph_size: Some(22),
                ..StyleOverrides::default()
            }),
            template: Some(SectionConfig {
                style: Some(StyleOverrides {
                    paragraph_alignment: Some(Alignment::Center),
                    ..StyleOverrides::default()
                }),
                ..SectionConfig::default()
            }),
            sections: Some(vec![
                section("Template wins alignment.", SectionConfig::default()),
                section(
                    "Section wins.",
                    SectionConfig {
                        style: Some(StyleOverrides {
                            paragraph_alignment: Some(Alignment::Right),
                            paragraph_size: Some(28),
                            ..StyleOverrides::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    assert_texts(&doc, &["Template wins alignment.", "Section wins."]);
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].align.as_deref(), Some("center"));
    assert_eq!(
        run_named(paragraphs[0], "Template wins alignment.").size,
        Some(22)
    );
    assert_eq!(paragraphs[1].align.as_deref(), Some("right"));
    assert_eq!(run_named(paragraphs[1], "Section wins.").size, Some(28));
    assert!(!doc.paragraph_texts().contains(&"ignored"));
}

// Plan: empty body content becomes one empty paragraph.
#[test]
fn empty_section_body_is_one_paragraph() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![section("", SectionConfig::default())]),
            ..ConvertOptions::default()
        },
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs.len(), 1);
    assert!(paragraphs[0].text.is_empty());
    assert_eq!(doc.sections.len(), 1);
    assert_default_page(&doc.sections[0]);
    assert_eq!(doc.sections[0].footer_refs.len(), 1);
}

// Landscape swaps the default edges. The longer edge is the width.
#[test]
fn landscape_swaps_page_edges() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![section(
                "# Wide",
                SectionConfig {
                    page: Some(PageInput {
                        size: Some(SizeInput {
                            orientation: Some(Orientation::Landscape),
                            ..SizeInput::default()
                        }),
                        ..PageInput::default()
                    }),
                    ..SectionConfig::default()
                },
            )]),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(paragraphs(&doc)[0].text, "Wide");
    assert_eq!(paragraphs(&doc)[0].bookmarks, ["_Toc_Wide_1"]);
    let section = &doc.sections[0];
    assert_eq!(section.page_width, Some(16838));
    assert_eq!(section.page_height, Some(11906));
    assert_eq!(section.orientation.as_deref(), Some("landscape"));
    assert_eq!(section.margin_top, Some(1440));
    assert_eq!(section.margin_right, Some(1080));
}

// Jest: "generates deterministic unique bookmark names across sections"
#[test]
fn bookmark_names_across_sections() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![
                section("# Repeat", SectionConfig::default()),
                section("# Repeat", SectionConfig::default()),
            ]),
            ..ConvertOptions::default()
        },
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].bookmarks, ["_Toc_Repeat_1"]);
    assert_eq!(paragraphs[1].bookmarks, ["_Toc_Repeat_2"]);
    assert_eq!(doc.sections.len(), 2);
}

// Plan: a footnote in a header or footer throws this message.
#[test]
fn footnote_in_header_or_footer_is_rejected() {
    let note = "See this[^n].\n\n[^n]: The note.";
    let header = expect_error(
        "# Body",
        &ConvertOptions {
            template: Some(SectionConfig {
                headers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some(note.to_string()),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                ..SectionConfig::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let footer = expect_error(
        "# Body",
        &ConvertOptions {
            template: Some(SectionConfig {
                footers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some(note.to_string()),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                ..SectionConfig::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(
        header.to_string(),
        "Word headers and footers cannot contain footnotes"
    );
    assert_eq!(
        footer.to_string(),
        "Word headers and footers cannot contain footnotes"
    );
}

// Jest: "emits a TOC field and an explicit page break"
#[test]
fn toc_field_and_page_break() {
    let doc = convert(
        "[TOC]\n\n# Section 1\n\nContent.\n\n\\pagebreak\n\n# Section 2\n\nMore content.",
        &ConvertOptions::default(),
    );
    let paragraphs = paragraphs(&doc);
    assert_eq!(paragraphs[0].text, "Table of Contents");
    assert_eq!(paragraphs[0].style_id.as_deref(), Some("TOCHeading"));
    assert_eq!(paragraphs[0].align.as_deref(), Some("center"));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " TOC \\o \"1-6\" \\h \\z "));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " PAGEREF _Toc_Section_1_1 \\h "));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " PAGEREF _Toc_Section_2_2 \\h "));
    assert_eq!(
        body_field_chars(&doc),
        [
            "begin".to_string(),
            "separate".to_string(),
            "end".to_string()
        ]
    );
    assert!(doc.update_fields);
    let leaders: Vec<_> = paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.tabs.iter())
        .map(|tab| (tab.val.as_str(), tab.pos.as_str(), tab.leader.as_deref()))
        .collect();
    assert!(leaders.contains(&("right", "9746", Some("dot"))));

    let break_at = paragraphs
        .iter()
        .position(|paragraph| paragraph.page_break)
        .unwrap_or_else(|| panic!("missing page break\n{}", doc.render()));
    assert!(paragraphs[break_at].text.is_empty());
    let headings: Vec<_> = paragraphs
        .iter()
        .filter(|paragraph| paragraph.heading.as_deref() == Some("Heading1"))
        .collect();
    assert_eq!(headings[0].text, "Section 1");
    assert_eq!(headings[0].bookmarks, ["_Toc_Section_1_1"]);
    assert_eq!(headings[1].text, "Section 2");
    assert_eq!(headings[1].bookmarks, ["_Toc_Section_2_2"]);
    let content_at = paragraphs
        .iter()
        .position(|paragraph| paragraph.text == "Content.")
        .unwrap();
    let more_at = paragraphs
        .iter()
        .position(|paragraph| paragraph.text == "More content.")
        .unwrap();
    assert!(content_at < break_at && break_at < more_at);
}

// Jest: "supports custom TOC title and depth filtering"
#[test]
fn toc_custom_title_and_depth() {
    let doc = convert(
        "[TOC]\n\n# Hidden H1\n\n## Included H2\n\n### Included H3\n\n#### Hidden H4",
        &ConvertOptions {
            toc: Some(TocOverrides {
                title: Some("Contents".to_string()),
                min_depth: Some(2),
                max_depth: Some(3),
                ..TocOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(paragraphs(&doc)[0].text, "Contents");
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " TOC \\o \"2-3\" \\h \\z "));
    assert_eq!(
        doc.paragraphs()
            .iter()
            .filter(|paragraph| paragraph.text.contains("Included H2"))
            .count(),
        2
    );
    assert_eq!(
        doc.paragraphs()
            .iter()
            .filter(|paragraph| paragraph.text.contains("Included H3"))
            .count(),
        2
    );
    assert_eq!(
        doc.paragraphs()
            .iter()
            .filter(|paragraph| paragraph.text.contains("Hidden H1"))
            .count(),
        1
    );
    assert_eq!(
        doc.paragraphs()
            .iter()
            .filter(|paragraph| paragraph.text.contains("Hidden H4"))
            .count(),
        1
    );
    let cached_h2 = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| {
            paragraph.text.contains("Included H2")
                && paragraph
                    .fields
                    .iter()
                    .any(|field| field.contains("PAGEREF"))
        })
        .expect("cached H2");
    assert_eq!(run_named(cached_h2, "Included H2").size, Some(22));
    let cached_h3 = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| {
            paragraph.text.contains("Included H3")
                && paragraph
                    .fields
                    .iter()
                    .any(|field| field.contains("PAGEREF"))
        })
        .expect("cached H3");
    assert_eq!(run_named(cached_h3, "Included H3").size, Some(20));
    let body_h1 = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.heading.as_deref() == Some("Heading1"))
        .unwrap();
    assert_eq!(run_named(body_h1, "Hidden H1").size, Some(32));
    assert_eq!(
        doc.paragraphs()
            .iter()
            .flat_map(|paragraph| paragraph.bookmarks.iter())
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "_Toc_Hidden_H1_1",
            "_Toc_Included_H2_2",
            "_Toc_Included_H3_3",
            "_Toc_Hidden_H4_4",
        ]
    );
    assert!(doc.update_fields);
}

// Jest: "emits native TOC fields, cached links, page references and leader tabs"
#[test]
fn native_toc_fields_and_link_mode() {
    let doc = convert("[TOC]\n\n# One\n\n## Two", &ConvertOptions::default());
    assert_eq!(paragraphs(&doc)[0].text, "Table of Contents");
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " TOC \\o \"1-6\" \\h \\z "));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " PAGEREF _Toc_One_1 \\h "));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " PAGEREF _Toc_Two_2 \\h "));
    assert_eq!(
        body_field_chars(&doc),
        [
            "begin".to_string(),
            "separate".to_string(),
            "end".to_string()
        ]
    );
    let cached = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.text.contains("One") && paragraph.text.contains('?'))
        .expect("cached One");
    assert_eq!(
        run_named(cached, "One").anchor.as_deref(),
        Some("_Toc_One_1")
    );
    assert_eq!(run_named(cached, "One").size, Some(24));
    assert_eq!(run_named(cached, "?").text, "?");
    let leaders: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.tabs.iter())
        .filter_map(|tab| tab.leader.clone())
        .collect();
    assert!(leaders.iter().any(|leader| leader == "dot"));
    assert!(doc.update_fields);
    assert_eq!(
        doc.paragraphs()
            .iter()
            .filter(|paragraph| paragraph.heading.as_deref() == Some("Heading1"))
            .map(|paragraph| paragraph.bookmarks.as_slice())
            .collect::<Vec<_>>(),
        [vec!["_Toc_One_1".to_string()].as_slice()]
    );

    let links = convert(
        "[TOC]\n\n# One",
        &ConvertOptions {
            toc: Some(TocOverrides {
                mode: Some(TocMode::Links),
                ..TocOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert!(!body_fields(&links)
        .iter()
        .any(|field| field.contains("PAGEREF")));
    assert!(body_field_chars(&links).is_empty());
    assert!(!links.update_fields);
    let linked = links
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.runs.iter().any(|run| run.anchor.is_some()))
        .expect("static toc link");
    assert_eq!(
        run_named(linked, "One").anchor.as_deref(),
        Some("_Toc_One_1")
    );
    assert!(linked
        .tabs
        .iter()
        .any(|tab| tab.leader.as_deref() == Some("dot")));
}

// Jest: "emits native TOC fields, cached links, page references and leader tabs"
//
// `update_fields_on_open: false` keeps the TOC field and omits `w:updateFields`.
#[test]
fn toc_update_fields_on_open_can_be_disabled() {
    let doc = convert(
        "[TOC]\n\n# One",
        &ConvertOptions {
            toc: Some(TocOverrides {
                update_fields_on_open: Some(false),
                ..TocOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert!(body_fields(&doc).iter().any(|field| field.contains("TOC")));
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field.contains("PAGEREF")));
    assert!(!doc.update_fields);
}

// Jest: "emits native TOC fields, cached links, page references and leader tabs"
//
// `page_numbers: false` and `dot_leaders: false` add `\n` and a leader of `none`.
#[test]
fn toc_without_page_numbers_or_dot_leaders() {
    let doc = convert(
        "[TOC]\n\n# One\n\n## Two",
        &ConvertOptions {
            toc: Some(TocOverrides {
                page_numbers: Some(false),
                dot_leaders: Some(false),
                ..TocOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert!(body_fields(&doc)
        .iter()
        .any(|field| field == " TOC \\o \"1-6\" \\h \\z \\n "));
    assert!(!body_fields(&doc)
        .iter()
        .any(|field| field.contains("PAGEREF")));
    assert!(doc.update_fields);
    let leaders: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.tabs.iter())
        .filter_map(|tab| tab.leader.clone())
        .collect();
    assert!(leaders.iter().all(|leader| leader == "none"));
    assert!(!leaders.is_empty());
}

// Jest: "resolves header and footer links to body headings across sections"
#[test]
fn header_and_footer_links_resolve_across_sections() {
    let doc = convert(
        "ignored",
        &ConvertOptions {
            sections: Some(vec![
                section("# Body", SectionConfig::default()),
                section("# Body\n\n# 重复", SectionConfig::default()),
            ]),
            template: Some(SectionConfig {
                headers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some("[Next](#body-1)".to_string()),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                footers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some("[Unicode](#%E9%87%8D%E5%A4%8D)".to_string()),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                ..SectionConfig::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let bookmarks: Vec<_> = doc
        .paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.bookmarks.iter().cloned())
        .collect();
    assert_eq!(
        bookmarks,
        [
            "_Toc_Body_1".to_string(),
            "_Toc_Body_2".to_string(),
            "_Toc___3".to_string(),
        ]
    );
    for file in ["header1.xml", "header2.xml"] {
        let paragraphs = story_paragraphs(story(&doc, file));
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(
            run_named(paragraphs[0], "Next").anchor.as_deref(),
            Some("_Toc_Body_2")
        );
    }
    for file in ["footer1.xml", "footer2.xml"] {
        let paragraphs = story_paragraphs(story(&doc, file));
        let unicode = paragraphs
            .iter()
            .find(|paragraph| paragraph.text.contains("Unicode"))
            .unwrap_or_else(|| panic!("missing unicode link in {file}\n{}", doc.render()));
        assert_eq!(
            run_named(unicode, "Unicode").anchor.as_deref(),
            Some("_Toc___3")
        );
    }
}

// Jest: "renders header Markdown tables and logos alongside footer fields"
#[test]
fn header_markdown_and_footer_fields() {
    let doc = convert(
        "# Body",
        &ConvertOptions {
            template: Some(SectionConfig {
                headers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some(format!(
                            "**Company**\n\n![Logo]({ONE_PX_PNG})\n\n| A | B |\n|---|---|\n| a | b |"
                        )),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                footers: Some(HeaderFooterGroup {
                    default: content(HeaderFooterContent {
                        markdown: Some("*Confidential*".to_string()),
                        page_number_display: Some(PageNumberDisplay::CurrentAndTotal),
                        ..HeaderFooterContent::default()
                    }),
                    ..HeaderFooterGroup::default()
                }),
                ..SectionConfig::default()
            }),
            ..ConvertOptions::default()
        },
    );
    assert_eq!(paragraphs(&doc)[0].text, "Body");
    let header = story(&doc, "header1.xml");
    assert_eq!(header.drawing_count, 1);
    assert!(
        doc.media_parts
            .iter()
            .any(|part| part.starts_with("word/media/") && part.ends_with(".png")),
        "header logo media\n{}",
        doc.render()
    );
    let header_paragraphs = story_paragraphs(header);
    let company = header_paragraphs
        .iter()
        .find(|paragraph| paragraph.text.contains("Company"))
        .expect("company");
    assert!(run_named(company, "Company").bold);
    let table = header
        .blocks
        .iter()
        .find_map(|block| match block {
            common::Block::Table(table) => Some(table),
            common::Block::Paragraph(_) => None,
        })
        .expect("header table");
    assert_eq!(table.columns, 2);
    assert_eq!(table.rows[0][0].paragraphs[0].text, "A");
    assert_eq!(table.rows[0][1].paragraphs[0].text, "B");
    assert_eq!(table.rows[1][0].paragraphs[0].text, "a");
    assert_eq!(table.rows[1][1].paragraphs[0].text, "b");

    let footer = story(&doc, "footer1.xml");
    let footer_paragraphs = story_paragraphs(footer);
    let confidential = footer_paragraphs
        .iter()
        .find(|paragraph| paragraph.text.contains("Confidential"))
        .expect("confidential");
    assert!(run_named(confidential, "Confidential").italic);
    assert!(
        confidential.text.contains(" / ")
            || footer_paragraphs
                .iter()
                .any(|paragraph| paragraph.text.contains(" / "))
    );
    assert_eq!(story_fields(footer), ["PAGE", "NUMPAGES"]);
}
