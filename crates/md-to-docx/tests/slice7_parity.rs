//! Slice 7 parity with code highlighting, rich tables, text replacements, and
//! processing limits.
//!
//! ZIP bytes are not compared. Highlighting checks the plain Courier path
//! against a themed run. Token colors are not required to match lowlight.
//!
//! Jest cases that need a lowlight token hex, or an `AbortSignal`, are not
//! sent. `ConvertOptions` has no signal field. The alias-whitelist table in
//! `tests/code-highlighting.test.ts` only passes when a token color appears.

#[path = "common/mod.rs"]
mod common;

use common::{Document, Paragraph, Run, Table};

use md_to_docx::options::{
    CodeHighlightTheme, CodeHighlightingOptions, DocumentSection, HeaderFooterContent,
    HeaderFooterGroup, HeaderFooterSlot, OptionBool, SectionConfig, TextFind, TextReplace,
    TextReplacement, TextReplacementFunction, TextReplacementMode,
};
use md_to_docx::ConvertOptions;

const JS_MARKDOWN: &str = "```javascript
const x = 1;
function greet(name) {
  return \"Hello, \" + name;
}
```";

const SPANS: &str = r#"{"columnWidths":[2000,4000],"headers":["Name","Content"],"rows":[[{"markdown":"Merged **row**","rowSpan":2},"1. First\n2. Second"],["Second row"],[{"markdown":"Spans both","columnSpan":2}]]}"#;

fn convert(markdown: &str) -> Document {
    project(
        &md_to_docx::convert_markdown(markdown)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn convert_with(markdown: &str, options: &ConvertOptions) -> Document {
    project(
        &md_to_docx::convert_with_options(markdown, options)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn project(bytes: &[u8]) -> Document {
    assert!(
        bytes.starts_with(b"PK"),
        "conversion should return a zip package"
    );
    common::project_docx(bytes)
}

fn expect_error(markdown: &str, options: &ConvertOptions) -> String {
    md_to_docx::convert_with_options(markdown, options)
        .expect_err("conversion should fail")
        .to_string()
}

fn utf16_units(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}

fn highlighting(enabled: bool) -> ConvertOptions {
    ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(enabled)),
            ..CodeHighlightingOptions::default()
        }),
        ..ConvertOptions::default()
    }
}

fn code_paragraph<'a>(doc: &'a Document, needle: &str) -> &'a Paragraph {
    doc.paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.text.replace('\u{a0}', " ").contains(needle))
        .unwrap_or_else(|| panic!("no paragraph contains {needle:?}\n{}", doc.render()))
}

fn plain_colors(paragraph: &Paragraph) {
    for run in &paragraph.runs {
        if let Some(color) = &run.color {
            assert!(
                color == "444444" || color == "666666",
                "plain code picked up color {color} on {:?}",
                run.text
            );
        }
    }
}

fn label_run<'a>(paragraph: &'a Paragraph, language: &str) -> &'a Run {
    paragraph
        .runs
        .iter()
        .find(|run| run.text.trim_matches(['\n', '\r']) == language)
        .unwrap_or_else(|| panic!("missing language label {language}\n{paragraph:?}"))
}

fn whole_line<'a>(paragraph: &'a Paragraph, line: &str) -> &'a Run {
    paragraph
        .runs
        .iter()
        .find(|run| run.text.contains(line))
        .unwrap_or_else(|| panic!("line was split: {line:?}\n{paragraph:?}"))
}

fn joined(doc: &Document) -> String {
    doc.paragraph_texts().join("\n")
}

fn cells(table: &Table) -> Vec<&common::Cell> {
    table.rows.iter().flatten().collect()
}

fn cell_text(cell: &common::Cell) -> String {
    cell.paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn deepest_text(table: &Table) -> String {
    let nested = table
        .rows
        .iter()
        .flatten()
        .flat_map(|cell| cell.tables.first());
    if let Some(inner) = nested.into_iter().next() {
        return deepest_text(inner);
    }
    table
        .rows
        .iter()
        .flatten()
        .flat_map(|cell| cell.paragraphs.iter())
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn nest_tables(levels: usize, leaf: &str) -> String {
    let mut markdown = leaf.to_string();
    for _ in 0..levels {
        let escaped = markdown
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        markdown = format!("```table\n{{\"rows\":[[{{\"markdown\":\"{escaped}\"}}]]}}\n```");
    }
    markdown
}

