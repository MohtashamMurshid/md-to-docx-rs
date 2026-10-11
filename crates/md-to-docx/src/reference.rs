//! Copy reference-docx styles onto a generated package.
//!
//! The template ZIP is untrusted. Entry metadata is read with
//! [`zip::ZipArchive::by_index_data`] and part bytes with [`ZipArchive::by_name`].
//! `ZipArchive::extract` is not used. Body text, comments, footnotes, macros,
//! and media that the final section does not reference stay in the template.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex, OnceLock};

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use regex::Regex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::MarkdownConversionError;

// Each fixed expression has one cache. Template contents cannot create entries.
macro_rules! cached_regex {
    ($pattern:literal) => {{
        static REGEX: OnceLock<Regex> = OnceLock::new();
        REGEX.get_or_init(|| Regex::new($pattern).expect("constant reference XML regex"))
    }};
}

const MAX_COMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;
const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;

const REQUIRED_PARTS: [&str; 5] = [
    "[Content_Types].xml",
    "_rels/.rels",
    "word/document.xml",
    "word/_rels/document.xml.rels",
    "word/styles.xml",
];

const RELATIONSHIP_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

const EMPTY_NUMBERING_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"></w:numbering>";

const PRESENTATION_TAGS: [&str; 16] = [
    "type",
    "pgSz",
    "pgMar",
    "paperSrc",
    "pgBorders",
    "lnNumType",
    "pgNumType",
    "cols",
    "formProt",
    "vAlign",
    "noEndnote",
    "titlePg",
    "textDirection",
    "bidi",
    "rtlGutter",
    "docGrid",
];

/// Conventional style ids and names from `AUTO_STYLE_CANDIDATES`.
struct AutoStyleCandidates {
    role: &'static str,
    ids: &'static [&'static str],
    names: &'static [&'static str],
}

const AUTO_STYLE_CANDIDATES: &[AutoStyleCandidates] = &[
    AutoStyleCandidates {
        role: "normal",
        ids: &["Normal"],
        names: &["Normal"],
    },
    AutoStyleCandidates {
        role: "title",
        ids: &["Title"],
        names: &["Title"],
    },
    AutoStyleCandidates {
        role: "heading1",
        ids: &["Heading1"],
        names: &["Heading 1", "heading 1"],
    },
    AutoStyleCandidates {
        role: "heading2",
        ids: &["Heading2"],
        names: &["Heading 2", "heading 2"],
    },
    AutoStyleCandidates {
        role: "heading3",
        ids: &["Heading3"],
        names: &["Heading 3", "heading 3"],
    },
    AutoStyleCandidates {
        role: "heading4",
        ids: &["Heading4"],
        names: &["Heading 4", "heading 4"],
    },
    AutoStyleCandidates {
        role: "heading5",
        ids: &["Heading5"],
        names: &["Heading 5", "heading 5"],
    },
    AutoStyleCandidates {
        role: "heading6",
        ids: &["Heading6"],
        names: &["Heading 6", "heading 6"],
    },
    AutoStyleCandidates {
        role: "blockquote",
        ids: &["BlockQuote", "Quote", "IntenseQuote"],
        names: &["Block Quote", "Quote", "Intense Quote"],
    },
    AutoStyleCandidates {
        role: "codeBlock",
        ids: &["CodeBlock", "Code"],
        names: &["Code Block", "Code"],
    },
    AutoStyleCandidates {
        role: "caption",
        ids: &["Caption"],
        names: &["Caption"],
    },
    AutoStyleCandidates {
        role: "listParagraph",
        ids: &["ListParagraph"],
        names: &["List Paragraph"],
    },
    AutoStyleCandidates {
        role: "table",
        ids: &["TableNormal"],
        names: &["Normal Table", "Table Normal"],
    },
    AutoStyleCandidates {
        role: "strong",
        ids: &["Strong"],
        names: &["Strong"],
    },
    AutoStyleCandidates {
        role: "emphasis",
        ids: &["Emphasis"],
        names: &["Emphasis"],
    },
    AutoStyleCandidates {
        role: "inlineCode",
        ids: &["InlineCode", "CodeChar"],
        names: &["Inline Code", "Code Char"],
    },
    AutoStyleCandidates {
        role: "hyperlink",
        ids: &["Hyperlink"],
        names: &["Hyperlink"],
    },
];

struct Limits {
    max_compressed_bytes: u64,
    max_uncompressed_bytes: u64,
    max_entry_uncompressed_bytes: u64,
    max_entries: usize,
}

const DEFAULT_LIMITS: Limits = Limits {
    max_compressed_bytes: MAX_COMPRESSED_BYTES,
    max_uncompressed_bytes: MAX_UNCOMPRESSED_BYTES,
    max_entry_uncompressed_bytes: MAX_ENTRY_UNCOMPRESSED_BYTES,
    max_entries: MAX_ENTRIES,
};

struct ReferenceStyle {
    id: String,
    name: Option<String>,
    style_type: String,
}

#[derive(Clone)]
struct Relationship {
    id: String,
    rel_type: String,
    target: String,
    target_mode: Option<String>,
    xml: String,
}

struct ContentTypes {
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
}

struct LoadedReference {
    content_types_xml: String,
    document_relationships_xml: String,
    styles_xml: String,
    numbering_xml: Option<String>,
    final_section_properties: Option<String>,
}

/// Apply reference styles, numbering, theme, fonts, and final-section layout.
///
/// `template` is an untrusted `.docx`. `generated` is the package that keeps
/// its body. The result is a new ZIP.
pub fn apply_reference(
    template: &[u8],
    generated: &[u8],
) -> Result<Vec<u8>, MarkdownConversionError> {
    apply_reference_with_options(template, generated, None)
}

pub(crate) fn apply_reference_with_options(
    template: &[u8],
    generated: &[u8],
    mode: Option<&crate::options::ReferenceDocxModeOptions>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    ensure_not_aborted(false)?;
    let reference = load_reference(template)?;
    ensure_not_aborted(false)?;
    let preserve = (
        mode.is_none_or(|mode| mode.preserve_page_layout()),
        mode.is_none_or(|mode| mode.preserve_headers_and_footers()),
    );
    apply_presentation(template, generated, &reference, preserve)
}

fn ensure_not_aborted(aborted: bool) -> Result<(), MarkdownConversionError> {
    if aborted {
        return Err(reference_error(
            "Reference DOCX generation was aborted",
            "ABORTED",
            &[],
        ));
    }
    Ok(())
}

fn reference_error(
    message: impl Into<String>,
    code: &str,
    details: &[(&str, &str)],
) -> MarkdownConversionError {
    let mut context = BTreeMap::new();
    context.insert("phase".to_string(), "reference-docx".to_string());
    context.insert("code".to_string(), code.to_string());
    for (key, value) in details {
        context.insert((*key).to_string(), (*value).to_string());
    }
    MarkdownConversionError::with_context(message, context)
}

fn invalid_zip(message: &str) -> MarkdownConversionError {
    reference_error(message, "INVALID_ZIP", &[])
}

fn invalid_package(message: &str) -> MarkdownConversionError {
    reference_error(message, "INVALID_PACKAGE", &[])
}

fn missing_part(entry: &str) -> MarkdownConversionError {
    reference_error(
        format!("Reference DOCX is missing required package part {entry}"),
        "MISSING_PACKAGE_PART",
        &[("entry", entry)],
    )
}

fn malformed_xml(entry: &str) -> MarkdownConversionError {
    reference_error(
        format!("Reference DOCX XML part {entry} is malformed"),
        "MALFORMED_XML",
        &[("entry", entry)],
    )
}

fn prohibited_dtd(entry: &str) -> MarkdownConversionError {
    reference_error(
        format!("Reference DOCX XML part {entry} contains a prohibited DTD or entity declaration"),
        "MALFORMED_XML",
        &[("entry", entry)],
    )
}

fn unsafe_path(entry: &str) -> MarkdownConversionError {
    reference_error(
        format!("Reference DOCX contains an unsafe ZIP entry path: {entry}"),
        "UNSAFE_ENTRY_PATH",
        &[("entry", entry)],
    )
}

fn package_limit(
    message: String,
    limit: u64,
    actual: u64,
    entry: Option<&str>,
) -> MarkdownConversionError {
    let limit_text = limit.to_string();
    let actual_text = actual.to_string();
    let mut details = vec![
        ("limit", limit_text.as_str()),
        ("actual", actual_text.as_str()),
    ];
    if let Some(entry) = entry {
        details.push(("entry", entry));
    }
    reference_error(message, "PACKAGE_LIMIT_EXCEEDED", &details)
}

fn load_reference(bytes: &[u8]) -> Result<LoadedReference, MarkdownConversionError> {
    let names = scan_archive(bytes, &DEFAULT_LIMITS, true)?;
    for part in REQUIRED_PARTS {
        if !names.contains(part) {
            return Err(missing_part(part));
        }
    }
    let content_types_xml = read_xml_part(bytes, "[Content_Types].xml", &DEFAULT_LIMITS)?;
    let root_rels_xml = read_xml_part(bytes, "_rels/.rels", &DEFAULT_LIMITS)?;
    let document_xml = read_xml_part(bytes, "word/document.xml", &DEFAULT_LIMITS)?;
    let document_relationships_xml =
        read_xml_part(bytes, "word/_rels/document.xml.rels", &DEFAULT_LIMITS)?;
    let styles_xml = read_xml_part(bytes, "word/styles.xml", &DEFAULT_LIMITS)?;
    ensure_not_aborted(false)?;
    if !content_types_xml.contains("wordprocessingml.document.main+xml")
        || !root_rels_xml.contains("relationships/officeDocument")
        || !has_word_element(&document_xml, "document")
        || !has_word_element(&styles_xml, "styles")
    {
        return Err(invalid_package(
            "Reference ZIP is not a valid WordprocessingML DOCX package",
        ));
    }
    let numbering_xml = if names.contains("word/numbering.xml") {
        Some(read_xml_part(bytes, "word/numbering.xml", &DEFAULT_LIMITS)?)
    } else {
        None
    };
    Ok(LoadedReference {
        content_types_xml,
        document_relationships_xml,
        styles_xml,
        numbering_xml,
        final_section_properties: last_element(&document_xml, "w:sectPr"),
    })
}

