//! Slice 8 parity with metadata, reference packages, and placeholder patch.
//!
//! ZIP bytes are not compared. Where a test needs a `.docx` fixture, the
//! TypeScript package in `dist/` builds that package and, for reference
//! errors, reports the code. Paragraph comparisons drop `MDPATCH_BOUNDARY`
//! donor markers.
//!
//! Jest helper cases that only check Blob, ArrayBuffer, or Buffer wrappers
//! are one bytes result here. Slice 10 owns those wrappers. Drawing alt tests
//! stay in slice 5 and slice 6.
//!
//! `aborted: true` stands in for an already-aborted `AbortSignal`.
//!
//! Not sent, because it calls `loadReferenceDocx` and
//! `applyReferenceDocxPresentation` rather than the bytes entry points:
//! "accepts generated packages without a numbering part".

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use common::Document;

use md_to_docx::options::{
    Direction, DocumentMetadata, DocumentSection, DuplicateStyleNameBehavior, MissingStyleBehavior,
    PatchMarkdownOptions, ReferenceDocxGenerationOptions, ReferenceDocxModeOptions,
    ReferenceDocxPackageLimits, ReferenceStyleSelector, SectionConfig, StyleOverrides,
};
use md_to_docx::{
    apply_document_metadata, convert_markdown, convert_with_options, convert_with_reference,
    patch_markdown, ConvertOptions, MarkdownConversionError,
};

const FRESH_REPORT: &str = "# Fresh report

## Details

Body with **strong**, *emphasis*, `inline code`, and [a link](https://example.com).

> Quoted guidance.

```text
const answer = 42;
```

| Metric | Value |
| --- | --- |
| Score | 42 |

1. First
2. Second

- Bullet";

const ONE_PX_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";

const RELATIONSHIP_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z7SAAAAAASUVORK5CYII=";

static ORACLE_SEQ: AtomicU64 = AtomicU64::new(0);

struct OracleFailure {
    code: String,
    phase: String,
    role: String,
    entry: String,
    limit: String,
    relationship_type: String,
    message: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/slice8_oracle.mjs")
}

fn temp_path(label: &str) -> PathBuf {
    let seq = ORACLE_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("slice8-{label}-{}-{seq}", std::process::id()))
}