fn literal_replacement(find: &str, replace: &str) -> TextReplacement {
    TextReplacement {
        find: TextFind::Literal(find.to_string()),
        replace: TextReplace::Text(replace.to_string()),
    }
}

fn limits(max_input_length: Option<u32>, max_elements: Option<u32>) -> ConvertOptions {
    ConvertOptions {
        max_input_length,
        max_elements,
        ..ConvertOptions::default()
    }
}

fn section_options(sections: Vec<DocumentSection>, max_input_length: u32) -> ConvertOptions {
    ConvertOptions {
        sections: Some(sections),
        max_input_length: Some(max_input_length),
        ..ConvertOptions::default()
    }
}

fn one_section(markdown: &str, config: SectionConfig) -> DocumentSection {
    DocumentSection {
        markdown: markdown.to_string(),
        config,
    }
}

// Jest: "does not emit per-token colors when codeHighlighting is absent"
#[test]
fn highlighting_absent_stays_plain() {
    let doc = convert(JS_MARKDOWN);
    let paragraph = code_paragraph(&doc, "const x = 1;");
    plain_colors(paragraph);
    let label = label_run(paragraph, "javascript");
    assert!(label.bold && label.code);
    assert_eq!(label.color.as_deref(), Some("666666"));
    let body = whole_line(paragraph, "const x = 1;");
    assert!(body.code);
    assert_eq!(body.color.as_deref(), Some("444444"));
    assert_eq!(paragraph.shade.as_deref(), Some("F5F5F5"));
}

// Jest: "does not emit per-token colors when enabled is false"
#[test]
fn highlighting_disabled_stays_plain() {
    let doc = convert_with(JS_MARKDOWN, &highlighting(false));
    let paragraph = code_paragraph(&doc, "const x = 1;");
    plain_colors(paragraph);
    assert_eq!(
        label_run(paragraph, "javascript").color.as_deref(),
        Some("666666")
    );
    assert_eq!(
        whole_line(paragraph, "const x = 1;").color.as_deref(),
        Some("444444")
    );
    assert_eq!(paragraph.shade.as_deref(), Some("F5F5F5"));
}

// Jest: "emits keyword, number, and string colors for a JS snippet"
// Jest: "respects the default language label color"
//
// The label and background come from the theme. Body runs must leave the
// plain 444444 color. The lowlight hex values are not required.
#[test]
fn highlighting_on_uses_the_theme_instead_of_plain_grey() {
    let doc = convert_with(JS_MARKDOWN, &highlighting(true));
    let paragraph = code_paragraph(&doc, "const");
    assert!(paragraph.text.contains('1'));
    assert!(paragraph.text.contains("Hello, "));
    assert_eq!(
        label_run(paragraph, "javascript").color.as_deref(),
        Some("6A737D")
    );
    assert_eq!(
        paragraph
            .shade
            .as_deref()
            .map(str::to_ascii_uppercase)
            .as_deref(),
        Some("FFFFFF")
    );
    assert!(
        paragraph.runs.iter().any(|run| {
            run.text.contains("const")
                && run
                    .color
                    .as_deref()
                    .is_some_and(|color| color != "444444" && color != "666666")
        }),
        "body stayed on the plain code color\n{}",
        doc.render()
    );
}

// Jest: "omits the language label when showLanguageLabel is false"
#[test]
fn highlighting_can_omit_the_language_label() {
    let markdown = "```javascript\nconst y = 2;\n```";
    let options = ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(true)),
            show_language_label: Some(OptionBool::Bool(false)),
            ..CodeHighlightingOptions::default()
        }),
        ..ConvertOptions::default()
    };
    let doc = convert_with(markdown, &options);
    let paragraph = code_paragraph(&doc, "const y = 2;");
    assert!(
        paragraph
            .runs
            .iter()
            .all(|run| run.text.trim_matches(['\n', '\r']) != "javascript"),
        "label still rendered\n{}",
        doc.render()
    );
}