fn scan_archive(
    bytes: &[u8],
    limits: &Limits,
    reference: bool,
) -> Result<BTreeSet<String>, MarkdownConversionError> {
    if bytes.len() < 4 {
        return Err(if reference {
            invalid_zip("Reference DOCX is not a valid ZIP package")
        } else {
            invalid_package("Failed to generate DOCX from reference presentation")
        });
    }
    if bytes.len() as u64 > limits.max_compressed_bytes {
        let message = if reference {
            "Reference DOCX exceeds the compressed package limit".to_string()
        } else {
            "Generated DOCX exceeds the compressed package limit".to_string()
        };
        return Err(package_limit(
            message,
            limits.max_compressed_bytes,
            bytes.len() as u64,
            None,
        ));
    }
    let archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        if reference {
            invalid_zip("Reference DOCX is not a valid ZIP package")
        } else {
            invalid_package("Failed to generate DOCX from reference presentation")
        }
    })?;
    if archive.len() > limits.max_entries {
        return Err(package_limit(
            format!(
                "Reference DOCX contains {} ZIP entries; limit is {}",
                archive.len(),
                limits.max_entries
            ),
            limits.max_entries as u64,
            archive.len() as u64,
            None,
        ));
    }

    let mut names = BTreeSet::new();
    let mut total_uncompressed = 0_u64;
    for index in 0..archive.len() {
        // Central-directory metadata only. The entry is not decompressed here.
        let entry = archive
            .by_index_data(index)
            .map_err(|_| invalid_zip("Reference DOCX is not a valid ZIP package"))?;
        if entry.name_raw().contains(&0) {
            return Err(unsafe_path(&String::from_utf8_lossy(entry.name_raw())));
        }
        let name = match entry.name() {
            Ok(name) => name.into_owned(),
            Err(_) => {
                return Err(unsafe_path(&String::from_utf8_lossy(entry.name_raw())));
            }
        };
        if entry.enclosed_name().is_none() || unsafe_entry_reason(&name) || entry.is_symlink() {
            return Err(unsafe_path(&name));
        }
        let size = entry.size();
        if size > i64::MAX as u64 || size as i64 > MAX_SAFE_INTEGER {
            let actual = size.to_string();
            return Err(reference_error(
                format!("Reference DOCX has invalid size metadata for {name}"),
                "INVALID_ZIP",
                &[("entry", name.as_str()), ("actual", actual.as_str())],
            ));
        }
        if size > limits.max_entry_uncompressed_bytes {
            return Err(package_limit(
                format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
                limits.max_entry_uncompressed_bytes,
                size,
                Some(&name),
            ));
        }
        total_uncompressed = total_uncompressed.saturating_add(size);
        if total_uncompressed > limits.max_uncompressed_bytes {
            return Err(package_limit(
                "Reference DOCX exceeds the total uncompressed package limit".to_string(),
                limits.max_uncompressed_bytes,
                total_uncompressed,
                None,
            ));
        }
        if !entry.is_dir() {
            names.insert(name);
        }
    }
    Ok(names)
}

fn unsafe_entry_reason(name: &str) -> bool {
    let decoded = match decode_uri_component(name) {
        Some(decoded) => decoded,
        None => return true,
    };
    let normalized = decoded.replace('\\', "/");
    decoded.contains('\\')
        || normalized.starts_with('/')
        || drive_prefix(&normalized)
        || normalized.contains('\0')
        || normalized.split('/').any(|segment| segment == "..")
}

fn drive_prefix(value: &str) -> bool {
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) => letter.is_ascii_alphabetic(),
        _ => false,
    }
}

fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn read_xml_part(
    bytes: &[u8],
    name: &str,
    limits: &Limits,
) -> Result<String, MarkdownConversionError> {
    let raw = read_part(bytes, name, limits)?;
    let xml = String::from_utf8(raw).map_err(|_| malformed_xml(name))?;
    assert_safe_xml(&xml, name)?;
    Ok(xml)
}

fn read_part(
    bytes: &[u8],
    name: &str,
    limits: &Limits,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| invalid_zip("Reference DOCX is not a valid ZIP package"))?;
    let mut file = archive.by_name(name).map_err(|_| missing_part(name))?;
    if file.enclosed_name().is_none() || unsafe_entry_reason(&file.name().unwrap_or_default()) {
        let owned = file
            .name()
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| name.to_string());
        return Err(unsafe_path(&owned));
    }
    let declared = file.size();
    if declared > limits.max_entry_uncompressed_bytes {
        return Err(package_limit(
            format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
            limits.max_entry_uncompressed_bytes,
            declared,
            Some(name),
        ));
    }
    let mut buf = Vec::new();
    let cap = limits.max_entry_uncompressed_bytes.saturating_add(1);
    Read::take(&mut file, cap)
        .read_to_end(&mut buf)
        .map_err(|_| {
            reference_error(
                format!("Reference DOCX package part {name} could not be decompressed"),
                "INVALID_ZIP",
                &[("entry", name)],
            )
        })?;
    if buf.len() as u64 > limits.max_entry_uncompressed_bytes {
        return Err(package_limit(
            format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
            limits.max_entry_uncompressed_bytes,
            buf.len() as u64,
            Some(name),
        ));
    }
    Ok(buf)
}

fn assert_safe_xml(xml: &str, entry: &str) -> Result<(), MarkdownConversionError> {
    if contains_ascii_ci(xml, "<!DOCTYPE") || contains_ascii_ci(xml, "<!ENTITY") {
        return Err(prohibited_dtd(entry));
    }
    let mut reader = Reader::from_str(xml);
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(_) | Event::Empty(_)) => saw_root = true,
            Ok(Event::DocType(_)) => return Err(prohibited_dtd(entry)),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(malformed_xml(entry)),
        }
    }
    if !saw_root {
        return Err(malformed_xml(entry));
    }
    Ok(())
}

fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return false;
    }
    bytes
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn has_word_element(xml: &str, tag: &str) -> bool {
    let needle = format!("<w:{tag}");
    let mut rest = xml;
    while let Some(offset) = rest.find(&needle) {
        let after = &rest[offset + needle.len()..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':'));
        if boundary {
            return true;
        }
        rest = &rest[offset + 1..];
    }
    false
}

fn apply_presentation(
    template: &[u8],
    generated: &[u8],
    reference: &LoadedReference,
    preserve: (bool, bool),
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut files = read_generated_files(generated)?;
    let mut document_xml = xml_from_files(&files, "word/document.xml")?;
    let generated_styles_xml = xml_from_files(&files, "word/styles.xml")?;
    let has_generated_numbering = files.contains_key("word/numbering.xml");
    let generated_numbering_xml = if has_generated_numbering {
        xml_from_files(&files, "word/numbering.xml")?
    } else {
        EMPTY_NUMBERING_XML.to_string()
    };
    let mut relationships_xml = xml_from_files(&files, "word/_rels/document.xml.rels")?;
    let mut content_types_xml = xml_from_files(&files, "[Content_Types].xml")?;
    ensure_not_aborted(false)?;

    let reference_styles = parse_styles(&reference.styles_xml)?;
    let style_map = resolve_style_map(&reference_styles, &[], false, true)?;
    let numbering =
        remap_reference_numbering(&generated_numbering_xml, reference.numbering_xml.as_deref())?;
    let remapped_styles = rewrite_style_numbering(&reference.styles_xml, &numbering.num_id_map);
    let styles_xml = merge_styles(&remapped_styles, &generated_styles_xml)?;
    document_xml = apply_style_map_to_document(&document_xml, &style_map);

    if !has_generated_numbering {
        let output_relationships =
            parse_relationships(&relationships_xml, "word/_rels/document.xml.rels")?;
        let existing = output_relationships
            .iter()
            .find(|relationship| relationship.rel_type.ends_with("/numbering"))
            .cloned();
        let id = existing
            .as_ref()
            .map(|relationship| relationship.id.clone())
            .unwrap_or_else(|| next_relationship_id(&output_relationships));
        relationships_xml = replace_or_add_relationship(
            &relationships_xml,
            &Relationship {
                id,
                rel_type: format!("{RELATIONSHIP_NS}numbering"),
                target: "numbering.xml".to_string(),
                target_mode: None,
                xml: String::new(),
            },
            existing.as_ref(),
        )?;
        content_types_xml = add_content_type(
            &content_types_xml,
            "word/numbering.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        );
    }

    let section = adopt_section_presentation(
        template,
        &mut files,
        &document_xml,
        &relationships_xml,
        &content_types_xml,
        reference,
        preserve,
    )?;
    document_xml = section.document_xml;
    relationships_xml = section.relationships_xml;
    content_types_xml = section.content_types_xml;

    for suffix in ["theme", "fontTable"] {
        let imported = import_singleton_part(
            template,
            &mut files,
            &relationships_xml,
            &content_types_xml,
            reference,
            suffix,
        )?;
        relationships_xml = imported.relationships_xml;
        content_types_xml = imported.content_types_xml;
    }

    files.insert("word/document.xml".to_string(), document_xml.into_bytes());
    files.insert("word/styles.xml".to_string(), styles_xml.into_bytes());
    files.insert("word/numbering.xml".to_string(), numbering.xml.into_bytes());
    files.insert(
        "word/_rels/document.xml.rels".to_string(),
        relationships_xml.into_bytes(),
    );
    files.insert(
        "[Content_Types].xml".to_string(),
        content_types_xml.into_bytes(),
    );
    ensure_not_aborted(false)?;
    write_zip(&files)
}

fn read_generated_files(
    bytes: &[u8],
) -> Result<BTreeMap<String, Vec<u8>>, MarkdownConversionError> {
    let names = scan_archive(bytes, &DEFAULT_LIMITS, false)?;
    let mut files = BTreeMap::new();
    for name in names {
        let part = read_part(bytes, &name, &DEFAULT_LIMITS)?;
        files.insert(name, part);
    }
    Ok(files)
}

fn xml_from_files(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> Result<String, MarkdownConversionError> {
    let bytes = files.get(name).ok_or_else(|| missing_part(name))?;
    let xml = String::from_utf8(bytes.clone()).map_err(|_| malformed_xml(name))?;
    assert_safe_xml(&xml, name)?;
    Ok(xml)
}

fn write_zip(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6));
        archive
            .start_file(name, options)
            .map_err(|_| invalid_package("Failed to generate DOCX from reference presentation"))?;
        archive
            .write_all(bytes)
            .map_err(|_| invalid_package("Failed to generate DOCX from reference presentation"))?;
    }
    let cursor = archive
        .finish()
        .map_err(|_| invalid_package("Failed to generate DOCX from reference presentation"))?;
    Ok(cursor.into_inner())
}