fn oracle(args: &[String]) -> Vec<u8> {
    let out = temp_path("out");
    let output = Command::new("node")
        .arg(oracle_script())
        .arg(&out)
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|err| panic!("node oracle failed to start: {err}"));
    if !output.status.success() {
        let _ = std::fs::remove_file(&out);
        panic!(
            "oracle {args:?} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = std::fs::read(&out).unwrap_or_else(|err| panic!("read oracle output: {err}"));
    let _ = std::fs::remove_file(&out);
    bytes
}

fn oracle_arg(command: &str, extra: &[&str]) -> Vec<u8> {
    let mut args = vec![command.to_string()];
    args.extend(extra.iter().map(|arg| (*arg).to_string()));
    oracle(&args)
}

fn write_request(body: &str) -> PathBuf {
    let path = temp_path("request.json");
    std::fs::write(&path, body).unwrap_or_else(|err| panic!("write oracle request: {err}"));
    path
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn reference_request(markdown: &str, options_json: &str) -> String {
    format!(
        "{{\"markdown\":{},\"options\":{options_json}}}",
        json_string(markdown)
    )
}

fn patch_request(patches_json: &str, options_json: &str) -> String {
    format!("{{\"patches\":{patches_json},\"options\":{options_json}}}")
}

fn one_patch_json(name: &str, markdown: &str, options_json: &str) -> String {
    patch_request(
        &format!("{{\"{name}\":{}}}", json_string(markdown)),
        options_json,
    )
}

fn corporate(mutation: &str) -> Vec<u8> {
    if mutation.is_empty() {
        oracle_arg("corporate", &[])
    } else {
        oracle_arg("corporate", &[mutation])
    }
}

fn template_docx() -> Vec<u8> {
    oracle_arg("template", &[])
}

fn placeholder_docx(names: &[&str]) -> Vec<u8> {
    oracle_arg("placeholders", names)
}

fn raw_package(kind: &str) -> Vec<u8> {
    oracle_arg("raw", &[kind])
}

fn oracle_reference(markdown: &str, reference: &[u8], options_json: &str) -> Vec<u8> {
    let reference_path = temp_path("reference.docx");
    std::fs::write(&reference_path, reference).expect("write reference fixture");
    let request = write_request(&reference_request(markdown, options_json));
    let bytes = oracle(&[
        "ts-reference".to_string(),
        reference_path.display().to_string(),
        request.display().to_string(),
    ]);
    let _ = std::fs::remove_file(&reference_path);
    let _ = std::fs::remove_file(&request);
    bytes
}

fn oracle_patch(reference: &[u8], request_body: &str) -> Vec<u8> {
    let reference_path = temp_path("reference.docx");
    std::fs::write(&reference_path, reference).expect("write patch fixture");
    let request = write_request(request_body);
    let bytes = oracle(&[
        "ts-patch".to_string(),
        reference_path.display().to_string(),
        request.display().to_string(),
    ]);
    let _ = std::fs::remove_file(&reference_path);
    let _ = std::fs::remove_file(&request);
    bytes
}

fn oracle_reference_error(markdown: &str, reference: &[u8], options_json: &str) -> OracleFailure {
    let reference_path = temp_path("reference.docx");
    std::fs::write(&reference_path, reference).expect("write reference fixture");
    let request = write_request(&reference_request(markdown, options_json));
    let bytes = oracle(&[
        "ts-error-reference".to_string(),
        reference_path.display().to_string(),
        request.display().to_string(),
    ]);
    let _ = std::fs::remove_file(&reference_path);
    let _ = std::fs::remove_file(&request);
    parse_oracle_failure(&bytes)
}

fn oracle_patch_error(reference: &[u8], request_body: &str) -> OracleFailure {
    let reference_path = temp_path("reference.docx");
    std::fs::write(&reference_path, reference).expect("write patch fixture");
    let request = write_request(request_body);
    let bytes = oracle(&[
        "ts-error-patch".to_string(),
        reference_path.display().to_string(),
        request.display().to_string(),
    ]);
    let _ = std::fs::remove_file(&reference_path);
    let _ = std::fs::remove_file(&request);
    parse_oracle_failure(&bytes)
}

fn parse_oracle_failure(bytes: &[u8]) -> OracleFailure {
    let text = String::from_utf8(bytes.to_vec()).expect("oracle error text");
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    OracleFailure {
        code: fields.remove("code").unwrap_or_default(),
        phase: fields.remove("phase").unwrap_or_default(),
        role: fields.remove("role").unwrap_or_default(),
        entry: fields.remove("entry").unwrap_or_default(),
        limit: fields.remove("limit").unwrap_or_default(),
        relationship_type: fields.remove("relationshipType").unwrap_or_default(),
        message: fields.remove("message").unwrap_or_default(),
    }
}

fn project(bytes: &[u8]) -> Document {
    assert!(
        bytes.starts_with(b"PK"),
        "conversion should return a zip package"
    );
    common::project_docx(bytes)
}

fn convert(markdown: &str) -> Document {
    project(&convert_markdown(markdown).unwrap_or_else(|err| panic!("convert failed: {err}")))
}

fn convert_with(markdown: &str, options: &ConvertOptions) -> Document {
    project(
        &convert_with_options(markdown, options)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn convert_reference(
    markdown: &str,
    reference: &[u8],
    options: &ReferenceDocxGenerationOptions,
) -> Document {
    project(
        &convert_with_reference(markdown, reference, options)
            .unwrap_or_else(|err| panic!("reference conversion failed: {err}")),
    )
}

fn patch_map(patches: &[(&str, &str)]) -> BTreeMap<String, String> {
    patches
        .iter()
        .map(|(name, markdown)| ((*name).to_string(), (*markdown).to_string()))
        .collect()
}

fn patch_bytes(
    reference: &[u8],
    patches: &[(&str, &str)],
    options: &PatchMarkdownOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    patch_markdown(reference, &patch_map(patches), options)
}

fn patch_with(
    reference: &[u8],
    patches: &[(&str, &str)],
    options: &PatchMarkdownOptions,
) -> Document {
    project(
        &patch_bytes(reference, patches, options)
            .unwrap_or_else(|err| panic!("patch failed: {err}")),
    )
}

fn part<'a>(doc: &'a Document, name: &str) -> &'a str {
    doc.parts
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn part_ending<'a>(doc: &'a Document, suffix: &str) -> &'a str {
    doc.parts
        .iter()
        .find(|(name, _)| name.ends_with(suffix))
        .map(|(_, text)| text.as_str())
        .unwrap_or_else(|| panic!("no part ends with {suffix}"))
}

fn has_part_suffix(doc: &Document, suffix: &str) -> bool {
    doc.parts.keys().any(|name| name.ends_with(suffix))
}

fn metadata_options(metadata: DocumentMetadata) -> ConvertOptions {
    ConvertOptions {
        metadata: Some(metadata),
        ..ConvertOptions::default()
    }
}

fn section(markdown: &str, config: SectionConfig) -> DocumentSection {
    DocumentSection {
        markdown: markdown.to_string(),
        config,
    }
}

fn style_map(
    pairs: Vec<(&str, ReferenceStyleSelector)>,
) -> Option<BTreeMap<String, ReferenceStyleSelector>> {
    Some(
        pairs
            .into_iter()
            .map(|(role, selector)| (role.to_string(), selector))
            .collect(),
    )
}

fn mode(fill: impl FnOnce(&mut ReferenceDocxModeOptions)) -> ReferenceDocxGenerationOptions {
    let mut reference = ReferenceDocxModeOptions::default();
    fill(&mut reference);
    ReferenceDocxGenerationOptions {
        reference: Some(reference),
        ..ReferenceDocxGenerationOptions::default()
    }
}

fn visible_texts(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .map(|paragraph| paragraph.text.trim().to_string())
        .filter(|text| !text.is_empty() && !text.contains("MDPATCH_BOUNDARY"))
        .collect()
}

fn assert_body_matches_reference_oracle(
    markdown: &str,
    reference: &[u8],
    options_json: &str,
    rust_doc: &Document,
) {
    let typescript = project(&oracle_reference(markdown, reference, options_json));
    assert_eq!(
        visible_texts(rust_doc),
        visible_texts(&typescript),
        "body text drifted from the TypeScript package"
    );
}

fn assert_body_matches_patch_oracle(reference: &[u8], request: &str, rust_doc: &Document) {
    let typescript = project(&oracle_patch(reference, request));
    assert_eq!(
        visible_texts(rust_doc),
        visible_texts(&typescript),
        "patched body text drifted from the TypeScript package"
    );
}

fn reference_error(
    markdown: &str,
    reference: &[u8],
    options_json: &str,
    options: &ReferenceDocxGenerationOptions,
) -> (OracleFailure, MarkdownConversionError) {
    let expected = oracle_reference_error(markdown, reference, options_json);
    let err = convert_with_reference(markdown, reference, options)
        .expect_err("reference conversion should fail");
    assert_eq!(
        err.context()
            .and_then(|ctx| ctx.get("phase"))
            .map(String::as_str),
        Some("reference-docx"),
        "{err}"
    );
    assert_eq!(
        err.context()
            .and_then(|ctx| ctx.get("code"))
            .map(String::as_str),
        Some(expected.code.as_str()),
        "code drifted from the TypeScript package: {err}"
    );
    (expected, err)
}

fn context_value<'a>(err: &'a MarkdownConversionError, key: &str) -> &'a str {
    err.context()
        .and_then(|ctx| ctx.get(key))
        .map(String::as_str)
        .unwrap_or_else(|| panic!("missing context {key} on {err}"))
}

fn bookmark_names(doc: &Document) -> Vec<String> {
    doc.paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.bookmarks.iter().cloned())
        .collect()
}