// Jest: "does not throw and keeps the plain-grey run output"
#[test]
fn unknown_language_stays_one_plain_run() {
    let markdown = "```doesnotexist\nkeyword value 1 2 3\n```";
    let doc = convert_with(markdown, &highlighting(true));
    let paragraph = code_paragraph(&doc, "keyword value 1 2 3");
    let line = whole_line(paragraph, "keyword value 1 2 3");
    assert!(line.code);
    assert!(
        paragraph
            .runs
            .iter()
            .filter(|run| run.text.contains("keyword"))
            .count()
            == 1,
        "unknown language was tokenized\n{}",
        doc.render()
    );
}

// Jest: "does not throw when the code block has no language at all"
#[test]
fn missing_language_stays_plain_when_highlighting_is_on() {
    let markdown = "```\nconst plain = true;\n```";
    let doc = convert_with(markdown, &highlighting(true));
    let paragraph = code_paragraph(&doc, "const plain = true");
    let line = whole_line(paragraph, "const plain = true");
    assert!(line.code);
    assert_ne!(line.color.as_deref(), Some("D73A49"));
}

// Jest: "uses a user-supplied keyword color over the default"
//
// Only the background key is asserted. A keyword hex would require lowlight.
#[test]
fn highlight_theme_background_overrides_the_code_shade() {
    let options = ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(true)),
            theme: Some(CodeHighlightTheme {
                background: Some("101010".to_string()),
                ..CodeHighlightTheme::default()
            }),
            ..CodeHighlightingOptions::default()
        }),
        ..ConvertOptions::default()
    };
    let doc = convert_with(JS_MARKDOWN, &options);
    let paragraph = code_paragraph(&doc, "const");
    assert_eq!(
        paragraph
            .shade
            .as_deref()
            .map(str::to_ascii_uppercase)
            .as_deref(),
        Some("101010")
    );
}

// Jest: "keeps explicit line breaks in multi-line highlighted code"
#[test]
fn highlighted_code_keeps_line_breaks() {
    let doc = convert_with(JS_MARKDOWN, &highlighting(true));
    let paragraph = code_paragraph(&doc, "const");
    let breaks = paragraph.text.chars().filter(|ch| *ch == '\n').count();
    assert!(breaks >= 3, "breaks {breaks}\n{}", doc.render());
}

// Jest: "falls back to plain rendering when the language is not in the whitelist"
#[test]
fn language_outside_the_whitelist_stays_plain() {
    let markdown = "```python\ndef greet(name):\n    return \"Hello, \" + name\n```";
    let options = ConvertOptions {
        code_highlighting: Some(CodeHighlightingOptions {
            enabled: Some(OptionBool::Bool(true)),
            languages: Some(vec!["javascript".to_string()]),
            ..CodeHighlightingOptions::default()
        }),
        ..ConvertOptions::default()
    };
    let doc = convert_with(markdown, &options);
    let paragraph = code_paragraph(&doc, "def greet(name):");
    let line = whole_line(paragraph, "def greet(name):");
    assert!(line.code);
}

// Jest: "renders merged cells, lists and images with explicit widths and row controls"
//
// The image half of that Jest case needs a PNG data URL and is not sent here.
#[test]
fn rich_table_spans_tile_the_grid() {
    let doc = convert(&format!("```table\n{SPANS}\n```"));
    let table = doc
        .tables()
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("table fence stayed code\n{}", doc.render()));
    assert_eq!(table.column_widths, ["2000", "4000"]);
    let spanned = cells(table)
        .into_iter()
        .find(|cell| cell.grid_span == Some(2))
        .unwrap_or_else(|| panic!("missing gridSpan\n{}", doc.render()));
    assert!(cell_text(spanned).contains("Spans both"));
    let restart = cells(table)
        .into_iter()
        .find(|cell| cell.v_merge.as_deref() == Some("restart"))
        .unwrap_or_else(|| panic!("missing vMerge restart\n{}", doc.render()));
    assert!(cell_text(restart).contains("Merged"));
    assert!(
        cells(table)
            .iter()
            .any(|cell| cell.v_merge.as_deref() == Some("continue")),
        "missing vMerge continue\n{}",
        doc.render()
    );
    assert!(
        restart.paragraphs.iter().any(|paragraph| {
            paragraph
                .runs
                .iter()
                .any(|run| run.bold && run.text.contains("row"))
        }),
        "merged markdown was not bold\n{}",
        doc.render()
    );
    assert!(
        cells(table).iter().any(|cell| {
            cell.paragraphs
                .iter()
                .any(|paragraph| paragraph.list.is_some() && paragraph.text.contains("First"))
        }),
        "cell list missing\n{}",
        doc.render()
    );
}