fn parse_styles(styles_xml: &str) -> Result<Vec<ReferenceStyle>, MarkdownConversionError> {
    let pattern = cached_regex!(r"(?s)<w:style\b.*?</w:style>");
    let name_pattern = cached_regex!(r"<w:name\b[^>]*/?\s*>");
    let mut styles = Vec::new();
    for found in pattern.find_iter(styles_xml) {
        let xml = found.as_str();
        let open = open_tag(xml);
        let name_element = name_pattern.find(xml).map(|name| name.as_str());
        styles.push(ReferenceStyle {
            id: get_attribute(open, "w:styleId").unwrap_or_default(),
            style_type: get_attribute(open, "w:type").unwrap_or_else(|| "paragraph".to_string()),
            name: name_element.and_then(|element| get_attribute(element, "w:val")),
        });
    }
    let mut seen = BTreeSet::new();
    for style in &styles {
        if style.id.is_empty() || !seen.insert(style.id.clone()) {
            let label = if style.id.is_empty() {
                "(empty)"
            } else {
                style.id.as_str()
            };
            return Err(reference_error(
                format!("Reference styles.xml contains a missing or duplicate style ID: {label}"),
                "MALFORMED_XML",
                &[("entry", "word/styles.xml")],
            ));
        }
    }
    Ok(styles)
}

fn expected_style_type(role: &str) -> &'static str {
    match role {
        "strong" | "emphasis" | "inlineCode" | "hyperlink" => "character",
        "table" => "table",
        _ => "paragraph",
    }
}

fn resolve_style_map(
    styles: &[ReferenceStyle],
    explicit: &[(&str, Option<&str>, Option<&str>, bool)],
    missing_throw: bool,
    duplicate_throw: bool,
) -> Result<BTreeMap<String, String>, MarkdownConversionError> {
    let mut resolved = BTreeMap::new();
    for candidate in AUTO_STYLE_CANDIDATES {
        let selector = explicit.iter().find(|item| item.0 == candidate.role);
        let style_id = resolve_role(styles, candidate, selector, missing_throw, duplicate_throw)?;
        if let Some(style_id) = style_id {
            resolved.insert(candidate.role.to_string(), style_id);
        }
    }
    Ok(resolved)
}

fn resolve_role(
    styles: &[ReferenceStyle],
    candidate: &AutoStyleCandidates,
    selector: Option<&(&str, Option<&str>, Option<&str>, bool)>,
    missing_throw: bool,
    duplicate_throw: bool,
) -> Result<Option<String>, MarkdownConversionError> {
    let role = candidate.role;
    let ids = candidate.ids;
    let names = candidate.names;
    let explicit = selector.is_some();
    let explicit_id = selector.map(|item| item.1);
    let explicit_name = selector.map(|item| item.2);
    let explicit_skip = selector.is_some_and(|item| item.3);
    if explicit_skip {
        return Ok(None);
    }
    let style_index = if explicit {
        let id = explicit_id.flatten();
        let name = explicit_name.flatten();
        if id.is_some() == name.is_some() {
            return Err(reference_error(
                format!(
                    "Reference style selector for {role} must specify exactly one of id or name"
                ),
                "INVALID_INPUT",
                &[("role", role)],
            ));
        }
        let selected = id.or(name).unwrap_or("");
        if selected.trim().is_empty() {
            return Err(reference_error(
                format!("Reference style selector for {role} must not be empty"),
                "INVALID_INPUT",
                &[("role", role)],
            ));
        }
        if let Some(id) = id {
            styles.iter().position(|style| style.id == id)
        } else {
            resolve_style_by_name(styles, selected, duplicate_throw, role)?
        }
    } else {
        let mut found = ids
            .iter()
            .find_map(|id| styles.iter().position(|style| style.id == *id));
        if found.is_none() {
            for name in names {
                if let Some(index) = resolve_style_by_name(styles, name, false, role)? {
                    found = Some(index);
                    break;
                }
            }
        }
        found
    };
    let style = style_index.map(|index| &styles[index]);
    validate_resolved_style(role, style, missing_throw, explicit)
}

fn resolve_style_by_name(
    styles: &[ReferenceStyle],
    name: &str,
    duplicate_throw: bool,
    role: &str,
) -> Result<Option<usize>, MarkdownConversionError> {
    let matches = styles
        .iter()
        .enumerate()
        .filter_map(|(index, style)| (style.name.as_deref() == Some(name)).then_some(index))
        .collect::<Vec<_>>();
    if matches.len() > 1 && duplicate_throw {
        return Err(reference_error(
            format!(
                "Reference DOCX style name {name} is duplicated; select by style ID or set duplicateStyleNameBehavior to \"first\""
            ),
            "DUPLICATE_STYLE_NAME",
            &[("role", role)],
        ));
    }
    Ok(matches.first().copied())
}

fn validate_resolved_style(
    role: &str,
    style: Option<&ReferenceStyle>,
    missing_throw: bool,
    explicit: bool,
) -> Result<Option<String>, MarkdownConversionError> {
    let Some(style) = style else {
        if explicit && missing_throw {
            return Err(reference_error(
                format!("Reference DOCX style for {role} could not be found"),
                "MISSING_STYLE",
                &[("role", role)],
            ));
        }
        return Ok(None);
    };
    let expected = expected_style_type(role);
    if style.style_type != expected {
        if !explicit || !missing_throw {
            return Ok(None);
        }
        return Err(reference_error(
            format!(
                "Reference style {} has type {}; {role} requires {expected}",
                style.id, style.style_type
            ),
            "STYLE_TYPE_MISMATCH",
            &[("role", role)],
        ));
    }
    Ok(Some(style.id.clone()))
}

struct NumberingRemap {
    xml: String,
    num_id_map: BTreeMap<i64, i64>,
}

