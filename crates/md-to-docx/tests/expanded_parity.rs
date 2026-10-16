//! Regressions found by the expanded full-package differential corpus.
#[path = "common/mod.rs"]
mod common;

use md_to_docx::options_json::load_options_json;
use md_to_docx::{convert_with_options, patch_markdown, ConvertOptions};
use std::collections::BTreeMap;

fn options(json: &str) -> ConvertOptions {
    load_options_json(json, "test").unwrap().convert
}

fn convert(markdown: &str, json: &str) -> common::Document {
    common::project_docx(&convert_with_options(markdown, &options(json)).unwrap())
}

#[test]
fn validation_rejects_disabled_toc_depth_and_empty_sections() {
    for (json, expected) in [
        (
            r##"{"toc":{"minDepth":0}}"##,
            "Invalid TOC minDepth: Must be an integer between 1 and 6",
        ),
        (
            r##"{"toc":{"minDepth":3,"maxDepth":2}}"##,
            "Invalid TOC depth range: minDepth cannot be greater than maxDepth",
        ),
        (
            r##"{"sections":[]}"##,
            "Invalid sections input: options.sections must contain at least one section",
        ),
        (
            r##"{"imageHandling":{"maxImages":0}}"##,
            "maxImages must be a positive integer",
        ),
        (
            r##"{"sections":[{"markdown":"x","page":{"size":{"width":0}}}]}"##,
            "Invalid page width: Must be a finite number > 0",
        ),
    ] {
        let error = load_options_json(json, "test")
            .map_err(|error| error.to_string())
            .and_then(|loaded| {
                convert_with_options("body", &loaded.convert).map_err(|error| error.message)
            })
            .unwrap_err();
        assert_eq!(error, expected);
    }
}

#[test]
fn paragraph_inline_code_uses_two_points_less_than_body() {
    let doc = convert("Body `code`.", r##"{"style":{"paragraphSize":30}}"##);
    let paragraph = doc.paragraphs()[0];
    assert_eq!(
        paragraph
            .runs
            .iter()
            .find(|run| run.text == "Body ")
            .unwrap()
            .size,
        Some(30)
    );
    assert_eq!(
        paragraph
            .runs
            .iter()
            .find(|run| run.text == "code")
            .unwrap()
            .size,
        Some(28)
    );
}

#[test]
fn section_style_does_not_replace_document_style_definitions() {
    let doc = convert(
        "ignored",
        r##"{"template":{"style":{"titleSize":40}},"sections":[{"markdown":"# Heading","style":{"direction":"RTL"}}]}"##,
    );
    assert_eq!(doc.paragraphs()[0].runs[0].size, Some(40));
    let styles = &doc.parts["word/styles.xml"];
    assert!(!styles.contains("<w:rtl"));
    assert!(!styles.contains("w:val=\"40\""));
}

#[test]
fn autolinks_precede_emphasis_and_entity_decoding() {
    let doc = convert("www.a.ba*b*c http://a.b&amp;", "{}");
    let runs = &doc.paragraphs()[0].runs;
    assert!(runs.iter().any(
        |run| run.text == "www.a.ba*b*c" && run.link.as_deref() == Some("http://www.a.ba*b*c")
    ));
    assert!(runs
        .iter()
        .any(|run| run.text == "http://a.b" && run.link.as_deref() == Some("http://a.b")));
    assert!(!runs
        .iter()
        .any(|run| run.link.as_deref() == Some("http://a.b&")));
}

#[test]
fn empty_replacement_pattern_terminates() {
    let doc = convert("x", r##"{"textReplacements":[{"find":"","replace":"v"}]}"##);
    assert_eq!(doc.paragraph_texts(), ["x"]);
}

#[test]
fn footnotes_keep_images_in_their_original_paragraphs() {
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=";
    let doc = convert(
        &format!(
            "x[^1]\n\n[^1]: ![first]({png}) and ![missing](missing.png)\n\n    ![second]({png})"
        ),
        "{}",
    );
    let note = &doc.footnotes[0];
    assert_eq!(note.paragraphs.len(), 2);
    assert_eq!(note.paragraphs[0].drawings.len(), 1);
    assert_eq!(note.paragraphs[1].drawings.len(), 1);
    assert!(note.paragraphs[0]
        .text
        .contains(" and [Image could not be displayed: missing]"));
}

#[test]
fn dollar_runs_pair_by_length_and_flow_math_interrupts_paragraphs() {
    let doc = convert("$a$$b$\n\nBefore\n$$\nx^2\n$$\nAfter\n===", "{}");
    assert_eq!(doc.paragraphs()[0].math[0].text, "a$$b");
    assert_eq!(doc.paragraphs()[1].text, "Before");
    assert_eq!(doc.paragraphs()[2].math[0].text, "x2");
    assert_eq!(doc.paragraphs()[3].heading.as_deref(), Some("Heading1"));
    assert_eq!(doc.paragraphs()[3].text, "After");
}

#[test]
fn whitespace_footnote_labels_remain_literal() {
    let doc = convert("a[^1] b[^ spaced ]\n\n[^1]: lower\n[^ spaced ]: body", "{}");
    assert_eq!(doc.footnotes.len(), 1);
    assert!(doc.paragraph_texts().join(" ").contains("[^ spaced ]"));
}

#[test]
fn patch_respects_custom_delimiters_and_sets_toc_fields() {
    let template = convert_with_options("{{body}}", &ConvertOptions::default()).unwrap();
    let loaded = load_options_json(
        r##"{"placeholderDelimiters":{"start":"[[","end":"]]"}}"##,
        "test",
    )
    .unwrap();
    let patches = BTreeMap::from([("body".to_string(), "Replacement".to_string())]);
    let untouched =
        common::project_docx(&patch_markdown(&template, &patches, &loaded.patch).unwrap());
    assert_eq!(untouched.paragraph_texts(), ["{{body}}"]);
    let loaded = load_options_json(r##"{"toc":{"enabled":true}}"##, "test").unwrap();
    let patches = BTreeMap::from([("body".to_string(), "[TOC]\n\n# Heading".to_string())]);
    let patched =
        common::project_docx(&patch_markdown(&template, &patches, &loaded.patch).unwrap());
    assert!(patched.update_fields);
    assert!(patched
        .paragraphs()
        .iter()
        .any(|paragraph| paragraph.tabs.iter().any(|tab| tab.pos == "9000")));
}

#[test]
fn partially_used_emphasis_runs_recheck_the_rule_of_three() {
    let doc = convert("***bi****i*", "{}");
    assert_eq!(doc.paragraph_texts(), ["bi**i"]);
    assert!(doc.paragraphs()[0]
        .runs
        .iter()
        .any(|run| run.text == "**i" && run.italic && !run.bold));
}