// Jest: "rejects invalid/overlapping spans and mismatched widths"
#[test]
fn rich_table_span_past_the_grid_throws() {
    let err = expect_error(
        "```table\n{\"rows\":[[{\"markdown\":\"A\",\"rowSpan\":3}]]}\n```",
        &ConvertOptions::default(),
    );
    assert!(err.contains("bounds"), "{err}");
}

// Slice 7 plan: spans must tile the row.
#[test]
fn rich_table_short_row_throws() {
    let err = expect_error(
        "```table\n{\"rows\":[[\"A\",\"B\"],[\"only\"]]}\n```",
        &ConvertOptions::default(),
    );
    assert!(
        err.contains("Table rows must cover the same number of columns"),
        "{err}"
    );
}

// Slice 7 plan: invalid JSON throws.
#[test]
fn invalid_table_json_throws() {
    let err = expect_error("```table\n{not json}\n```", &ConvertOptions::default());
    assert!(err.contains("Invalid table fence JSON"), "{err}");
}

// Slice 7 plan: nesting stops at 8 levels.
#[test]
fn eight_nested_tables_keep_the_leaf() {
    let doc = convert(&nest_tables(8, "Leaf"));
    let table = doc
        .tables()
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("nested table stayed code\n{}", doc.render()));
    assert_eq!(deepest_text(table), "Leaf", "{}", doc.render());
}

// Slice 7 plan: nesting stops at 8 levels.
#[test]
fn ninth_nested_table_throws() {
    let err = expect_error(&nest_tables(9, "Leaf"), &ConvertOptions::default());
    assert!(err.contains("Rich table nesting limit exceeded"), "{err}");
}

// Jest: "allows literal text replacements in untrusted mode"
#[test]
fn untrusted_literal_replacement_rewrites_text() {
    let options = ConvertOptions {
        text_replacement_mode: Some(TextReplacementMode::Untrusted),
        text_replacements: Some(vec![literal_replacement("Company Name", "Acme Corp")]),
        ..ConvertOptions::default()
    };
    let doc = convert_with("Company Name", &options);
    let text = joined(&doc);
    assert!(text.contains("Acme Corp"), "{text}");
    assert!(!text.contains("Company Name"), "{text}");
}

// Jest: "rejects function text replacements in untrusted mode before execution"
#[test]
fn untrusted_function_replacement_throws() {
    let options = ConvertOptions {
        text_replacement_mode: Some(TextReplacementMode::Untrusted),
        text_replacements: Some(vec![TextReplacement {
            find: TextFind::Regex(r"(\d+)".to_string()),
            replace: TextReplace::Function(TextReplacementFunction::new(|matched: &str| {
                format!("Number: {matched}")
            })),
        }]),
        ..ConvertOptions::default()
    };
    let err = expect_error("Invoice 42", &options);
    assert!(
        err.contains("Function textReplacements are not allowed"),
        "{err}"
    );
}

// Jest: "rejects nested-quantifier text replacement regexes before replacement"
#[test]
fn nested_quantifier_regex_is_rejected() {
    let err = expect_error("aaaa", &regex_options(r"(a+)+$", "safe"));
    assert!(
        err.contains("Unsafe textReplacements RegExp rejected"),
        "{err}"
    );
}

// Jest: "rejects nested optional-quantifier text replacement regexes"
#[test]
fn nested_optional_quantifier_regex_is_rejected() {
    let err = expect_error("aaaa", &regex_options(r"(a?)+$", "safe"));
    assert!(
        err.contains("Unsafe textReplacements RegExp rejected"),
        "{err}"
    );
}

// Slice 7 plan: a pattern the regex crate cannot compile uses the unsafe-pattern error.
#[test]
fn backreference_regex_is_rejected() {
    let err = expect_error("1212", &regex_options(r"(\d+)\1", "safe"));
    assert!(
        err.contains("Unsafe textReplacements RegExp rejected"),
        "{err}"
    );
}