fn remap_reference_numbering(
    generated_numbering_xml: &str,
    reference_numbering_xml: Option<&str>,
) -> Result<NumberingRemap, MarkdownConversionError> {
    let Some(reference_numbering_xml) = reference_numbering_xml else {
        return Ok(NumberingRemap {
            xml: generated_numbering_xml.to_string(),
            num_id_map: BTreeMap::new(),
        });
    };
    let generated_abstract_ids = ids_from_xml(
        generated_numbering_xml,
        r#"<w:abstractNum\b[^>]*w:abstractNumId="(\d+)""#,
    );
    let generated_num_ids =
        ids_from_xml(generated_numbering_xml, r#"<w:num\b[^>]*w:numId="(\d+)""#);
    let next_abstract_id = generated_abstract_ids.iter().copied().max().unwrap_or(0) + 1;
    let mut next_num_id = generated_num_ids.iter().copied().max().unwrap_or(0) + 1;
    let mut abstract_id_map = BTreeMap::new();
    let abstract_pattern = cached_regex!(r"(?s)<w:abstractNum\b.*?</w:abstractNum>");
    let mut abstract_definitions = Vec::new();
    for (mapped, found) in
        (next_abstract_id..).zip(abstract_pattern.find_iter(reference_numbering_xml))
    {
        let definition = found.as_str();
        let open = open_tag(definition);
        let id = get_attribute(open, "w:abstractNumId")
            .as_deref()
            .and_then(parse_safe_int)
            .ok_or_else(|| {
                reference_error(
                    "Reference numbering.xml contains an invalid abstract numbering ID",
                    "MALFORMED_XML",
                    &[("entry", "word/numbering.xml")],
                )
            })?;
        abstract_id_map.insert(id, mapped);
        abstract_definitions.push(replace_attribute(
            definition,
            "w:abstractNumId",
            &mapped.to_string(),
        ));
    }

    let num_pattern = cached_regex!(r"(?s)<w:num\b.*?</w:num>");
    let abstract_ref_pattern = cached_regex!(r"<w:abstractNumId\b[^>]*/?\s*>");
    let mut num_id_map = BTreeMap::new();
    let mut num_definitions = Vec::new();
    for found in num_pattern.find_iter(reference_numbering_xml) {
        let definition = found.as_str();
        let open = open_tag(definition);
        let id = get_attribute(open, "w:numId")
            .as_deref()
            .and_then(parse_safe_int);
        let abstract_element = abstract_ref_pattern
            .find(definition)
            .map(|item| item.as_str());
        let abstract_id = abstract_element
            .and_then(|element| get_attribute(element, "w:val"))
            .as_deref()
            .and_then(parse_safe_int);
        let Some(id) = id else {
            return Err(invalid_numbering());
        };
        let Some(abstract_id) = abstract_id else {
            return Err(invalid_numbering());
        };
        if !abstract_id_map.contains_key(&abstract_id) {
            return Err(invalid_numbering());
        }
        let mapped = next_num_id;
        next_num_id += 1;
        num_id_map.insert(id, mapped);
        let mut rewritten = replace_attribute(definition, "w:numId", &mapped.to_string());
        if let Some(element) = abstract_ref_pattern.find(&rewritten) {
            let replaced = replace_attribute(
                element.as_str(),
                "w:val",
                &abstract_id_map[&abstract_id].to_string(),
            );
            let start = element.start();
            let end = element.end();
            rewritten = format!("{}{replaced}{}", &rewritten[..start], &rewritten[end..]);
        }
        num_definitions.push(rewritten);
    }
    let additions = abstract_definitions
        .into_iter()
        .chain(num_definitions)
        .collect::<String>();
    if !generated_numbering_xml.contains("</w:numbering>") {
        return Err(malformed_xml("word/numbering.xml"));
    }
    Ok(NumberingRemap {
        xml: generated_numbering_xml
            .replace("</w:numbering>", &format!("{additions}</w:numbering>")),
        num_id_map,
    })
}

fn invalid_numbering() -> MarkdownConversionError {
    reference_error(
        "Reference numbering.xml contains an invalid numbering definition",
        "MALFORMED_XML",
        &[("entry", "word/numbering.xml")],
    )
}

fn ids_from_xml(xml: &str, pattern: &str) -> Vec<i64> {
    let pattern = tag_regex(pattern);
    pattern
        .captures_iter(xml)
        .filter_map(|caps| parse_safe_int(&caps[1]))
        .collect()
}

fn parse_safe_int(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let parsed = trimmed.parse::<i64>().ok()?;
    (parsed.unsigned_abs() <= MAX_SAFE_INTEGER as u64).then_some(parsed)
}

fn rewrite_style_numbering(styles_xml: &str, num_id_map: &BTreeMap<i64, i64>) -> String {
    if num_id_map.is_empty() {
        return styles_xml.to_string();
    }
    let pattern = cached_regex!(r"<w:numId\b[^>]*/?\s*>");
    pattern
        .replace_all(styles_xml, |caps: &regex::Captures| {
            let element = &caps[0];
            let Some(id) = get_attribute(element, "w:val")
                .as_deref()
                .and_then(parse_safe_int)
            else {
                return element.to_string();
            };
            match num_id_map.get(&id) {
                Some(mapped) => replace_attribute(element, "w:val", &mapped.to_string()),
                None => element.to_string(),
            }
        })
        .into_owned()
}

fn merge_styles(
    reference_styles_xml: &str,
    generated_styles_xml: &str,
) -> Result<String, MarkdownConversionError> {
    let reference_styles = parse_styles(reference_styles_xml)?;
    let mut reference_ids = reference_styles
        .iter()
        .map(|style| style.id.clone())
        .collect::<BTreeSet<_>>();
    let pattern = cached_regex!(r"(?s)<w:style\b.*?</w:style>");
    let mut appended = String::new();
    for found in pattern.find_iter(generated_styles_xml) {
        let xml = found.as_str();
        let id = get_attribute(open_tag(xml), "w:styleId");
        if let Some(id) = id {
            if reference_ids.insert(id) {
                appended.push_str(xml);
            }
        }
    }
    if !reference_styles_xml.contains("</w:styles>") {
        return Err(reference_error(
            "Reference styles.xml is malformed",
            "MALFORMED_XML",
            &[("entry", "word/styles.xml")],
        ));
    }
    Ok(reference_styles_xml.replace("</w:styles>", &format!("{appended}</w:styles>")))
}

fn apply_style_map_to_document(document_xml: &str, style_map: &BTreeMap<String, String>) -> String {
    let paragraphs = cached_regex!(r"(?s)<w:p\b.*?</w:p>");
    let mut rewritten = paragraphs
        .replace_all(document_xml, |caps: &regex::Captures| {
            let paragraph = &caps[0];
            let role = paragraph_role(paragraph);
            let style_id = role.and_then(|role| style_map.get(role));
            let mut output = paragraph.to_string();
            if let Some(style_id) = style_id {
                output = replace_paragraph_style(&output, style_id);
                output = strip_paragraph_direct_formatting(&output);
                output = map_runs(&output, strip_base_run_formatting);
            }
            apply_run_styles(&output, style_map)
        })
        .into_owned();
    if let Some(table_style) = style_map.get("table") {
        rewritten = apply_table_style(&rewritten, table_style);
    }
    rewritten
}

fn paragraph_role(paragraph: &str) -> Option<&'static str> {
    let p_pr = first_capture(paragraph, r"(?s)<w:pPr\b[^>]*>.*?</w:pPr>").unwrap_or("");
    let style_pattern = cached_regex!(r#"<w:pStyle\b[^>]*w:val=["']([^"']+)["']"#);
    let style_ids = style_pattern
        .captures_iter(p_pr)
        .map(|caps| caps[1].to_string())
        .collect::<Vec<_>>();
    for level in 1..=6 {
        let heading = format!("Heading{level}");
        let numeric = level.to_string();
        if style_ids.iter().any(|id| id == &heading || id == &numeric) {
            return Some(match level {
                1 => "heading1",
                2 => "heading2",
                3 => "heading3",
                4 => "heading4",
                5 => "heading5",
                _ => "heading6",
            });
        }
    }
    if style_ids.iter().any(|id| id == "Title") {
        return Some("title");
    }
    if style_ids.iter().any(|id| id == "Caption") {
        return Some("caption");
    }
    if style_ids.iter().any(|id| id == "ListParagraph") {
        return Some("listParagraph");
    }
    if word_tag_present(p_pr, "shd") {
        return Some("codeBlock");
    }
    if word_tag_present(p_pr, "pBdr") {
        return Some("blockquote");
    }
    if style_ids.is_empty() {
        return Some("normal");
    }
    None
}

fn replace_paragraph_style(paragraph: &str, style_id: &str) -> String {
    let style_element = format!(r#"<w:pStyle w:val="{}"/>"#, escape_xml(style_id));
    replace_container_style(paragraph, "w:p", "w:pPr", "w:pStyle", &style_element)
}

fn replace_run_style(run: &str, style_id: &str) -> String {
    let style_element = format!(r#"<w:rStyle w:val="{}"/>"#, escape_xml(style_id));
    replace_container_style(run, "w:r", "w:rPr", "w:rStyle", &style_element)
}

fn apply_table_style(table_xml: &str, style_id: &str) -> String {
    let tables = cached_regex!(r"(?s)<w:tbl\b.*?</w:tbl>");
    let style_element = format!(r#"<w:tblStyle w:val="{}"/>"#, escape_xml(style_id));
    tables
        .replace_all(table_xml, |caps: &regex::Captures| {
            replace_container_style(&caps[0], "w:tbl", "w:tblPr", "w:tblStyle", &style_element)
        })
        .into_owned()
}

fn replace_container_style(
    xml: &str,
    element: &str,
    properties: &str,
    style_tag: &str,
    style_element: &str,
) -> String {
    let properties_pattern = tag_regex(&format!(r"(?s)<{properties}\b[^>]*>.*?</{properties}>"));
    if let Some(found) = properties_pattern.find(xml) {
        let mut properties_xml = remove_leaf_tag(found.as_str(), style_tag);
        properties_xml = insert_after_open_tag(&properties_xml, properties, style_element);
        splice(xml, found.start(), found.end(), &properties_xml)
    } else {
        insert_after_open_tag(
            xml,
            element,
            &format!("<{properties}>{style_element}</{properties}>"),
        )
    }
}

fn strip_paragraph_direct_formatting(paragraph: &str) -> String {
    let pattern = cached_regex!(r"(?s)<w:pPr\b[^>]*>.*?</w:pPr>");
    pattern
        .replace(paragraph, |caps: &regex::Captures| {
            let mut rewritten = caps[0].to_string();
            for tag in [
                "bidi",
                "spacing",
                "jc",
                "pBdr",
                "shd",
                "ind",
                "keepNext",
                "keepLines",
                "widowControl",
            ] {
                rewritten = remove_word_element(&rewritten, tag);
            }
            rewritten
        })
        .into_owned()
}

fn strip_base_run_formatting(run: &str) -> String {
    let pattern = cached_regex!(r"<w:(?:color|sz|szCs|rFonts|rtl)\b[^>]*/?\s*>");
    pattern.replace_all(run, "").into_owned()
}

fn strip_inline_code_direct(run: &str) -> String {
    let pattern = cached_regex!(r"<w:(?:shd|rFonts|color|sz|szCs)\b[^>]*/?\s*>");
    pattern.replace_all(run, "").into_owned()
}

fn apply_run_styles(paragraph: &str, style_map: &BTreeMap<String, String>) -> String {
    let mut rewritten = paragraph.to_string();
    if let Some(hyperlink_style) = style_map.get("hyperlink") {
        let links = cached_regex!(r"(?s)<w:hyperlink\b.*?</w:hyperlink>");
        let style_id = hyperlink_style.clone();
        rewritten = links
            .replace_all(&rewritten, |caps: &regex::Captures| {
                map_runs(&caps[0], |run| replace_run_style(run, &style_id))
            })
            .into_owned();
    }
    let hyperlink_id = style_map.get("hyperlink").cloned();
    map_runs(&rewritten, |original| {
        let is_inline_code = word_tag_present(original, "shd") || mono_font(original);
        let is_strong = word_tag_present(original, "b");
        let is_emphasis = word_tag_present(original, "i");
        let existing = run_style_value(original);
        let is_hyperlink = existing.as_deref() == Some("Hyperlink")
            || hyperlink_id
                .as_deref()
                .is_some_and(|style| existing.as_deref() == Some(style));
        let role = if is_inline_code {
            Some("inlineCode")
        } else if is_hyperlink {
            Some("hyperlink")
        } else if is_strong {
            Some("strong")
        } else if is_emphasis {
            Some("emphasis")
        } else {
            None
        };
        let mut run = original.to_string();
        if let Some(role) = role {
            if let Some(style_id) = style_map.get(role) {
                run = replace_run_style(&run, style_id);
                if role == "inlineCode" {
                    run = strip_inline_code_direct(&run);
                }
            }
        }
        run
    })
}

fn map_runs(xml: &str, mut mapper: impl FnMut(&str) -> String) -> String {
    let runs = cached_regex!(r"(?s)<w:r\b.*?</w:r>");
    runs.replace_all(xml, |caps: &regex::Captures| mapper(&caps[0]))
        .into_owned()
}

fn mono_font(run: &str) -> bool {
    let pattern = cached_regex!(r"<w:rFonts\b[^>]*(?:Courier|Consolas|monospace)");
    pattern.is_match(run)
}

fn run_style_value(run: &str) -> Option<String> {
    let pattern = cached_regex!(r#"<w:rStyle\b[^>]*w:val=["']([^"']+)["']"#);
    pattern.captures(run).map(|caps| caps[1].to_string())
}

fn parse_relationships(
    xml: &str,
    entry: &str,
) -> Result<Vec<Relationship>, MarkdownConversionError> {
    if !xml.contains("<Relationships") {
        return Err(reference_error(
            format!("Relationship part {entry} is malformed"),
            "MALFORMED_XML",
            &[("entry", entry)],
        ));
    }
    let pattern = cached_regex!(r"<Relationship\b[^>]*/?\s*>");
    let mut relationships = Vec::new();
    for found in pattern.find_iter(xml) {
        let relation = found.as_str();
        let id = get_attribute(relation, "Id");
        let rel_type = get_attribute(relation, "Type");
        let target = get_attribute(relation, "Target");
        let (Some(id), Some(rel_type), Some(target)) = (id, rel_type, target) else {
            return Err(reference_error(
                format!("Relationship part {entry} is malformed"),
                "MALFORMED_XML",
                &[("entry", entry)],
            ));
        };
        relationships.push(Relationship {
            id,
            rel_type,
            target,
            target_mode: get_attribute(relation, "TargetMode"),
            xml: relation.to_string(),
        });
    }
    Ok(relationships)
}

fn parse_content_types(xml: &str) -> ContentTypes {
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    {
        let pattern = cached_regex!(r"<Default\b[^>]*/?\s*>");
        for found in pattern.find_iter(xml) {
            let element = found.as_str();
            let extension = get_attribute(element, "Extension");
            let content_type = get_attribute(element, "ContentType");
            if let (Some(extension), Some(content_type)) = (extension, content_type) {
                defaults.insert(extension.to_ascii_lowercase(), content_type);
            }
        }
    }
    {
        let pattern = cached_regex!(r"<Override\b[^>]*/?\s*>");
        for found in pattern.find_iter(xml) {
            let element = found.as_str();
            let part_name = get_attribute(element, "PartName");
            let content_type = get_attribute(element, "ContentType");
            if let (Some(part_name), Some(content_type)) = (part_name, content_type) {
                overrides.insert(part_name, content_type);
            }
        }
    }
    ContentTypes {
        defaults,
        overrides,
    }
}

fn relationship_part_path(part: &str) -> String {
    match part.rfind('/') {
        Some(index) => format!("{}_rels/{}.rels", &part[..=index], &part[index + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

fn resolve_part_target(source_part: &str, target: &str) -> Result<String, MarkdownConversionError> {
    if unsafe_entry_reason(target) || target.starts_with('/') {
        return Err(reference_error(
            format!("Relationship target contains an unsafe package path: {target}"),
            "UNSAFE_ENTRY_PATH",
            &[("entry", target)],
        ));
    }
    let base = match source_part.rfind('/') {
        Some(index) => &source_part[..=index],
        None => "",
    };
    let mut resolved = Vec::new();
    for segment in format!("{base}{target}").split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if resolved.pop().is_none() {
                return Err(reference_error(
                    format!("Relationship target escapes the DOCX package: {target}"),
                    "UNSAFE_ENTRY_PATH",
                    &[("entry", target)],
                ));
            }
        } else {
            resolved.push(segment.to_string());
        }
    }
    Ok(resolved.join("/"))
}

fn relative_target(from_part: &str, to_part: &str) -> String {
    let from = from_part.split('/').collect::<Vec<_>>();
    let from = if from.is_empty() {
        &[][..]
    } else {
        &from[..from.len() - 1]
    };
    let to = to_part.split('/').collect::<Vec<_>>();
    let mut common = 0;
    while common < from.len() && common < to.len() && from[common] == to[common] {
        common += 1;
    }
    format!(
        "{}{}",
        "../".repeat(from.len() - common),
        to[common..].join("/")
    )
}

fn content_type_for_part(types: &ContentTypes, part: &str) -> Option<String> {
    if let Some(content_type) = types.overrides.get(&format!("/{part}")) {
        return Some(content_type.clone());
    }
    if !part.contains('.') {
        return None;
    }
    part.rsplit('.')
        .next()
        .map(|value| value.to_ascii_lowercase())
        .and_then(|extension| types.defaults.get(&extension).cloned())
}

fn add_content_type(xml: &str, part: &str, content_type: &str) -> String {
    let types = parse_content_types(xml);
    if types.overrides.contains_key(&format!("/{part}")) {
        return xml.to_string();
    }
    let entry = format!(
        r#"<Override PartName="/{}" ContentType="{}"/>"#,
        escape_xml(part),
        escape_xml(content_type)
    );
    xml.replace("</Types>", &format!("{entry}</Types>"))
}

fn add_default_content_types(output_xml: &str, reference_xml: &str) -> String {
    let mut output = parse_content_types(output_xml);
    let reference = parse_content_types(reference_xml);
    let mut rewritten = output_xml.to_string();
    for (extension, content_type) in reference.defaults {
        if let std::collections::btree_map::Entry::Vacant(entry) = output.defaults.entry(extension)
        {
            rewritten = rewritten.replace(
                "</Types>",
                &format!(
                    r#"<Default Extension="{}" ContentType="{}"/></Types>"#,
                    escape_xml(entry.key()),
                    escape_xml(&content_type)
                ),
            );
            entry.insert(content_type);
        }
    }
    rewritten
}

fn next_relationship_id(relationships: &[Relationship]) -> String {
    let used = relationships
        .iter()
        .map(|relationship| relationship.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut counter = 1_u32;
    loop {
        let id = format!("rIdReference{counter}");
        if !used.contains(id.as_str()) {
            return id;
        }
        counter += 1;
    }
}

fn relationship_xml(relationship: &Relationship) -> String {
    let mode = relationship
        .target_mode
        .as_ref()
        .map(|mode| format!(r#" TargetMode="{}""#, escape_xml(mode)));
    format!(
        r#"<Relationship Id="{}" Type="{}" Target="{}"{}/>"#,
        escape_xml(&relationship.id),
        escape_xml(&relationship.rel_type),
        escape_xml(&relationship.target),
        mode.unwrap_or_default()
    )
}

fn replace_or_add_relationship(
    xml: &str,
    relationship: &Relationship,
    existing: Option<&Relationship>,
) -> Result<String, MarkdownConversionError> {
    let replacement = relationship_xml(relationship);
    if let Some(existing) = existing {
        return Ok(xml.replacen(&existing.xml, &replacement, 1));
    }
    if !xml.contains("</Relationships>") {
        return Err(malformed_xml("word/_rels/document.xml.rels"));
    }
    Ok(xml.replace(
        "</Relationships>",
        &format!("{replacement}</Relationships>"),
    ))
}

fn import_part(
    template: &[u8],
    files: &mut BTreeMap<String, Vec<u8>>,
    content_types_xml: &str,
    source_part: &str,
    imported: &mut BTreeMap<String, String>,
    allowed_internal: &BTreeSet<&str>,
) -> Result<String, MarkdownConversionError> {
    if let Some(existing) = imported.get(source_part) {
        return Ok(existing.clone());
    }
    if unsafe_entry_reason(source_part) {
        return Err(reference_error(
            format!("Reference relationship target is missing or unsafe: {source_part}"),
            "MISSING_PACKAGE_PART",
            &[("entry", source_part)],
        ));
    }
    let bytes = read_part(template, source_part, &DEFAULT_LIMITS).map_err(|_| {
        reference_error(
            format!("Reference relationship target is missing or unsafe: {source_part}"),
            "MISSING_PACKAGE_PART",
            &[("entry", source_part)],
        )
    })?;
    let destination = format!("word/reference-package/{source_part}");
    imported.insert(source_part.to_string(), destination.clone());
    if source_part.to_ascii_lowercase().ends_with(".xml") {
        let xml = String::from_utf8(bytes.clone()).map_err(|_| malformed_xml(source_part))?;
        assert_safe_xml(&xml, source_part)?;
    }
    files.insert(destination.clone(), bytes);
    let reference_types = parse_content_types(content_types_xml);
    if content_type_for_part(&reference_types, source_part).is_none() {
        return Err(reference_error(
            format!("Reference package has no content type for {source_part}"),
            "INVALID_PACKAGE",
            &[("entry", source_part)],
        ));
    }
    let rels_path = relationship_part_path(source_part);
    if part_exists(template, &rels_path)? {
        let rels_xml = read_xml_part(template, &rels_path, &DEFAULT_LIMITS)?;
        let relationships = parse_relationships(&rels_xml, &rels_path)?;
        for relationship in &relationships {
            let type_suffix = relationship
                .rel_type
                .get(RELATIONSHIP_NS.len()..)
                .unwrap_or("");
            if relationship
                .target_mode
                .as_deref()
                .is_some_and(|mode| mode.eq_ignore_ascii_case("external"))
            {
                if type_suffix != "hyperlink" {
                    return Err(reference_error(
                        format!(
                            "Unsupported external reference relationship type: {}",
                            relationship.rel_type
                        ),
                        "UNSUPPORTED_RELATIONSHIP",
                        &[
                            ("entry", rels_path.as_str()),
                            ("relationshipType", relationship.rel_type.as_str()),
                        ],
                    ));
                }
                continue;
            }
            if !allowed_internal.contains(type_suffix) {
                return Err(reference_error(
                    format!(
                        "Unsupported reference relationship type: {}",
                        relationship.rel_type
                    ),
                    "UNSUPPORTED_RELATIONSHIP",
                    &[
                        ("entry", rels_path.as_str()),
                        ("relationshipType", relationship.rel_type.as_str()),
                    ],
                ));
            }
            let target_part = resolve_part_target(source_part, &relationship.target)?;
            import_part(
                template,
                files,
                content_types_xml,
                &target_part,
                imported,
                allowed_internal,
            )?;
        }
        files.insert(relationship_part_path(&destination), rels_xml.into_bytes());
    }
    Ok(destination)
}

fn part_exists(bytes: &[u8], name: &str) -> Result<bool, MarkdownConversionError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| invalid_zip("Reference DOCX is not a valid ZIP package"))?;
    let exists = archive.by_name(name).is_ok();
    Ok(exists)
}

struct SectionAdoption {
    document_xml: String,
    relationships_xml: String,
    content_types_xml: String,
}

fn adopt_section_presentation(
    template: &[u8],
    files: &mut BTreeMap<String, Vec<u8>>,
    document_xml: &str,
    relationships_xml: &str,
    content_types_xml: &str,
    reference: &LoadedReference,
    preserve: (bool, bool),
) -> Result<SectionAdoption, MarkdownConversionError> {
    let (preserve_page_layout, preserve_headers_and_footers) = preserve;
    let Some(section) = reference.final_section_properties.as_deref() else {
        return Ok(SectionAdoption {
            document_xml: document_xml.to_string(),
            relationships_xml: relationships_xml.to_string(),
            content_types_xml: content_types_xml.to_string(),
        });
    };
    let mut relationships_xml = relationships_xml.to_string();
    let mut content_types_xml =
        add_default_content_types(content_types_xml, &reference.content_types_xml);
    let reference_relationships = parse_relationships(
        &reference.document_relationships_xml,
        "word/_rels/document.xml.rels",
    )?;
    let mut imported = BTreeMap::new();
    let mut rewritten_references = String::new();
    if preserve_headers_and_footers {
        let pattern = cached_regex!(r"<w:(headerReference|footerReference)\b[^>]*/?\s*>");
        for found in pattern.captures_iter(section) {
            let kind = found.get(1).map(|item| item.as_str()).unwrap_or("");
            let element = found.get(0).map(|item| item.as_str()).unwrap_or("");
            let reference_id = get_attribute(element, "r:id");
            let relationship = reference_relationships
                .iter()
                .find(|candidate| Some(candidate.id.as_str()) == reference_id.as_deref());
            let expected_suffix = if kind == "headerReference" {
                "header"
            } else {
                "footer"
            };
            let supported = relationship.is_some_and(|relationship| {
                relationship
                    .rel_type
                    .ends_with(&format!("/{expected_suffix}"))
                    && !relationship
                        .target_mode
                        .as_deref()
                        .is_some_and(|mode| mode.eq_ignore_ascii_case("external"))
            });
            if !supported {
                let rel_type = relationship.map(|item| item.rel_type.as_str());
                let mut details = Vec::new();
                if let Some(rel_type) = rel_type {
                    details.push(("relationshipType", rel_type));
                }
                return Err(reference_error(
                    format!("Reference {expected_suffix} relationship is missing or unsupported"),
                    "UNSUPPORTED_RELATIONSHIP",
                    &details,
                ));
            }
            let relationship = relationship.expect("supported relationship");
            let source_part = resolve_part_target("word/document.xml", &relationship.target)?;
            let allowed = BTreeSet::from(["image"]);
            let destination = import_part(
                template,
                files,
                &reference.content_types_xml,
                &source_part,
                &mut imported,
                &allowed,
            )?;
            let output_relationships =
                parse_relationships(&relationships_xml, "word/_rels/document.xml.rels")?;
            let id = next_relationship_id(&output_relationships);
            relationships_xml = replace_or_add_relationship(
                &relationships_xml,
                &Relationship {
                    id: id.clone(),
                    rel_type: relationship.rel_type.clone(),
                    target: relative_target("word/document.xml", &destination),
                    target_mode: None,
                    xml: String::new(),
                },
                None,
            )?;
            rewritten_references.push_str(&replace_attribute(element, "r:id", &id));
        }
    }
    let reference_types = parse_content_types(&reference.content_types_xml);
    for (source, destination) in &imported {
        if let Some(content_type) = content_type_for_part(&reference_types, source) {
            content_types_xml = add_content_type(&content_types_xml, destination, &content_type);
        }
    }
    let layout = if preserve_page_layout {
        presentation_tags(section)
    } else {
        String::new()
    };
    let sections = cached_regex!(r"(?s)<w:sectPr\b.*?</w:sectPr>");
    let document_xml = sections
        .replace_all(document_xml, |caps: &regex::Captures| {
            let mut rewritten = caps[0].to_string();
            if preserve_page_layout {
                rewritten = remove_presentation_tags(&rewritten);
            }
            if preserve_headers_and_footers {
                let refs = cached_regex!(r"<w:(?:headerReference|footerReference)\b[^>]*/?\s*>");
                rewritten = refs.replace_all(&rewritten, "").into_owned();
            }
            let additions = format!("{rewritten_references}{layout}");
            insert_after_open_tag(&rewritten, "w:sectPr", &additions)
        })
        .into_owned();
    Ok(SectionAdoption {
        document_xml,
        relationships_xml,
        content_types_xml,
    })
}

fn presentation_tags(section_properties: &str) -> String {
    let mut output = String::new();
    for name in PRESENTATION_TAGS {
        let pattern = tag_regex(&format!(r"(?s)<w:{name}\b(?:[^>]*/>|[^>]*>.*?</w:{name}>)"));
        if let Some(found) = pattern.find(section_properties) {
            output.push_str(found.as_str());
        }
    }
    output
}

fn remove_presentation_tags(section_properties: &str) -> String {
    let mut rewritten = section_properties.to_string();
    for name in PRESENTATION_TAGS {
        rewritten = remove_word_element(&rewritten, name);
    }
    rewritten
}

struct SingletonImport {
    relationships_xml: String,
    content_types_xml: String,
}

fn import_singleton_part(
    template: &[u8],
    files: &mut BTreeMap<String, Vec<u8>>,
    relationships_xml: &str,
    content_types_xml: &str,
    reference: &LoadedReference,
    relation_suffix: &str,
) -> Result<SingletonImport, MarkdownConversionError> {
    let reference_relationship = parse_relationships(
        &reference.document_relationships_xml,
        "word/_rels/document.xml.rels",
    )?
    .into_iter()
    .find(|relationship| {
        relationship
            .rel_type
            .ends_with(&format!("/{relation_suffix}"))
    });
    let Some(reference_relationship) = reference_relationship else {
        return Ok(SingletonImport {
            relationships_xml: relationships_xml.to_string(),
            content_types_xml: content_types_xml.to_string(),
        });
    };
    if reference_relationship.target_mode.is_some() {
        return Ok(SingletonImport {
            relationships_xml: relationships_xml.to_string(),
            content_types_xml: content_types_xml.to_string(),
        });
    }
    let source_part = resolve_part_target("word/document.xml", &reference_relationship.target)?;
    let mut imported = BTreeMap::new();
    let allowed = if relation_suffix == "fontTable" {
        BTreeSet::from(["font"])
    } else {
        BTreeSet::new()
    };
    let destination = import_part(
        template,
        files,
        &reference.content_types_xml,
        &source_part,
        &mut imported,
        &allowed,
    )?;
    let output_relationships =
        parse_relationships(relationships_xml, "word/_rels/document.xml.rels")?;
    let existing = output_relationships
        .iter()
        .find(|relationship| {
            relationship
                .rel_type
                .ends_with(&format!("/{relation_suffix}"))
        })
        .cloned();
    let id = existing
        .as_ref()
        .map(|relationship| relationship.id.clone())
        .unwrap_or_else(|| next_relationship_id(&output_relationships));
    let relationships_xml = replace_or_add_relationship(
        relationships_xml,
        &Relationship {
            id,
            rel_type: reference_relationship.rel_type,
            target: relative_target("word/document.xml", &destination),
            target_mode: None,
            xml: String::new(),
        },
        existing.as_ref(),
    )?;
    let mut content_types_xml =
        add_default_content_types(content_types_xml, &reference.content_types_xml);
    let reference_types = parse_content_types(&reference.content_types_xml);
    for (source, imported_destination) in &imported {
        if let Some(content_type) = content_type_for_part(&reference_types, source) {
            content_types_xml =
                add_content_type(&content_types_xml, imported_destination, &content_type);
        }
    }
    Ok(SingletonImport {
        relationships_xml,
        content_types_xml,
    })
}

// The callers construct these expressions only from internal XML tag names.
// Keep the cache bounded even if more patterns are added later.
fn tag_regex(pattern: &str) -> Arc<Regex> {
    const MAX_PATTERNS: usize = 128;
    static CACHE: OnceLock<Mutex<BTreeMap<String, Arc<Regex>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .expect("reference regex cache lock");
    if let Some(regex) = cache.get(pattern) {
        return Arc::clone(regex);
    }
    let regex = Arc::new(Regex::new(pattern).expect("internal reference XML regex"));
    if cache.len() < MAX_PATTERNS {
        cache.insert(pattern.to_string(), Arc::clone(&regex));
    }
    regex
}

fn last_element(xml: &str, tag: &str) -> Option<String> {
    let pattern = tag_regex(&format!(r"(?s)<{tag}\b.*?</{tag}>"));
    pattern
        .find_iter(xml)
        .last()
        .map(|found| found.as_str().to_string())
}

fn first_capture<'a>(xml: &'a str, pattern: &str) -> Option<&'a str> {
    tag_regex(pattern).find(xml).map(|found| found.as_str())
}

fn open_tag(xml: &str) -> &str {
    match xml.find('>') {
        Some(index) => &xml[..=index],
        None => xml,
    }
}

fn word_tag_present(xml: &str, tag: &str) -> bool {
    has_prefixed_tag(xml, &format!("w:{tag}"))
}

fn has_prefixed_tag(xml: &str, needle_body: &str) -> bool {
    let needle = format!("<{needle_body}");
    let mut rest = xml;
    while let Some(offset) = rest.find(&needle) {
        let after = &rest[offset + needle.len()..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' || ch == '-'));
        if boundary {
            return true;
        }
        rest = &rest[offset + 1..];
    }
    false
}

fn remove_word_element(xml: &str, tag: &str) -> String {
    let pattern = tag_regex(&format!(r"(?s)<w:{tag}\b(?:[^>]*/>|[^>]*>.*?</w:{tag}>)"));
    pattern.replace_all(xml, "").into_owned()
}

fn remove_leaf_tag(xml: &str, tag: &str) -> String {
    let pattern = tag_regex(&format!(r"<{tag}\b[^>]*/?\s*>"));
    pattern.replace_all(xml, "").into_owned()
}

fn insert_after_open_tag(xml: &str, tag: &str, insertion: &str) -> String {
    let pattern = tag_regex(&format!(r"^<{tag}\b[^>]*>"));
    if let Some(found) = pattern.find(xml) {
        splice(xml, found.end(), found.end(), insertion)
    } else {
        xml.to_string()
    }
}

fn splice(xml: &str, start: usize, end: usize, insertion: &str) -> String {
    let mut output = String::with_capacity(xml.len() - (end - start) + insertion.len());
    output.push_str(&xml[..start]);
    output.push_str(insertion);
    output.push_str(&xml[end..]);
    output
}

fn get_attribute(xml: &str, name: &str) -> Option<String> {
    let (_name_at, value_at, value_end) = find_attribute(xml, name)?;
    Some(decode_xml(&xml[value_at..value_end]))
}

fn replace_attribute(xml: &str, name: &str, value: &str) -> String {
    let encoded = escape_xml(value);
    if let Some((name_at, _value_at, value_end)) = find_attribute(xml, name) {
        let mut output = String::new();
        output.push_str(&xml[..name_at]);
        output.push_str(name);
        output.push_str("=\"");
        output.push_str(&encoded);
        output.push('"');
        output.push_str(&xml[value_end + 1..]);
        return output;
    }
    if xml.ends_with("/>") {
        let bytes = xml.as_bytes();
        let mut cut = bytes.len() - 2;
        while cut > 0 && bytes[cut - 1].is_ascii_whitespace() {
            cut -= 1;
        }
        let mut output = String::new();
        output.push_str(&xml[..cut]);
        output.push(' ');
        output.push_str(name);
        output.push_str("=\"");
        output.push_str(&encoded);
        output.push_str("\"/>");
        return output;
    }
    xml.to_string()
}

fn find_attribute(xml: &str, name: &str) -> Option<(usize, usize, usize)> {
    if name.is_empty() {
        return None;
    }
    let bytes = xml.as_bytes();
    let mut search_from = 0;
    while search_from < xml.len() {
        let offset = xml[search_from..].find(name)?;
        let start = search_from + offset;
        if !attribute_boundary(xml, start) {
            search_from = start + name.len();
            continue;
        }
        let mut cursor = start + name.len();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            search_from = start + name.len();
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || (bytes[cursor] != b'"' && bytes[cursor] != b'\'') {
            search_from = start + name.len();
            continue;
        }
        let quote = bytes[cursor];
        let value_at = cursor + 1;
        let mut value_end = value_at;
        while value_end < bytes.len() && bytes[value_end] != quote {
            value_end += 1;
        }
        if value_end >= bytes.len() {
            return None;
        }
        return Some((start, value_at, value_end));
    }
    None
}

fn attribute_boundary(xml: &str, start: usize) -> bool {
    if start == 0 {
        return false;
    }
    xml[..start]
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_whitespace() || ch == '<')
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn escape_xml(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_reference_rewrites_preserve_text_and_numbering_links() {
        let source = r#"<w:document><w:p><w:pPr><w:spacing w:after="120"/></w:pPr><w:r><w:rPr><w:b/><w:color w:val="123456"/></w:rPr><w:t>Keep &amp; bold</w:t></w:r></w:p></w:document>"#;
        let map = BTreeMap::from([
            ("normal".to_string(), "CorpBody".to_string()),
            ("strong".to_string(), "CorpStrong".to_string()),
        ]);
        let expected = r#"<w:document><w:p><w:pPr><w:pStyle w:val="CorpBody"/></w:pPr><w:r><w:rPr><w:rStyle w:val="CorpStrong"/><w:b/></w:rPr><w:t>Keep &amp; bold</w:t></w:r></w:p></w:document>"#;
        let generated = r#"<w:numbering><w:abstractNum w:abstractNumId="4"></w:abstractNum><w:num w:numId="8"><w:abstractNumId w:val="4"/></w:num></w:numbering>"#;
        let reference = r#"<w:numbering><w:abstractNum w:abstractNumId="1"></w:abstractNum><w:abstractNum w:abstractNumId="2"></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num></w:numbering>"#;
        for _ in 0..2 {
            assert_eq!(apply_style_map_to_document(source, &map), expected);
            let remap = remap_reference_numbering(generated, Some(reference)).unwrap();
            assert_eq!(remap.num_id_map, BTreeMap::from([(1, 9), (2, 10)]));
            assert!(remap
                .xml
                .contains(r#"<w:num w:numId="9"><w:abstractNumId w:val="5"/></w:num>"#));
            assert!(remap
                .xml
                .contains(r#"<w:num w:numId="10"><w:abstractNumId w:val="6"/></w:num>"#));
            assert!(remap
                .xml
                .contains(r#"<w:num w:numId="8"><w:abstractNumId w:val="4"/></w:num>"#));
        }
    }

    fn code_of(error: &MarkdownConversionError) -> Option<&str> {
        error
            .context()
            .and_then(|context| context.get("code"))
            .map(String::as_str)
    }

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, body) in files {
            writer.start_file(*name, options).expect("start");
            writer.write_all(body).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    fn package_contains(bytes: &[u8], token: &[u8]) -> bool {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).expect("entry");
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).expect("read");
            if buf.windows(token.len()).any(|window| window == token) {
                return true;
            }
        }
        false
    }

    fn package_names(bytes: &[u8]) -> Vec<String> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut names = Vec::new();
        for index in 0..archive.len() {
            let file = archive.by_index(index).expect("entry");
            names.push(file.name().expect("name").into_owned());
        }
        names
    }

    fn part_text(bytes: &[u8], name: &str) -> String {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut file = archive.by_name(name).expect(name);
        let mut text = String::new();
        file.read_to_string(&mut text).expect("text");
        text
    }

    fn style_xml(id: &str, name: &str, style_type: &str) -> ReferenceStyle {
        ReferenceStyle {
            id: id.to_string(),
            name: Some(name.to_string()),
            style_type: style_type.to_string(),
        }
    }

    #[test]
    fn rejects_unsafe_entry_path() {
        // tests/reference-docx-styles.test.ts: "rejects unsafe ZIP paths and bounded zip-bomb-style packages"
        let template = zip_with(&[("../secret.xml", b"unsafe")]);
        let error = apply_reference(&template, b"unused").expect_err("unsafe path");
        assert_eq!(code_of(&error), Some("UNSAFE_ENTRY_PATH"));
        assert!(error.message().contains("unsafe ZIP entry path"));
        assert_eq!(
            error
                .context()
                .and_then(|context| context.get("phase"))
                .map(String::as_str),
            Some("reference-docx")
        );

        let encoded = zip_with(&[("%2e%2e/secret.xml", b"unsafe")]);
        let error = apply_reference(&encoded, b"unused").expect_err("encoded traversal");
        assert_eq!(code_of(&error), Some("UNSAFE_ENTRY_PATH"));
    }

    #[test]
    fn rejects_missing_required_part() {
        // tests/reference-docx-styles.test.ts: "defines fallback, throw, duplicate-name, malformed-package, and abort behavior"
        let template = zip_with(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("_rels/.rels", b"<Relationships></Relationships>"),
            ("word/document.xml", b"<w:document></w:document>"),
            (
                "word/_rels/document.xml.rels",
                b"<Relationships></Relationships>",
            ),
        ]);
        let error = apply_reference(&template, b"unused").expect_err("missing styles");
        assert_eq!(code_of(&error), Some("MISSING_PACKAGE_PART"));
        assert_eq!(
            error
                .context()
                .and_then(|context| context.get("entry"))
                .map(String::as_str),
            Some("word/styles.xml")
        );
        assert!(error
            .message()
            .contains("missing required package part word/styles.xml"));
    }

    #[test]
    fn style_resolution_keeps_error_codes() {
        let styles = vec![
            style_xml("CorpH1", "Executive Heading", "paragraph"),
            style_xml("CorpH1Duplicate", "Executive Heading", "paragraph"),
            style_xml("Heading1", "Heading 1", "character"),
        ];
        let duplicate = resolve_role(
            &styles,
            &AutoStyleCandidates {
                role: "heading1",
                ids: &["Heading1"],
                names: &["Heading 1"],
            },
            Some(&("heading1", None, Some("Executive Heading"), false)),
            false,
            true,
        )
        .expect_err("duplicate");
        assert_eq!(code_of(&duplicate), Some("DUPLICATE_STYLE_NAME"));

        let missing = resolve_role(
            &styles,
            &AutoStyleCandidates {
                role: "heading1",
                ids: &["Heading1"],
                names: &["Heading 1"],
            },
            Some(&("heading1", None, Some("Missing Heading"), false)),
            true,
            true,
        )
        .expect_err("missing");
        assert_eq!(code_of(&missing), Some("MISSING_STYLE"));

        let mismatch = resolve_role(
            &styles,
            &AutoStyleCandidates {
                role: "heading1",
                ids: &["Heading1"],
                names: &["Heading 1"],
            },
            Some(&("heading1", Some("Heading1"), None, false)),
            true,
            true,
        )
        .expect_err("type");
        assert_eq!(code_of(&mismatch), Some("STYLE_TYPE_MISMATCH"));

        let aborted = ensure_not_aborted(true).expect_err("aborted");
        assert_eq!(code_of(&aborted), Some("ABORTED"));
    }

    #[test]
    fn rejects_invalid_zip_and_non_word_package() {
        let error = apply_reference(b"not a zip", b"unused").expect_err("zip");
        assert_eq!(code_of(&error), Some("INVALID_ZIP"));

        let template = zip_with(&[
            (
                "[Content_Types].xml",
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                br#"<root/>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#,
            ),
            (
                "word/styles.xml",
                br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:styles>"#,
            ),
        ]);
        let error = apply_reference(&template, b"unused").expect_err("package");
        assert_eq!(code_of(&error), Some("INVALID_PACKAGE"));
    }

    #[test]
    fn copies_reference_styles_without_body_comments_footnotes_macros_or_unused_media() {
        let template = reference_template(false);
        let generated = generated_package();
        let output = apply_reference(&template, &generated).expect("apply");
        let names = package_names(&output);
        let document = part_text(&output, "word/document.xml");
        let styles = part_text(&output, "word/styles.xml");
        let numbering = part_text(&output, "word/numbering.xml");

        assert!(document.contains("GENERATED_BODY_TOKEN"));
        assert!(!package_contains(&output, b"SECRET_BODY_TOKEN"));
        assert!(!package_contains(&output, b"SECRET_COMMENT_TOKEN"));
        assert!(!package_contains(&output, b"SECRET_FOOTNOTE_TOKEN"));
        assert!(!package_contains(&output, b"VBA_MACRO_TOKEN"));
        assert!(!package_contains(&output, b"UNUSED_MEDIA_TOKEN"));
        assert!(package_contains(&output, b"USED_IMAGE_TOKEN"));
        assert!(package_contains(&output, b"Template Header"));
        assert!(package_contains(&output, b"Corporate Theme"));
        assert!(package_contains(&output, b"Aptos"));
        assert!(styles.contains("CorpOnly"));
        assert!(styles.contains("GeneratedOnly"));
        assert!(styles.contains(r#"w:numId w:val="2""#) || styles.contains(r#"w:val="2""#));
        assert!(document.contains(r#"w:top="2000""#));
        assert!(document.contains("w:headerReference"));
        assert!(names
            .iter()
            .any(|name| name.contains("reference-package/word/header1.xml")));
        assert!(names
            .iter()
            .any(|name| name.contains("reference-package/word/theme/theme1.xml")));
        assert!(names
            .iter()
            .any(|name| name.contains("reference-package/word/fontTable.xml")));
        assert!(!names.iter().any(|name| name.ends_with("comments.xml")));
        assert!(!names.iter().any(|name| name.ends_with("footnotes.xml")));
        assert!(!names.iter().any(|name| name.ends_with("vbaProject.bin")));
        assert!(!names.iter().any(|name| name.ends_with("unused.png")));
        assert!(numbering.contains("AAAAAAA1"));
        assert!(numbering.contains("BBBBBBB2"));
        assert!(numbering.contains(r#"w:abstractNumId="1""#));
    }

    #[test]
    fn rejects_unsupported_header_relationship() {
        let template = reference_template(true);
        let error = apply_reference(&template, &generated_package()).expect_err("chart");
        assert_eq!(code_of(&error), Some("UNSUPPORTED_RELATIONSHIP"));
        assert_eq!(
            error
                .context()
                .and_then(|context| context.get("relationshipType"))
                .map(String::as_str),
            Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart")
        );
    }

    fn generated_package() -> Vec<u8> {
        zip_with(&[
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
</Types>"#,
            ),
            (
                "_rels/.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>GENERATED_BODY_TOKEN</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
  </w:body>
</w:document>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#,
            ),
            (
                "word/styles.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style>
  <w:style w:type="paragraph" w:styleId="GeneratedOnly"><w:name w:val="Generated Only"/></w:style>
</w:styles>"#,
            ),
            (
                "word/numbering.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0"><w:nsid w:val="BBBBBBB2"/></w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
</w:numbering>"#,
            ),
        ])
    }

    fn reference_template(chart_in_header: bool) -> Vec<u8> {
        let header_rels: &[u8] = if chart_in_header {
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/used.png"/><Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/></Relationships>"#
        } else {
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/used.png"/></Relationships>"#
        };
        zip_with(&[
            (
                "[Content_Types].xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
  <Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
  <Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
</Types>"#,
            ),
            (
                "_rels/.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <w:body>
    <w:p><w:r><w:t>SECRET_BODY_TOKEN</w:t></w:r></w:p>
    <w:sectPr>
      <w:headerReference w:type="default" r:id="rId2"/>
      <w:pgSz w:w="12240" w:h="15840"/>
      <w:pgMar w:top="2000" w:right="1200" w:bottom="1100" w:left="1400"/>
    </w:sectPr>
  </w:body>
</w:document>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/>
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>
  <Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/unused.png"/>
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>
  <Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
</Relationships>"#,
            ),
            (
                "word/styles.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading 1"/></w:style>
  <w:style w:type="paragraph" w:styleId="CorpOnly"><w:name w:val="Corporate Only"/></w:style>
  <w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>
</w:styles>"#,
            ),
            (
                "word/header1.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Template Header</w:t></w:r></w:p></w:hdr>"#,
            ),
            ("word/_rels/header1.xml.rels", header_rels),
            ("word/media/used.png", b"USED_IMAGE_TOKEN"),
            ("word/media/unused.png", b"UNUSED_MEDIA_TOKEN"),
            ("word/comments.xml", b"<w:comments>SECRET_COMMENT_TOKEN</w:comments>"),
            ("word/footnotes.xml", b"<w:footnotes>SECRET_FOOTNOTE_TOKEN</w:footnotes>"),
            ("word/vbaProject.bin", b"VBA_MACRO_TOKEN"),
            (
                "word/theme/theme1.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Corporate Theme"></a:theme>"#,
            ),
            (
                "word/fontTable.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:font w:name="Aptos"><w:family w:val="swiss"/></w:font></w:fonts>"#,
            ),
            (
                "word/numbering.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0"><w:nsid w:val="AAAAAAA1"/></w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
</w:numbering>"#,
            ),
        ])
    }
}