fn assert_tbl_w_type_then_width(xml: &str, width: &str) {
    let width_attr = format!("w:w=\"{width}\"");
    let mut rest = xml;
    let mut found = false;
    while let Some(start) = rest.find("<w:tblW") {
        let end = rest[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(rest.len());
        let tag = &rest[start..end];
        if let (Some(type_at), Some(width_at)) = (tag.find("w:type=\"dxa\""), tag.find(&width_attr))
        {
            if type_at < width_at {
                found = true;
                break;
            }
        }
        rest = &rest[end..];
    }
    assert!(found, "missing tblW type then width {width}");
}

fn numbering_ids(numbering: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut rest = numbering;
    while let Some(start) = rest.find("<w:num") {
        let end = rest[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(rest.len());
        let tag = &rest[start..end];
        let boundary = tag
            .strip_prefix("<w:num")
            .is_some_and(|after| after.starts_with(|ch: char| !ch.is_ascii_alphanumeric()));
        if boundary {
            if let Some(value) = tag.split("w:numId=\"").nth(1) {
                let id = value.split('"').next().unwrap_or("");
                if let Ok(id) = id.parse() {
                    ids.push(id);
                }
            }
        }
        rest = &rest[start + 1..];
    }
    ids
}

fn corp_list_num_id(styles: &str) -> u32 {
    let start = styles
        .find("w:styleId=\"CorpList\"")
        .expect("CorpList style");
    let rest = &styles[start..];
    let key = "w:numId w:val=\"";
    let at = rest.find(key).expect("CorpList numId");
    let value = &rest[at + key.len()..];
    let end = value.find('"').expect("numId end");
    value[..end].parse().expect("numId integer")
}

fn custom_props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

// Jest: "writes every supported core, app, and custom property with explicit timestamps"
// tests/accessibility-metadata.test.ts
#[test]
fn writes_every_supported_metadata_property() {
    let doc = convert_with(
        "# Report",
        &metadata_options(DocumentMetadata {
            title: Some("Roadmap v3".to_string()),
            subject: Some("Accessibility".to_string()),
            description: Some("Generated report".to_string()),
            creator: Some("Primary creator".to_string()),
            author: Some("Ignored alias".to_string()),
            keywords: Some(vec![
                "docx".to_string(),
                "a11y".to_string(),
                "Unicode 二".to_string(),
            ]),
            category: Some("Engineering".to_string()),
            company: Some("Example Co".to_string()),
            language: Some("en-GB".to_string()),
            created: Some("2026-01-02T03:04:05Z".to_string()),
            modified: Some("2026-02-03T04:05:06Z".to_string()),
            custom: Some(custom_props(&[("Build", "v3"), ("ReviewState", "Draft")])),
            ..DocumentMetadata::default()
        }),
    );
    let core = part(&doc, "docProps/core.xml");
    let app = part(&doc, "docProps/app.xml");
    let custom = part(&doc, "docProps/custom.xml");
    assert!(core.contains("<dc:title"));
    assert!(core.contains(">Roadmap v3</dc:title>"));
    assert!(core.contains(">Accessibility</dc:subject>"));
    assert!(core.contains(">Generated report</dc:description>"));
    assert!(core.contains(">Primary creator</dc:creator>"));
    assert!(!core.contains("Ignored alias"));
    assert!(core.contains(">docx; a11y; Unicode 二</cp:keywords>"));
    assert!(core.contains(">Engineering</cp:category>"));
    assert!(core.contains(">en-GB</dc:language>"));
    assert!(core.contains("2026-01-02T03:04:05.000Z"));
    assert!(core.contains("2026-02-03T04:05:06.000Z"));
    assert!(app.contains("<Company>Example Co</Company>"));
    assert!(custom.contains("name=\"Build\""));
    assert!(custom.contains(">v3</vt:lpwstr>"));
    assert!(custom.contains("name=\"ReviewState\""));
}

// Jest: "escapes XML-special characters, preserves Unicode, and omits implicit timestamps"
// tests/accessibility-metadata.test.ts
#[test]
fn escapes_metadata_and_omits_implicit_timestamps() {
    let doc = convert_with(
        "Content",
        &metadata_options(DocumentMetadata {
            title: Some("R&D <測試> \"alpha\"".to_string()),
            creator: Some("Zoë & فريق".to_string()),
            company: Some("A < B & C".to_string()),
            custom: Some(custom_props(&[("Owner & team", "Miyuki <美雪> & co")])),
            ..DocumentMetadata::default()
        }),
    );
    let core = part(&doc, "docProps/core.xml");
    let app = part(&doc, "docProps/app.xml");
    let custom = part(&doc, "docProps/custom.xml");
    assert!(core.contains("R&amp;D &lt;測試&gt; \"alpha\""));
    assert!(core.contains("Zoë &amp; فريق"));
    assert!(!core.contains("dcterms:created"));
    assert!(!core.contains("dcterms:modified"));
    assert!(app.contains("A &lt; B &amp; C"));
    assert!(custom.contains("name=\"Owner &amp; team\""));
    assert!(custom.contains("Miyuki &lt;美雪&gt; &amp; co"));
    assert_eq!(app.matches("<?xml").count(), 1);
}

// Jest: "rejects invalid controls, oversized values, dates, language, and custom-property counts"
// tests/accessibility-metadata.test.ts
#[test]
fn rejects_invalid_metadata() {
    let control = convert_with_options(
        "Content",
        &metadata_options(DocumentMetadata {
            title: Some("bad\u{0001}value".to_string()),
            ..DocumentMetadata::default()
        }),
    )
    .expect_err("control characters should fail");
    assert_eq!(
        control
            .context()
            .and_then(|ctx| ctx.get("context"))
            .map(String::as_str),
        Some("metadata.title")
    );

    let oversized = convert_with_options(
        "Content",
        &metadata_options(DocumentMetadata {
            description: Some("x".repeat(4_097)),
            ..DocumentMetadata::default()
        }),
    )
    .expect_err("oversized description should fail");
    assert!(oversized.to_string().contains("maximum length of 4096"));

    let date = convert_with_options(
        "Content",
        &metadata_options(DocumentMetadata {
            created: Some("not-a-date".to_string()),
            ..DocumentMetadata::default()
        }),
    )
    .expect_err("bad created date should fail");
    assert!(date
        .to_string()
        .contains("metadata.created must be a valid ISO date"));

    let language = convert_with_options(
        "Content",
        &metadata_options(DocumentMetadata {
            language: Some("not a tag".to_string()),
            ..DocumentMetadata::default()
        }),
    )
    .expect_err("bad language should fail");
    assert!(language.to_string().contains("BCP 47-style"));

    let custom = (0..65)
        .map(|index| (format!("P{index}"), "value".to_string()))
        .collect();
    let count = convert_with_options(
        "Content",
        &metadata_options(DocumentMetadata {
            custom: Some(custom),
            ..DocumentMetadata::default()
        }),
    )
    .expect_err("65 custom properties should fail");
    assert!(count.to_string().contains("maximum of 64 properties"));
}

// Jest: "keeps the legacy metadata packing path when metadata is unused"
// tests/accessibility-metadata.test.ts
#[test]
fn legacy_metadata_when_metadata_is_omitted() {
    let doc = convert("Content");
    let core = part(&doc, "docProps/core.xml");
    let styles = part(&doc, "word/styles.xml");
    assert!(core.contains("dcterms:created"));
    assert!(core.contains("dcterms:modified"));
    assert!(core.contains("Un-named"));
    assert!(!styles.contains("<w:lang"));
}

// Jest: "treats an empty metadata object as an explicit normalized metadata set"
// tests/accessibility-metadata.test.ts
#[test]
fn empty_metadata_object_omits_legacy_fields() {
    let doc = convert_with("Content", &metadata_options(DocumentMetadata::default()));
    let core = part(&doc, "docProps/core.xml");
    assert!(!core.contains("dcterms:created"));
    assert!(!core.contains("dcterms:modified"));
    assert!(!core.contains("Un-named"));
}

// Jest: "returns metadata consistently from Blob, ArrayBuffer, and Buffer helpers"
// tests/accessibility-metadata.test.ts
//
// Those three wrappers are the same bytes. This asserts the bytes path once.
#[test]
fn metadata_title_on_the_bytes_path() {
    let doc = convert_with(
        "Content",
        &metadata_options(DocumentMetadata {
            title: Some("Helper metadata".to_string()),
            ..DocumentMetadata::default()
        }),
    );
    assert!(part(&doc, "docProps/core.xml").contains("Helper metadata"));
}

// Jest: "preserves extended app properties when company is omitted"
// tests/accessibility-metadata.test.ts
#[test]
fn preserves_app_company_when_company_is_omitted() {
    let source = oracle_arg("company-source", &[]);
    let updated = apply_document_metadata(
        &source,
        &DocumentMetadata {
            title: Some("Updated title".to_string()),
            ..DocumentMetadata::default()
        },
    )
    .unwrap_or_else(|err| panic!("apply metadata failed: {err}"));
    let doc = project(&updated);
    assert!(part(&doc, "docProps/core.xml").contains("Updated title"));
    assert!(part(&doc, "docProps/app.xml").contains("<Company>Existing Co</Company>"));
}

// Jest: "emits language, bidirectional runs/paragraphs, semantic headings, and table headers"
// tests/accessibility-metadata.test.ts
#[test]
fn metadata_language_and_rtl_structure() {
    let doc = convert_with(
        "# عنوان\n\nنص\n\n| الرأس | القيمة |\n| --- | --- |\n| أ | 1 |",
        &ConvertOptions {
            metadata: Some(DocumentMetadata {
                language: Some("ar-SA".to_string()),
                ..DocumentMetadata::default()
            }),
            style: Some(StyleOverrides {
                direction: Some(Direction::Rtl),
                ..StyleOverrides::default()
            }),
            ..ConvertOptions::default()
        },
    );
    let document = part(&doc, "word/document.xml");
    let styles = part(&doc, "word/styles.xml");
    assert!(styles.contains("w:lang w:val=\"ar-SA\" w:bidi=\"ar-SA\""));
    assert!(styles.contains("<w:outlineLvl w:val=\"1\"/>"));
    assert!(document.contains("<w:bidi/>"));
    assert!(document.contains("<w:rtl/>"));
    assert!(document.contains("w:pStyle w:val=\"Heading1\""));
    assert!(document.contains("<w:tblHeader/>"));
}

// Jest: "uses style language over metadata and section style over document style"
// tests/accessibility-metadata.test.ts
#[test]
fn style_language_overrides_metadata_and_section_style() {
    let doc = convert_with(
        "",
        &ConvertOptions {
            metadata: Some(DocumentMetadata {
                language: Some("fr-FR".to_string()),
                ..DocumentMetadata::default()
            }),
            style: Some(StyleOverrides {
                language: Some("en-GB".to_string()),
                ..StyleOverrides::default()
            }),
            sections: Some(vec![
                section("First section.", SectionConfig::default()),
                section(
                    "القسم الثاني.",
                    SectionConfig {
                        style: Some(StyleOverrides {
                            language: Some("ar-SA".to_string()),
                            direction: Some(Direction::Rtl),
                            ..StyleOverrides::default()
                        }),
                        ..SectionConfig::default()
                    },
                ),
            ]),
            ..ConvertOptions::default()
        },
    );
    let core = part(&doc, "docProps/core.xml");
    let document = part(&doc, "word/document.xml");
    let styles = part(&doc, "word/styles.xml");
    assert!(core.contains(">fr-FR</dc:language>"));
    assert!(styles.contains("w:lang w:val=\"en-GB\""));
    assert!(document.contains("w:lang w:val=\"en-GB\""));
    assert!(document.contains("w:lang w:val=\"ar-SA\" w:bidi=\"ar-SA\""));
}

// Jest: "applies one metadata set to a multi-section package"
// tests/accessibility-metadata.test.ts
#[test]
fn one_metadata_set_on_a_multi_section_package() {
    let doc = convert_with(
        "",
        &ConvertOptions {
            metadata: Some(DocumentMetadata {
                title: Some("Multi-section report".to_string()),
                company: Some("Sections Inc".to_string()),
                ..DocumentMetadata::default()
            }),
            sections: Some(vec![
                section("# One", SectionConfig::default()),
                section("# Two", SectionConfig::default()),
            ]),
            ..ConvertOptions::default()
        },
    );
    let document = part(&doc, "word/document.xml");
    assert_eq!(document.matches("<w:sectPr>").count(), 2);
    assert!(part(&doc, "docProps/core.xml").contains("Multi-section report"));
    assert!(part(&doc, "docProps/app.xml").contains("Sections Inc"));
}

// Jest: "adopts named styles by name and ID without copying static body content"
// tests/reference-docx-styles.test.ts
#[test]
fn adopts_named_styles_without_static_body() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"styles\":{\"heading1\":{\"name\":\"Executive Heading\"},\"heading2\":{\"id\":\"CorpH2\"}}}}"#;
    let options = mode(|reference| {
        reference.styles = style_map(vec![
            (
                "heading1",
                ReferenceStyleSelector::Name("Executive Heading".to_string()),
            ),
            ("heading2", ReferenceStyleSelector::Id("CorpH2".to_string())),
        ]);
    });
    let doc = convert_reference(FRESH_REPORT, &reference, &options);
    let document = part(&doc, "word/document.xml");
    let styles = part(&doc, "word/styles.xml");
    assert!(document.contains("Fresh report"));
    assert!(!document.contains("Corporate Cover Page"));
    assert!(!document.contains("Static closing paragraph"));
    assert!(document.contains("w:pStyle w:val=\"CorpH1\""));
    assert!(document.contains("w:pStyle w:val=\"CorpH2\""));
    assert!(document.contains("w:pStyle w:val=\"CorpBody\""));
    assert!(document.contains("w:pStyle w:val=\"CorpQuote\""));
    assert!(document.contains("w:pStyle w:val=\"CorpCode\""));
    assert!(document.contains("w:pStyle w:val=\"CorpList\""));
    assert!(document.contains("w:tblStyle w:val=\"CorpTable\""));
    assert!(document.contains("w:rStyle w:val=\"CorpStrong\""));
    assert!(document.contains("w:rStyle w:val=\"CorpEmphasis\""));
    assert!(document.contains("w:rStyle w:val=\"CorpInline\""));
    assert!(document.contains("w:rStyle w:val=\"CorpLink\""));
    assert!(styles.contains("w:styleId=\"CorpH1\""));
    assert!(styles.contains("w:styleId=\"FootnoteText\""));
    assert_body_matches_reference_oracle(FRESH_REPORT, &reference, options_json, &doc);
}

// Jest: "preserves theme, fonts, page geometry, headers, footers, and usable table width"
// tests/reference-docx-styles.test.ts
#[test]
fn preserves_theme_fonts_geometry_headers_and_footers() {
    let markdown = "# New body\n\n| A | B |\n| --- | --- |\n| 1 | 2 |";
    let reference = corporate("");
    let doc = convert_reference(
        markdown,
        &reference,
        &ReferenceDocxGenerationOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    let rels = part(&doc, "word/_rels/document.xml.rels");
    let types = part(&doc, "[Content_Types].xml");
    assert!(document.contains("w:top=\"2000\""));
    assert!(document.contains("w:left=\"1400\""));
    assert!(document.contains("w:tblW w:type=\"dxa\" w:w=\"9306\""));
    assert!(document.contains("w:headerReference"));
    assert!(document.contains("w:footerReference"));
    assert!(rels.contains("reference-package/word/header1.xml"));
    assert!(rels.contains("reference-package/word/footer1.xml"));
    assert!(rels.contains("reference-package/word/theme/theme1.xml"));
    assert!(rels.contains("reference-package/word/fontTable.xml"));
    assert!(types.contains("reference-package/word/theme/theme1.xml"));
    assert!(
        part_ending(&doc, "reference-package/word/theme/theme1.xml").contains("Corporate Theme")
    );
    assert!(part_ending(&doc, "reference-package/word/fontTable.xml").contains("w:name=\"Aptos\""));
    assert!(part_ending(&doc, "reference-package/word/header1.xml").contains("Template Header"));
    assert!(part_ending(&doc, "reference-package/word/footer1.xml").contains("Template Footer"));
    assert_body_matches_reference_oracle(markdown, &reference, "{}", &doc);
}

// Jest: "remaps reference numbering above generated ordered and bullet list IDs"
// tests/reference-docx-styles.test.ts
#[test]
fn remaps_reference_numbering_above_generated_ids() {
    let markdown = "1. Ordered\n2. Still ordered\n\n- Bullet";
    let reference = corporate("");
    let doc = convert_reference(
        markdown,
        &reference,
        &ReferenceDocxGenerationOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    let numbering = part(&doc, "word/numbering.xml");
    let styles = part(&doc, "word/styles.xml");
    let ids = numbering_ids(numbering);
    let style_num_id = corp_list_num_id(styles);
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::BTreeSet<_>>().len()
    );
    assert!(document.contains("<w:numId w:val=\"1\"/>"));
    assert!(document.contains("<w:numId w:val=\"2\"/>"));
    assert!(style_num_id > 2);
    assert!(ids.contains(&style_num_id));
    assert_body_matches_reference_oracle(markdown, &reference, "{}", &doc);
}

// Jest: "keeps generated tables, images, and links relationship-safe"
// tests/reference-docx-styles.test.ts
#[test]
fn keeps_tables_images_and_links_relationship_safe() {
    let markdown = format!(
        "| A | B |\n| --- | --- |\n| 1 | 2 |\n\n![pixel](data:image/png;base64,{RELATIONSHIP_PNG})\n\n[OpenAI](https://openai.com)"
    );
    let reference = corporate("");
    let doc = convert_reference(
        &markdown,
        &reference,
        &ReferenceDocxGenerationOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    let rels = part(&doc, "word/_rels/document.xml.rels");
    assert!(document.contains("<w:tbl>"));
    assert!(document.contains("<w:drawing>"));
    assert!(document.contains("<w:hyperlink"));
    assert!(rels.contains("Target=\"https://openai.com\""));
    assert!(doc.parts.keys().any(|name| name.starts_with("word/media/")));
    assert_body_matches_reference_oracle(&markdown, &reference, "{}", &doc);
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn missing_explicit_style_falls_back_to_heading1() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"styles\":{\"heading1\":{\"name\":\"Missing Heading\"}}}}"#;
    let options = mode(|reference| {
        reference.styles = style_map(vec![(
            "heading1",
            ReferenceStyleSelector::Name("Missing Heading".to_string()),
        )]);
    });
    let doc = convert_reference("# Heading", &reference, &options);
    assert!(part(&doc, "word/document.xml").contains("w:pStyle w:val=\"Heading1\""));
    assert_body_matches_reference_oracle("# Heading", &reference, options_json, &doc);
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn missing_explicit_style_throws() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"missingStyleBehavior\":\"throw\",\"styles\":{\"heading1\":{\"name\":\"Missing Heading\"}}}}"#;
    let options = mode(|reference| {
        reference.missing_style_behavior = Some(MissingStyleBehavior::Throw);
        reference.styles = style_map(vec![(
            "heading1",
            ReferenceStyleSelector::Name("Missing Heading".to_string()),
        )]);
    });
    let (expected, err) = reference_error("# Heading", &reference, options_json, &options);
    assert_eq!(expected.code, "MISSING_STYLE");
    assert_eq!(context_value(&err, "role"), "heading1");
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn duplicate_explicit_style_name_throws() {
    let reference = corporate("duplicate-styles");
    let options_json = r#"{\"reference\":{\"styles\":{\"heading1\":{\"name\":\"Executive Heading\"}}}}"#;
    let options = mode(|reference| {
        reference.styles = style_map(vec![(
            "heading1",
            ReferenceStyleSelector::Name("Executive Heading".to_string()),
        )]);
    });
    let (expected, _err) = reference_error("# Heading", &reference, options_json, &options);
    assert_eq!(expected.code, "DUPLICATE_STYLE_NAME");
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn duplicate_explicit_style_name_keeps_the_first() {
    let reference = corporate("duplicate-styles");
    let options_json = r#"{\"reference\":{\"duplicateStyleNameBehavior\":\"first\",\"styles\":{\"heading1\":{\"name\":\"Executive Heading\"}}}}"#;
    let options = mode(|reference| {
        reference.duplicate_style_name_behavior = Some(DuplicateStyleNameBehavior::First);
        reference.styles = style_map(vec![(
            "heading1",
            ReferenceStyleSelector::Name("Executive Heading".to_string()),
        )]);
    });
    let doc = convert_reference("# Heading", &reference, &options);
    assert!(part(&doc, "word/document.xml").contains("w:pStyle w:val=\"CorpH1\""));
    assert_body_matches_reference_oracle("# Heading", &reference, options_json, &doc);
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn automatic_quote_resolves_when_the_name_is_duplicated() {
    let reference = corporate("duplicate-styles");
    let markdown = "> Automatically resolved quote";
    let doc = convert_reference(
        markdown,
        &reference,
        &ReferenceDocxGenerationOptions::default(),
    );
    assert!(part(&doc, "word/document.xml").contains("w:pStyle w:val=\"CorpQuote\""));
    assert_body_matches_reference_oracle(markdown, &reference, "{}", &doc);
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_a_buffer_that_is_not_a_zip() {
    let reference = raw_package("not-a-zip");
    let (expected, _err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "INVALID_ZIP");
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_a_zip_that_is_not_a_package() {
    let reference = raw_package("hello");
    let (expected, _err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "MISSING_PACKAGE_PART");
}

// Jest: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
// tests/reference-docx-styles.test.ts
//
// `aborted: true` is the bytes stand-in for an already-aborted AbortSignal.
#[test]
fn rejects_an_already_aborted_reference_conversion() {
    let reference = corporate("");
    let err = convert_with_reference(
        "text",
        &reference,
        &ReferenceDocxGenerationOptions {
            aborted: true,
            ..ReferenceDocxGenerationOptions::default()
        },
    )
    .expect_err("an aborted reference conversion should fail");
    assert_eq!(
        err.context()
            .and_then(|ctx| ctx.get("phase"))
            .map(String::as_str),
        Some("reference-docx")
    );
    assert_eq!(
        err.context()
            .and_then(|ctx| ctx.get("code"))
            .map(String::as_str),
        Some("ABORTED")
    );
    assert!(err
        .to_string()
        .contains("Reference DOCX generation was aborted"));
}

// Plan error code INVALID_PACKAGE. The TypeScript package rejects this zip
// because the required parts are not a WordprocessingML package.
#[test]
fn rejects_a_zip_that_is_not_wordprocessingml() {
    let reference = raw_package("invalid-package");
    let (expected, _err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "INVALID_PACKAGE");
}

// Plan error code STYLE_TYPE_MISMATCH. A character style selected for heading1
// with throw behavior. tests/reference-docx-styles.test.ts does not name it.
#[test]
fn rejects_a_style_whose_type_does_not_match_the_role() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"missingStyleBehavior\":\"throw\",\"styles\":{\"heading1\":{\"id\":\"CorpStrong\"}}}}"#;
    let options = mode(|reference| {
        reference.missing_style_behavior = Some(MissingStyleBehavior::Throw);
        reference.styles = style_map(vec![(
            "heading1",
            ReferenceStyleSelector::Id("CorpStrong".to_string()),
        )]);
    });
    let (expected, err) = reference_error("# Heading", &reference, options_json, &options);
    assert_eq!(expected.code, "STYLE_TYPE_MISMATCH");
    assert_eq!(context_value(&err, "role"), "heading1");
}

// Jest: "rejects unsafe ZIP paths and bounded zip-bomb-style packages"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_an_unsafe_zip_entry_path() {
    let reference = raw_package("unsafe");
    let (expected, _err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "UNSAFE_ENTRY_PATH");
}

// Jest: "rejects unsafe ZIP paths and bounded zip-bomb-style packages"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_packages_over_the_uncompressed_limit() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"limits\":{\"maxUncompressedBytes\":1024}}}"#;
    let options = mode(|reference| {
        reference.limits = Some(ReferenceDocxPackageLimits {
            max_uncompressed_bytes: Some(1024),
            ..ReferenceDocxPackageLimits::default()
        });
    });
    let (expected, err) = reference_error("text", &reference, options_json, &options);
    assert_eq!(expected.code, "PACKAGE_LIMIT_EXCEEDED");
    assert_eq!(expected.limit, "1024");
    assert_eq!(context_value(&err, "limit"), "1024");
}

// Jest: "rejects unsafe ZIP paths and bounded zip-bomb-style packages"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_packages_over_the_entry_limit() {
    let reference = corporate("");
    let options_json = r#"{\"reference\":{\"limits\":{\"maxEntries\":3}}}"#;
    let options = mode(|reference| {
        reference.limits = Some(ReferenceDocxPackageLimits {
            max_entries: Some(3),
            ..ReferenceDocxPackageLimits::default()
        });
    });
    let (expected, _err) = reference_error("text", &reference, options_json, &options);
    assert_eq!(expected.code, "PACKAGE_LIMIT_EXCEEDED");
}

// Jest: "rejects malformed required and recursively imported XML parts"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_malformed_styles_xml() {
    let reference = corporate("malformed-styles");
    let (expected, err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "MALFORMED_XML");
    assert_eq!(expected.entry, "word/styles.xml");
    assert_eq!(context_value(&err, "entry"), "word/styles.xml");
}

// Jest: "rejects malformed required and recursively imported XML parts"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_malformed_header_xml() {
    let reference = corporate("malformed-header");
    let (expected, err) = reference_error(
        "text",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "MALFORMED_XML");
    assert_eq!(expected.entry, "word/header1.xml");
    assert_eq!(context_value(&err, "entry"), "word/header1.xml");
}

// Jest: "reuses generated DOCX files as deterministic reference input"
// tests/reference-docx-styles.test.ts
#[test]
fn reuses_a_generated_docx_as_reference_input() {
    let generated = convert_markdown("# Generated reference")
        .unwrap_or_else(|err| panic!("convert failed: {err}"));
    let generated_doc = project(&generated);
    let styles = part(&generated_doc, "word/styles.xml");
    let mut style_ids = Vec::new();
    let mut rest = styles;
    while let Some(start) = rest.find("<w:style") {
        let end = rest[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(rest.len());
        let tag = &rest[start..end];
        if let Some(value) = tag.split("w:styleId=\"").nth(1) {
            style_ids.push(value.split('"').next().unwrap_or("").to_string());
        }
        rest = &rest[start + 1..];
    }
    assert_eq!(
        style_ids.len(),
        style_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    let doc = convert_reference(
        "# Reused successfully",
        &generated,
        &ReferenceDocxGenerationOptions::default(),
    );
    assert!(part(&doc, "word/document.xml").contains("Reused successfully"));
}

// Jest: "rejects unsupported active header/footer relationship constructs"
// tests/reference-docx-styles.test.ts
#[test]
fn rejects_a_chart_relationship_in_a_header() {
    let reference = corporate("chart-rel");
    let (expected, err) = reference_error(
        "Fresh body",
        &reference,
        "{}",
        &ReferenceDocxGenerationOptions::default(),
    );
    assert_eq!(expected.code, "UNSUPPORTED_RELATIONSHIP");
    assert_eq!(
        expected.relationship_type,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart"
    );
    assert_eq!(
        context_value(&err, "relationshipType"),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart"
    );
}

// Jest: "returns Blob, ArrayBuffer, and Buffer and keeps ordinary conversion isolated"
// tests/reference-docx-styles.test.ts
//
// The Blob, ArrayBuffer, and Buffer wrappers are one bytes result. This checks
// that an ordinary conversion does not carry reference-package parts.
#[test]
fn ordinary_conversion_stays_free_of_reference_package_parts() {
    let doc = convert("# Ordinary");
    assert!(!doc
        .parts
        .keys()
        .any(|name| name.contains("reference-package")));
    assert!(part(&doc, "word/document.xml").contains("w:pStyle w:val=\"Heading1\""));
    let _ = has_part_suffix(&doc, "word/document.xml");
}

// Jest: "produces deterministic Word XML for the same markdown and reference"
// tests/reference-docx-styles.test.ts
#[test]
fn reference_word_xml_is_deterministic() {
    let reference = corporate("");
    let markdown = "# Stable\n\n1. One";
    let options = ReferenceDocxGenerationOptions::default();
    let first = convert_reference(markdown, &reference, &options);
    let second = convert_reference(markdown, &reference, &options);
    for name in [
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
        "word/_rels/document.xml.rels",
    ] {
        assert_eq!(part(&first, name), part(&second, name), "{name}");
    }
}

// Jest: "patches markdown into a placeholder while preserving template package parts"
// tests/reference-docx-patch.test.ts
#[test]
fn patches_a_placeholder_and_keeps_template_parts() {
    let markdown = "# Inserted Report

Hello **team**.

| Metric | Value |
| --- | --- |
| Score | 42 |";
    let reference = template_docx();
    let request = one_patch_json("body", markdown, "{}");
    let doc = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    assert!(document.contains("Corporate Cover Page"));
    assert!(document.contains("Inserted Report"));
    assert!(document.contains("Hello"));
    assert!(document.contains("team"));
    assert!(document.contains("<w:tbl>"));
    assert!(document.contains("Static closing paragraph"));
    assert!(!document.contains("{{body}}"));
    assert!(document.contains("w:top=\"2000\""));
    assert!(document.contains("w:left=\"1400\""));
    assert!(part(&doc, "word/header1.xml").contains("Template Header"));
    assert!(part(&doc, "word/footer1.xml").contains("Template Footer"));
    assert!(part(&doc, "word/styles.xml").contains("w:styleId=\"CorpTitle\""));
    assert_body_matches_patch_oracle(&reference, &request, &doc);
}

// Jest: "keeps the Blob-returning helper aligned with existing conversion helpers"
// tests/reference-docx-patch.test.ts
//
// The Blob wrapper is the bytes path. Non-empty output is the size check.
#[test]
fn patch_bytes_are_a_package() {
    let reference = template_docx();
    let bytes = patch_bytes(
        &reference,
        &[("body", "Patched as a Blob.")],
        &PatchMarkdownOptions::default(),
    )
    .unwrap_or_else(|err| panic!("patch failed: {err}"));
    assert!(bytes.starts_with(b"PK"));
    assert!(!bytes.is_empty());
    assert!(project(&bytes)
        .paragraph_texts()
        .iter()
        .any(|text| text.contains("Patched as a Blob.")));
}

// Jest: "shares heading bookmark counters across patched placeholders"
// tests/reference-docx-patch.test.ts
#[test]
fn heading_bookmark_counters_are_shared_across_placeholders() {
    let reference = placeholder_docx(&["first", "second"]);
    let request = patch_request(
        &format!(
            "{{\"first\":{},\"second\":{}}}",
            json_string("# Repeat"),
            json_string("# Repeat")
        ),
        "{}",
    );
    let doc = patch_with(
        &reference,
        &[("first", "# Repeat"), ("second", "# Repeat")],
        &PatchMarkdownOptions::default(),
    );
    let names = bookmark_names(&doc);
    assert!(names.iter().any(|name| name.contains("_Toc_Repeat_1")));
    assert!(names.iter().any(|name| name.contains("_Toc_Repeat_2")));
    assert_eq!(
        names.len(),
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    assert_body_matches_patch_oracle(&reference, &request, &doc);
}

// Jest: "uses the configured patch table width for inserted markdown tables"
// tests/reference-docx-patch.test.ts
#[test]
fn patch_table_uses_the_configured_width() {
    let markdown = "| Metric | Value |\n| --- | --- |\n| Score | 42 |";
    let reference = template_docx();
    let doc = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions {
            table_width_twips: Some(4321),
            ..PatchMarkdownOptions::default()
        },
    );
    assert_tbl_w_type_then_width(part(&doc, "word/document.xml"), "4321");
}

// Jest: "enforces maxElements across all patched placeholders"
// tests/reference-docx-patch.test.ts
#[test]
fn max_elements_applies_across_placeholders() {
    let reference = placeholder_docx(&["first", "second"]);
    let request = patch_request(
        &format!(
            "{{\"first\":{},\"second\":{}}}",
            json_string("First paragraph."),
            json_string("Second paragraph.")
        ),
        r#"{\"maxElements\":1}"#,
    );
    let expected = oracle_patch_error(&reference, &request);
    assert!(expected
        .message
        .contains("Markdown element count exceeds maxElements"));
    let err = patch_bytes(
        &reference,
        &[
            ("first", "First paragraph."),
            ("second", "Second paragraph."),
        ],
        &PatchMarkdownOptions {
            max_elements: Some(1),
            ..PatchMarkdownOptions::default()
        },
    )
    .expect_err("two paragraphs should exceed maxElements 1");
    assert!(err
        .to_string()
        .contains("Markdown element count exceeds maxElements"));
}

// Jest: "merges numbering for ordered lists"
// tests/reference-docx-patch.test.ts
#[test]
fn patch_merges_ordered_list_numbering() {
    let markdown = "5. first\n6. second";
    let reference = template_docx();
    let doc = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions::default(),
    );
    assert!(part(&doc, "word/document.xml").contains("<w:numPr>"));
    assert!(part(&doc, "word/numbering.xml").contains("w:start w:val=\"5\""));
}

// Jest: "merges caption fields and bookmarks"
// tests/reference-docx-patch.test.ts
#[test]
fn patch_merges_caption_fields() {
    let markdown = "See [@tbl:patched].\n\n| A |\n| --- |\n| B |\n\n: Patched table {#tbl:patched}";
    let reference = template_docx();
    let doc = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    assert!(document.contains(" SEQ MdToDocxTable"));
    assert!(document.contains(" REF mdp"));
    assert!(!document.contains("[@tbl:patched]"));
}

// Jest: "gives repeated copies unique footnotes and updates cached caption references"
// tests/feature-completeness.test.ts
#[test]
fn repeated_placeholder_copies_get_distinct_ids_and_recursive_false_stops() {
    let reference = placeholder_docx(&["body", "body"]);
    let markdown = "See [@tbl:data]. Note[^n] and again[^n].\n\n[^n]: Repeated note\n\n| A |\n|---|\n| B |\n\n: Data {#tbl:data}";
    let doc = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions::default(),
    );
    let document = part(&doc, "word/document.xml");
    let mut ids = Vec::new();
    let mut rest = document;
    while let Some(start) = rest.find("<w:footnoteReference") {
        let end = rest[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(rest.len());
        let tag = &rest[start..end];
        if let Some(value) = tag.split("w:id=\"").nth(1) {
            ids.push(value.split('"').next().unwrap_or("").to_string());
        }
        rest = &rest[start + 1..];
    }
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::BTreeSet<_>>().len()
    );
    let footnotes = part(&doc, "word/footnotes.xml");
    for id in &ids {
        assert!(footnotes.contains(&format!("<w:footnote w:id=\"{id}\"")));
    }
    assert!(document.contains("Table 1"));
    assert!(document.contains("Table 2"));
    let names = bookmark_names(&doc);
    assert_eq!(
        names.len(),
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );

    let once = patch_with(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions {
            recursive: Some(false),
            ..PatchMarkdownOptions::default()
        },
    );
    assert!(part(&once, "word/document.xml").contains("{{body}}"));
}

// Plan: new comments in patch Markdown throw this sentence.
#[test]
fn patch_markdown_rejects_word_comments() {
    let reference = template_docx();
    let markdown = "<!-- COMMENT: hello world this is a note -->";
    let request = one_patch_json("body", markdown, "{}");
    let expected = oracle_patch_error(&reference, &request);
    assert!(expected
        .message
        .contains("Patch markdown does not support Word comments yet"));
    let err = patch_bytes(
        &reference,
        &[("body", markdown)],
        &PatchMarkdownOptions::default(),
    )
    .expect_err("patch comments should fail");
    assert!(err
        .to_string()
        .contains("Patch markdown does not support Word comments yet"));
}

// Validation message from patch option checks.
#[test]
fn rejects_a_non_positive_patch_table_width() {
    let reference = template_docx();
    let err = patch_bytes(
        &reference,
        &[("body", "Hello")],
        &PatchMarkdownOptions {
            table_width_twips: Some(0),
            ..PatchMarkdownOptions::default()
        },
    )
    .expect_err("table width 0 should fail");
    assert!(err
        .to_string()
        .contains("Invalid tableWidthTwips: Must be a positive integer"));
}

// Plan: keepOriginalStyles is accepted and does not override explicit formatting.
#[test]
fn keep_original_styles_keeps_explicit_bold() {
    let reference = template_docx();
    let doc = patch_with(
        &reference,
        &[("body", "Hello **team**.")],
        &PatchMarkdownOptions {
            keep_original_styles: Some(true),
            ..PatchMarkdownOptions::default()
        },
    );
    let bold = doc.paragraphs().into_iter().any(|paragraph| {
        paragraph
            .runs
            .iter()
            .any(|run| run.text.contains("team") && run.bold)
    });
    assert!(bold, "explicit bold should survive keepOriginalStyles");
    assert!(part(&doc, "word/styles.xml").contains("w:styleId=\"CorpTitle\""));
}

// Jest: "preserves reference metadata by default and overrides/merges only supplied fields"
// tests/accessibility-metadata.test.ts
#[test]
fn patch_preserves_metadata_and_merges_supplied_fields() {
    let request = write_request(&format!(
        "{{\"markdown\":{},\"options\":{{\"metadata\":{{\"title\":\"Template title\",\"subject\":\"Preserved subject\",\"company\":\"Template Co\",\"created\":\"2024-01-01T00:00:00Z\",\"custom\":{{\"Build\":\"old\",\"Keep\":\"yes\"}}}}}}}}",
        json_string("{{body}}")
    ));
    let reference = oracle(&["ts-convert".to_string(), request.display().to_string()]);
    let _ = std::fs::remove_file(&request);
    let reference_doc = project(&reference);
    let reference_core = part(&reference_doc, "docProps/core.xml").to_string();
    let reference_styles = part(&reference_doc, "word/styles.xml").to_string();

    let image = format!("![Patched image]({ONE_PX_PNG})");
    let preserved = patch_with(
        &reference,
        &[("body", image.as_str())],
        &PatchMarkdownOptions::default(),
    );
    assert_eq!(part(&preserved, "docProps/core.xml"), reference_core);
    assert!(part(&preserved, "word/document.xml").contains("descr=\"Patched image\""));

    let overridden = patch_with(
        &reference,
        &[("body", "# Updated")],
        &PatchMarkdownOptions {
            metadata: Some(DocumentMetadata {
                title: Some("Override title".to_string()),
                company: Some("Override Co".to_string()),
                language: Some("de-DE".to_string()),
                custom: Some(custom_props(&[("Build", "new"), ("Added", "true")])),
                ..DocumentMetadata::default()
            }),
            ..PatchMarkdownOptions::default()
        },
    );
    let core = part(&overridden, "docProps/core.xml");
    let app = part(&overridden, "docProps/app.xml");
    let custom = part(&overridden, "docProps/custom.xml");
    let document = part(&overridden, "word/document.xml");
    assert!(core.contains("Override title"));
    assert!(core.contains("Preserved subject"));
    assert!(core.contains("2024-01-01T00:00:00.000Z"));
    assert!(core.contains(">de-DE</dc:language>"));
    assert!(app.contains("Override Co"));
    assert!(custom.contains("name=\"Keep\""));
    assert!(custom.contains("name=\"Build\""));
    assert!(custom.contains(">new</vt:lpwstr>"));
    assert!(custom.contains("name=\"Added\""));
    assert!(document.contains("w:lang w:val=\"de-DE\""));
    assert_eq!(part(&overridden, "word/styles.xml"), reference_styles);
}

// Jest: "keeps patched metadata aligned across Blob, ArrayBuffer, and Buffer helpers"
// tests/accessibility-metadata.test.ts
//
// The three wrappers are the same bytes. This asserts the bytes path once.
#[test]
fn patched_metadata_title_on_the_bytes_path() {
    let request = write_request(&format!(
        "{{\"markdown\":{},\"options\":{{\"metadata\":{{\"title\":\"Template\"}}}}}}",
        json_string("{{body}}")
    ));
    let reference = oracle(&["ts-convert".to_string(), request.display().to_string()]);
    let _ = std::fs::remove_file(&request);
    let doc = patch_with(
        &reference,
        &[("body", "Buffer")],
        &PatchMarkdownOptions {
            metadata: Some(DocumentMetadata {
                title: Some("Patched helper".to_string()),
                ..DocumentMetadata::default()
            }),
            ..PatchMarkdownOptions::default()
        },
    );
    assert!(part(&doc, "docProps/core.xml").contains("Patched helper"));
}