// Slice 7 plan: at most 50 entries.
#[test]
fn fifty_replacements_are_accepted() {
    let options = ConvertOptions {
        text_replacements: Some(
            (0..50)
                .map(|index| literal_replacement(&format!("T{index}"), "X"))
                .collect(),
        ),
        ..ConvertOptions::default()
    };
    let doc = convert_with("T0", &options);
    assert!(joined(&doc).contains('X'), "{}", doc.render());
    assert!(!joined(&doc).contains("T0"), "{}", doc.render());
}

// Slice 7 plan: at most 50 entries.
#[test]
fn fifty_first_replacement_throws() {
    let options = ConvertOptions {
        text_replacements: Some(
            (0..51)
                .map(|index| literal_replacement(&format!("T{index}"), "X"))
                .collect(),
        ),
        ..ConvertOptions::default()
    };
    let err = expect_error("T0", &options);
    assert!(
        err.contains("textReplacements supports at most 50 entries"),
        "{err}"
    );
}

// Slice 7 plan: pattern length at most 256.
#[test]
fn replacement_pattern_of_256_is_accepted() {
    let pattern = "a".repeat(256);
    let options = ConvertOptions {
        text_replacements: Some(vec![literal_replacement(&pattern, "OK")]),
        ..ConvertOptions::default()
    };
    let doc = convert_with(&pattern, &options);
    assert!(joined(&doc).contains("OK"), "{}", doc.render());
}

// Slice 7 plan: pattern length at most 256.
#[test]
fn replacement_pattern_of_257_throws() {
    let options = ConvertOptions {
        text_replacements: Some(vec![literal_replacement(&"a".repeat(257), "OK")]),
        ..ConvertOptions::default()
    };
    let err = expect_error("a", &options);
    assert!(
        err.contains("textReplacements string pattern is too large"),
        "{err}"
    );
}

// Slice 7 plan: replacement text at most 4096.
#[test]
fn replacement_text_of_4096_is_accepted() {
    let replacement = "b".repeat(4096);
    let options = ConvertOptions {
        text_replacements: Some(vec![literal_replacement("Hi", &replacement)]),
        ..ConvertOptions::default()
    };
    let doc = convert_with("Hi", &options);
    assert!(joined(&doc).contains(&replacement), "{}", doc.render());
}

// Slice 7 plan: replacement text at most 4096.
#[test]
fn replacement_text_of_4097_throws() {
    let options = ConvertOptions {
        text_replacements: Some(vec![literal_replacement("Hi", &"b".repeat(4097))]),
        ..ConvertOptions::default()
    };
    let err = expect_error("Hi", &options);
    assert!(
        err.contains("textReplacements replacement text is too large"),
        "{err}"
    );
}

// Jest: "rejects invalid textReplacementMode values"
// Jest: "rejects null textReplacementMode values"
#[test]
fn invalid_text_replacement_mode_throws() {
    let options = ConvertOptions {
        text_replacement_mode: Some(TextReplacementMode::Invalid),
        text_replacements: Some(vec![literal_replacement("Hello", "Hi")]),
        ..ConvertOptions::default()
    };
    let err = expect_error("Hello", &options);
    assert!(
        err.contains("Invalid textReplacementMode: Must be trusted or untrusted"),
        "{err}"
    );
}

fn regex_options(pattern: &str, replace: &str) -> ConvertOptions {
    ConvertOptions {
        text_replacements: Some(vec![TextReplacement {
            find: TextFind::Regex(pattern.to_string()),
            replace: TextReplace::Text(replace.to_string()),
        }]),
        ..ConvertOptions::default()
    }
}

// Jest: "rejects oversized single markdown input when maxInputLength is set"
#[test]
fn ascii_over_max_input_length_throws() {
    let err = expect_error("123456", &limits(Some(5), None));
    assert!(
        err.contains("Markdown input length exceeds maxInputLength"),
        "{err}"
    );
}

// Jest: "rejects oversized single markdown input when maxInputLength is set"
//
// "é" is 1 UTF-16 code unit and 2 bytes. A byte length would reject it.
#[test]
fn bmp_character_counts_as_one_utf16_unit() {
    assert_eq!(utf16_units("é"), 1);
    assert_eq!("é".len(), 2);
    let doc = convert_with("é", &limits(Some(1), None));
    assert!(joined(&doc).contains('é'), "{}", doc.render());
}

// Slice 7 plan: maxInputLength counts UTF-16 code units.
//
// "😀" is 2 UTF-16 code units, 4 bytes, and 1 scalar value.
#[test]
fn emoji_counts_as_two_utf16_units() {
    let emoji = "😀";
    assert_eq!(utf16_units(emoji), 2);
    assert_eq!(emoji.len(), 4);
    assert_eq!(emoji.chars().count(), 1);
    let err = expect_error(emoji, &limits(Some(1), None));
    assert!(
        err.contains("Markdown input length exceeds maxInputLength"),
        "{err}"
    );
    let doc = convert_with(emoji, &limits(Some(2), None));
    assert!(joined(&doc).contains(emoji), "{}", doc.render());
}

// Jest: "rejects summed multi-section markdown input when maxInputLength is set"
#[test]
fn section_lengths_are_summed() {
    let options = section_options(
        vec![
            one_section("123", SectionConfig::default()),
            one_section("456", SectionConfig::default()),
        ],
        5,
    );
    let err = expect_error("", &options);
    assert!(
        err.contains("Markdown input length exceeds maxInputLength"),
        "{err}"
    );
}

// Slice 7 plan: header and footer markdown are added after the section sum.
//
// Body "a" is 1 code unit. Header "😀" is 2. Total 3.
#[test]
fn header_markdown_counts_in_utf16_units() {
    let emoji = "😀";
    assert_eq!(utf16_units(emoji), 2);
    let options = |limit: u32| {
        section_options(
            vec![one_section(
                "a",
                SectionConfig {
                    headers: Some(HeaderFooterGroup {
                        default: Some(HeaderFooterSlot::Content(HeaderFooterContent {
                            markdown: Some(emoji.to_string()),
                            ..HeaderFooterContent::default()
                        })),
                        ..HeaderFooterGroup::default()
                    }),
                    ..SectionConfig::default()
                },
            )],
            limit,
        )
    };
    let err = expect_error("", &options(2));
    assert_eq!(err, "Markdown input exceeds maxInputLength");
    let doc = convert_with("", &options(3));
    assert!(joined(&doc).contains('a'), "{}", doc.render());
    let stories = doc
        .stories
        .iter()
        .flat_map(|story| story.blocks.iter())
        .filter_map(|block| match block {
            common::Block::Paragraph(paragraph) => Some(paragraph.text.as_str()),
            common::Block::Table(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(stories.contains(emoji), "{stories}");
}

// Jest: "rejects invalid maxInputLength values"
#[test]
fn zero_max_input_length_throws() {
    let err = expect_error("Valid input", &limits(Some(0), None));
    assert!(
        err.contains("maxInputLength must be a positive integer"),
        "{err}"
    );
}

// Jest: "rejects markdown that exceeds maxElements"
#[test]
fn max_elements_rejects_a_paragraph() {
    let err = expect_error("Hello", &limits(None, Some(1)));
    assert!(
        err.contains("Markdown element count exceeds maxElements"),
        "{err}"
    );
}

// Jest: "applies maxElements across sections"
#[test]
fn max_elements_counts_across_sections() {
    let options = ConvertOptions {
        sections: Some(vec![
            one_section("# One", SectionConfig::default()),
            one_section("# Two", SectionConfig::default()),
        ]),
        max_elements: Some(3),
        ..ConvertOptions::default()
    };
    let err = expect_error("", &options);
    assert!(
        err.contains("Markdown element count exceeds maxElements"),
        "{err}"
    );
}

// Jest: "does not apply input length or element limits by default"
#[test]
fn limits_are_unset_by_default() {
    let doc = convert(&"a".repeat(1_048_577));
    assert!(joined(&doc).contains(&"a".repeat(1_048_577)));
}

// Jest: "rejects invalid maxElements values"
#[test]
fn zero_max_elements_throws() {
    let err = expect_error("Valid input", &limits(None, Some(0)));
    assert!(
        err.contains("maxElements must be a positive integer"),
        "{err}"
    );
}
