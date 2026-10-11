use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use regex::Regex;
use zip::ZipArchive;

use crate::error::MarkdownConversionError;
use crate::ooxml::{self, PackageMetadata};
use crate::options::{
    normalize_document_metadata, ConvertOptions, Direction, DocumentMetadata,
    DuplicateStyleNameBehavior, MarginInput, MissingStyleBehavior, NormalizedMetadata, Orientation,
    PageInput, PatchMarkdownOptions, ReferenceDocxGenerationOptions, ReferenceDocxModeOptions,
    ReferenceStyleSelector, SectionConfig, SizeInput, UpdateFieldsOnOpen,
};
use crate::package;
use crate::parse;

/// Convert `markdown` into a `.docx` package.
///
/// This is [`convert_with_options`] with [`ConvertOptions::default`].
pub fn convert_markdown(markdown: &str) -> Result<Vec<u8>, MarkdownConversionError> {
    convert_with_options(markdown, &ConvertOptions::default())
}

/// Replace `{{name}}` placeholders in `template` with Markdown bytes.
///
/// Delimiters are `{{` and `}}`. A Word comment in a patch fails with
/// `Patch markdown does not support Word comments yet`.
pub fn patch_placeholders(
    template: &[u8],
    patches: &[(&str, &[u8])],
) -> Result<Vec<u8>, MarkdownConversionError> {
    crate::patch::patch_placeholders(template, patches)
}

/// Generate `markdown`, then copy styles and layout from `template`.
///
/// A missing `word/styles.xml` returns `MISSING_PACKAGE_PART`. An unsafe ZIP
/// path returns `UNSAFE_ENTRY_PATH`.
pub fn convert_with_reference(
    markdown: &str,
    template: &[u8],
    options: &ReferenceDocxGenerationOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    if options.aborted {
        return Err(reference_failure(
            "Reference DOCX generation was aborted",
            "ABORTED",
            &[],
        ));
    }
    crate::options::validate_reference_options(options.reference.as_ref())?;
    enforce_reference_limits(template, options.reference.as_ref())?;
    let mut convert_options = options.convert.clone();
    if options
        .reference
        .as_ref()
        .is_none_or(|mode| mode.preserve_page_layout())
    {
        if let Some(page) = reference_page(template) {
            let template = convert_options
                .template
                .get_or_insert_with(SectionConfig::default);
            if template.page.is_none() {
                template.page = Some(page);
            }
        }
    }
    let generated = convert_with_options(markdown, &convert_options)?;
    let generated = adopt_explicit_styles(template, &generated, options.reference.as_ref())?;
    let generated = ensure_footnote_text_style(&generated)?;
    if options.reference.is_none() {
        crate::reference::apply_reference(template, &generated)
    } else {
        crate::reference::apply_reference_with_options(
            template,
            &generated,
            options.reference.as_ref(),
        )
    }
}

/// Convert each patch as Markdown, then replace the first `{{name}}` hit.
pub fn patch_markdown(
    template: &[u8],
    patches: &BTreeMap<String, String>,
    options: &PatchMarkdownOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    options.validate()?;
    let mut convert_options = options.convert.clone();
    let stricter_limit = |outer: Option<u32>, inner: Option<u32>| match (outer, inner) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    convert_options.max_input_length =
        stricter_limit(options.max_input_length, convert_options.max_input_length);
    convert_options.max_elements =
        stricter_limit(options.max_elements, convert_options.max_elements);
    for markdown in patches.values() {
        enforce_input_length(markdown, &convert_options)?;
    }
    let source = template.to_vec();
    let (template, patches) = expand_repeated_placeholders(template, patches, options.recursive)?;
    let owned: Vec<(&str, &[u8])> = patches
        .iter()
        .map(|(name, markdown)| (name.as_str(), markdown.as_bytes()))
        .collect();
    let mut patched =
        crate::patch::patch_placeholders_with_options(&template, &owned, &convert_options)?;
    patched = finish_patch_document(&source, &patched)?;
    if let Some(width) = options.table_width_twips {
        patched = rewrite_table_widths(&patched, width)?;
    }
    if let Some(metadata) = &options.metadata {
        patched = merge_patch_metadata(&source, &patched, metadata)?;
    }
    Ok(patched)
}

/// Rewrite document properties on an existing package.
///
/// `None` is not representable here: a caller that wants the legacy package
/// leaves it untouched. An empty metadata value omits the legacy defaults.
pub fn apply_document_metadata(
    bytes: &[u8],
    metadata: &DocumentMetadata,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let normalized = normalize_document_metadata(metadata)?;
    let mut parts = package::unpack(bytes)?;
    parts.metadata = Some(package_metadata(&normalized));
    package::pack(&parts)
}

/// Convert `markdown` with section, header, footer, style, and TOC options.
///
/// Plain options take the same path as before: parse, plant footnote images,
/// render, pack. Other options are resolved first. The current renderer still
/// receives one document, with a `[TOC]` placeholder projected back to a
/// paragraph until it writes the TOC field itself.
pub fn convert_with_options(
    markdown: &str,
    options: &ConvertOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    options.validate()?;
    enforce_input_length(markdown, options)?;
    if options.is_plain() {
        let mut document = parse::parse_markdown(markdown)?;
        ooxml::plant_footnote_images(&mut document, markdown);
        let parts = ooxml::render(&document)?;
        return pack_with_metadata(parts, options);
    }
    let prepared = parse::prepare(markdown, options)?;
    let mut parts = ooxml::render_prepared(&prepared, markdown, options)?;
    if suppress_caption_field_update(options) && !prepared.update_fields {
        if let Some(settings) = parts.settings.as_mut() {
            strip_update_fields(settings);
        }
    }
    pack_with_metadata(parts, options)
}

/// `None` metadata does not touch the package. A set value, including an
/// empty object, is written by the package installer.
fn pack_with_metadata(
    mut parts: ooxml::Parts,
    options: &ConvertOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    if let Some(metadata) = &options.metadata {
        let normalized = normalize_document_metadata(metadata)?;
        parts.metadata = Some(package_metadata(&normalized));
    }
    paint_accessibility(&mut parts, options)?;
    package::pack(&parts)
}

fn package_metadata(metadata: &NormalizedMetadata) -> PackageMetadata {
    PackageMetadata {
        title: metadata.title.clone(),
        subject: metadata.subject.clone(),
        description: metadata.description.clone(),
        creator: metadata.creator.clone(),
        keywords: metadata.keywords.clone(),
        category: metadata.category.clone(),
        company: metadata.company.clone(),
        language: metadata.language.clone(),
        created: metadata.created.clone(),
        modified: metadata.modified.clone(),
        custom: metadata.custom.as_ref().map(|properties| {
            properties
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        }),
    }
}

/// `captions.updateFieldsOnOpen: false` omits `w:updateFields` when no TOC
/// field still needs it. The writer turns the flag on for every caption.
/// `maxInputLength` is a count of UTF-16 code units.
///
/// When `sections` is set, even as an empty list, the sum is the section
/// Markdown only. Header and footer Markdown is checked later, with a
/// different message, once those slots are resolved.
fn enforce_input_length(
    markdown: &str,
    options: &ConvertOptions,
) -> Result<(), MarkdownConversionError> {
    let Some(max) = options.max_input_length else {
        return Ok(());
    };
    let length = if let Some(sections) = &options.sections {
        sections
            .iter()
            .map(|section| utf16_len(&section.markdown))
            .sum::<usize>()
    } else {
        utf16_len(markdown)
    };
    if length > usize::try_from(max).unwrap_or(usize::MAX) {
        return Err(MarkdownConversionError::new(
            "Markdown input length exceeds maxInputLength",
        ));
    }
    Ok(())
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn suppress_caption_field_update(options: &ConvertOptions) -> bool {
    matches!(
        options
            .captions
            .as_ref()
            .and_then(|captions| captions.update_fields_on_open),
        Some(UpdateFieldsOnOpen::Bool(false))
    )
}

fn strip_update_fields(xml: &mut Vec<u8>) {
    let Ok(text) = std::str::from_utf8(xml) else {
        return;
    };
    let mut cleaned = text.to_string();
    while let Some(start) = cleaned.find("<w:updateFields") {
        let Some(relative_end) = cleaned[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        cleaned.replace_range(start..end, "");
        let close = "</w:updateFields>";
        if cleaned[start..].starts_with(close) {
            cleaned.replace_range(start..start + close.len(), "");
        }
    }
    *xml = cleaned.into_bytes();
}

fn reference_failure(
    message: impl Into<String>,
    code: &str,
    details: &[(&str, String)],
) -> MarkdownConversionError {
    let mut context = BTreeMap::new();
    context.insert("phase".to_string(), "reference-docx".to_string());
    context.insert("code".to_string(), code.to_string());
    for (key, value) in details {
        context.insert((*key).to_string(), value.clone());
    }
    MarkdownConversionError::with_context(message, context)
}

fn enforce_reference_limits(
    template: &[u8],
    mode: Option<&ReferenceDocxModeOptions>,
) -> Result<(), MarkdownConversionError> {
    let Some(mode) = mode else {
        return Ok(());
    };
    let Some(explicit) = &mode.limits else {
        return Ok(());
    };
    let _ = explicit;
    let limits = mode.resolved_limits()?;
    let mut archive = match ZipArchive::new(Cursor::new(template)) {
        Ok(archive) => archive,
        Err(_) => return Ok(()),
    };
    if template.len() as u64 > limits.max_compressed_bytes {
        return Err(reference_failure(
            "Reference DOCX exceeds the compressed package limit",
            "PACKAGE_LIMIT_EXCEEDED",
            &[
                ("limit", limits.max_compressed_bytes.to_string()),
                ("actual", template.len().to_string()),
            ],
        ));
    }
    if archive.len() as u64 > limits.max_entries {
        return Err(reference_failure(
            format!(
                "Reference DOCX contains {} ZIP entries; limit is {}",
                archive.len(),
                limits.max_entries
            ),
            "PACKAGE_LIMIT_EXCEEDED",
            &[
                ("limit", limits.max_entries.to_string()),
                ("actual", archive.len().to_string()),
            ],
        ));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
        let size = entry.size();
        let name = entry
            .name()
            .map(|name| name.into_owned())
            .unwrap_or_default();
        if size > limits.max_entry_uncompressed_bytes {
            return Err(reference_failure(
                format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
                "PACKAGE_LIMIT_EXCEEDED",
                &[
                    ("limit", limits.max_entry_uncompressed_bytes.to_string()),
                    ("actual", size.to_string()),
                    ("entry", name),
                ],
            ));
        }
        total = total.saturating_add(size);
        if total > limits.max_uncompressed_bytes {
            return Err(reference_failure(
                "Reference DOCX exceeds the total uncompressed package limit",
                "PACKAGE_LIMIT_EXCEEDED",
                &[
                    ("limit", limits.max_uncompressed_bytes.to_string()),
                    ("actual", total.to_string()),
                ],
            ));
        }
    }
    Ok(())
}

fn reference_page(template: &[u8]) -> Option<PageInput> {
    let xml = zip_text(template, "word/document.xml")?;
    let sect = last_match(&xml, r"(?s)<w:sectPr\b.*?</w:sectPr>")?;
    let size_tag = first_match(&sect, r"<w:pgSz\b[^>]*/>");
    let margin_tag = first_match(&sect, r"<w:pgMar\b[^>]*/>");
    if size_tag.is_none() && margin_tag.is_none() {
        return None;
    }
    let size = size_tag.map(|tag| SizeInput {
        width: xml_u32(&tag, "w:w"),
        height: xml_u32(&tag, "w:h"),
        orientation: xml_attr(&tag, "w:orient").map(|value| {
            if value.eq_ignore_ascii_case("landscape") {
                Orientation::Landscape
            } else {
                Orientation::Portrait
            }
        }),
    });
    let margin = margin_tag.map(|tag| MarginInput {
        top: xml_u32(&tag, "w:top"),
        right: xml_u32(&tag, "w:right"),
        bottom: xml_u32(&tag, "w:bottom"),
        left: xml_u32(&tag, "w:left"),
        header: xml_u32(&tag, "w:header"),
        footer: xml_u32(&tag, "w:footer"),
        gutter: xml_u32(&tag, "w:gutter"),
    });
    Some(PageInput { size, margin })
}

struct ParsedStyle {
    id: String,
    style_type: String,
    name: Option<String>,
}

fn adopt_explicit_styles(
    template: &[u8],
    generated: &[u8],
    mode: Option<&ReferenceDocxModeOptions>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let Some(mode) = mode else {
        return Ok(generated.to_vec());
    };
    let Some(selectors) = &mode.styles else {
        return Ok(generated.to_vec());
    };
    if selectors.is_empty() {
        return Ok(generated.to_vec());
    }
    let Some(styles_xml) = zip_text(template, "word/styles.xml") else {
        return Ok(generated.to_vec());
    };
    let styles = parse_reference_styles(&styles_xml);
    if styles.is_empty() {
        return Ok(generated.to_vec());
    }
    let missing_throw = mode.missing_style_behavior() == MissingStyleBehavior::Throw;
    let duplicate_throw = mode.duplicate_style_name_behavior() == DuplicateStyleNameBehavior::Throw;
    let mut replacements = Vec::new();
    for (role, selector) in selectors {
        let style_id =
            resolve_explicit_style(&styles, role, selector, missing_throw, duplicate_throw)?;
        let Some(style_id) = style_id else {
            continue;
        };
        if let Some(generated_id) = generated_style_id(role) {
            replacements.push((generated_id.to_string(), style_id));
        }
    }
    if replacements.is_empty() {
        return Ok(generated.to_vec());
    }
    let mut parts = package::unpack(generated)?;
    let mut document = String::from_utf8(parts.document.clone())
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    for (from, to) in replacements {
        document = document.replace(
            &format!(r#"<w:pStyle w:val="{from}"/>"#),
            &format!(r#"<w:pStyle w:val="{to}"/>"#),
        );
        if let Some(level) = from.strip_prefix("Heading") {
            document = document.replace(
                &format!(r#"<w:pStyle w:val="{level}"/>"#),
                &format!(r#"<w:pStyle w:val="{to}"/>"#),
            );
        }
    }
    parts.document = document.into_bytes();
    package::pack(&parts)
}

fn parse_reference_styles(styles_xml: &str) -> Vec<ParsedStyle> {
    let Ok(pattern) = Regex::new(r"(?s)<w:style\b.*?</w:style>") else {
        return Vec::new();
    };
    let mut styles = Vec::new();
    for found in pattern.find_iter(styles_xml) {
        let xml = found.as_str();
        let open = xml.split('>').next().unwrap_or(xml);
        let id = xml_attr(open, "w:styleId").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let name = first_match(xml, r"<w:name\b[^>]*/>").and_then(|tag| xml_attr(&tag, "w:val"));
        styles.push(ParsedStyle {
            id,
            style_type: xml_attr(open, "w:type").unwrap_or_else(|| "paragraph".to_string()),
            name,
        });
    }
    styles
}

fn resolve_explicit_style(
    styles: &[ParsedStyle],
    role: &str,
    selector: &ReferenceStyleSelector,
    missing_throw: bool,
    duplicate_throw: bool,
) -> Result<Option<String>, MarkdownConversionError> {
    let style = match selector {
        ReferenceStyleSelector::Id(id) => styles.iter().find(|style| style.id == *id),
        ReferenceStyleSelector::Name(name) => {
            let matches = styles
                .iter()
                .filter(|style| style.name.as_deref() == Some(name.as_str()))
                .collect::<Vec<_>>();
            if matches.len() > 1 && duplicate_throw {
                return Err(reference_failure(
                    format!(
                        "Reference DOCX style name {name} is duplicated; select by style ID or set duplicateStyleNameBehavior to \"first\""
                    ),
                    "DUPLICATE_STYLE_NAME",
                    &[("role", role.to_string())],
                ));
            }
            matches.first().copied()
        }
    };
    let Some(style) = style else {
        if missing_throw {
            return Err(reference_failure(
                format!("Reference DOCX style for {role} could not be found"),
                "MISSING_STYLE",
                &[("role", role.to_string())],
            ));
        }
        return Ok(None);
    };
    let expected = expected_style_type(role);
    if style.style_type != expected {
        if missing_throw {
            return Err(reference_failure(
                format!(
                    "Reference style {} has type {}; {role} requires {expected}",
                    style.id, style.style_type
                ),
                "STYLE_TYPE_MISMATCH",
                &[("role", role.to_string())],
            ));
        }
        return Ok(None);
    }
    Ok(Some(style.id.clone()))
}

fn expected_style_type(role: &str) -> &'static str {
    match role {
        "strong" | "emphasis" | "inlineCode" | "hyperlink" => "character",
        "table" => "table",
        _ => "paragraph",
    }
}

fn generated_style_id(role: &str) -> Option<&'static str> {
    match role {
        "normal" => Some("Normal"),
        "title" => Some("Title"),
        "heading1" => Some("Heading1"),
        "heading2" => Some("Heading2"),
        "heading3" => Some("Heading3"),
        "heading4" => Some("Heading4"),
        "heading5" => Some("Heading5"),
        "heading6" => Some("Heading6"),
        "caption" => Some("Caption"),
        "listParagraph" => Some("ListParagraph"),
        "blockquote" => None,
        "codeBlock" => None,
        "table" => None,
        "strong" => Some("Strong"),
        "emphasis" => Some("Emphasis"),
        "inlineCode" => Some("InlineCode"),
        "hyperlink" => Some("Hyperlink"),
        _ => None,
    }
}

fn expand_repeated_placeholders(
    template: &[u8],
    patches: &BTreeMap<String, String>,
    recursive: Option<bool>,
) -> Result<(Vec<u8>, BTreeMap<String, String>), MarkdownConversionError> {
    if recursive == Some(false) {
        return Ok((template.to_vec(), patches.clone()));
    }
    let Ok(mut parts) = package::unpack(template) else {
        return Ok((template.to_vec(), patches.clone()));
    };
    let Ok(mut xml) = String::from_utf8(parts.document.clone()) else {
        return Ok((template.to_vec(), patches.clone()));
    };
    let mut expanded = patches.clone();
    let mut changed = false;
    for (name, markdown) in patches {
        let token = format!("{{{{{name}}}}}");
        let indexes = xml
            .match_indices(&token)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if indexes.len() <= 1 {
            continue;
        }
        for (ordinal, index) in indexes.iter().enumerate().skip(1).rev() {
            let alias = format!("{name}__{ordinal}");
            let replacement = format!("{{{{{alias}}}}}");
            xml.replace_range(*index..*index + token.len(), &replacement);
            expanded.insert(alias, markdown.clone());
            changed = true;
        }
    }
    if !changed {
        return Ok((template.to_vec(), patches.clone()));
    }
    parts.document = xml.into_bytes();
    Ok((package::pack(&parts)?, expanded))
}

/// The docx library's default styles include `FootnoteText`. Reference merge
/// keeps generated style ids that the template does not already define.
fn ensure_footnote_text_style(generated: &[u8]) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut parts = match package::unpack(generated) {
        Ok(parts) => parts,
        Err(_) => return Ok(generated.to_vec()),
    };
    let Ok(styles) = String::from_utf8(parts.styles.clone()) else {
        return Ok(generated.to_vec());
    };
    if styles.contains("w:styleId=\"FootnoteText\"") || !styles.contains("</w:styles>") {
        return Ok(generated.to_vec());
    }
    let style = r#"<w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/><w:basedOn w:val="Normal"/><w:link w:val="FootnoteTextChar"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style>"#;
    parts.styles = styles
        .replace("</w:styles>", &format!("{style}</w:styles>"))
        .into_bytes();
    package::pack(&parts)
}

fn rewrite_table_widths(bytes: &[u8], width: u32) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut parts = package::unpack(bytes)?;
    let xml = String::from_utf8(parts.document.clone())
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    let pattern = Regex::new(r"<w:tblW\b[^>]*/>").expect("tblW");
    let rewritten = pattern
        .replace_all(&xml, format!(r#"<w:tblW w:type="dxa" w:w="{width}"/>"#))
        .into_owned();
    parts.document = rewritten.into_bytes();
    package::pack(&parts)
}

/// Share heading counters across separately converted placeholders, and give
/// repeated caption SEQ fields document order (`Table 1`, then `Table 2`).
fn finish_patch_document(
    template: &[u8],
    bytes: &[u8],
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut parts = package::unpack(bytes)?;
    let xml = String::from_utf8(parts.document.clone())
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    let rewritten =
        renumber_caption_fields(&uniquify_bookmark_names(&share_heading_bookmarks(&xml)));
    let template_styles = zip_bytes(template, "word/styles.xml");
    let styles_changed = template_styles
        .as_ref()
        .is_some_and(|styles| styles != &parts.styles);
    if let Some(styles) = template_styles {
        parts.styles = styles;
    }
    if rewritten == xml && !styles_changed {
        return Ok(bytes.to_vec());
    }
    parts.document = rewritten.into_bytes();
    package::pack(&parts)
}

/// `prefix_bookmarks` keeps each donor's own `_Toc_{slug}_{n}`. Number the
/// slug again in document order so the second `# Repeat` is `_Toc_Repeat_2`.
fn share_heading_bookmarks(xml: &str) -> String {
    let pattern = Regex::new(r#"w:name="([^"]+)""#).expect("bookmark name");
    let mut next = BTreeMap::<String, u32>::new();
    let mut output = String::new();
    let mut last = 0;
    for found in pattern.captures_iter(xml) {
        let Some(whole) = found.get(0) else {
            continue;
        };
        output.push_str(&xml[last..whole.start()]);
        let name = found.get(1).map(|value| value.as_str()).unwrap_or("");
        let renamed = match split_heading_bookmark(name) {
            Some((prefix, slug, suffix)) => {
                let assigned = next.entry(slug.clone()).or_insert(0);
                *assigned += 1;
                format!(r#"w:name="{prefix}_Toc_{slug}_{assigned}{suffix}""#)
            }
            None => whole.as_str().to_string(),
        };
        output.push_str(&renamed);
        last = whole.end();
    }
    output.push_str(&xml[last..]);
    output
}

/// The patch engine can stamp the same donor prefix on every copy. Later
/// copies need a distinct `w:name`.
fn uniquify_bookmark_names(xml: &str) -> String {
    let pattern = Regex::new(r#"w:name="([^"]+)""#).expect("bookmark name");
    let mut seen = BTreeMap::<String, u32>::new();
    let mut output = String::new();
    let mut last = 0;
    for found in pattern.captures_iter(xml) {
        let Some(whole) = found.get(0) else {
            continue;
        };
        output.push_str(&xml[last..whole.start()]);
        let name = found.get(1).map(|value| value.as_str()).unwrap_or("");
        let count = seen.entry(name.to_string()).or_insert(0);
        *count += 1;
        if *count == 1 {
            output.push_str(whole.as_str());
        } else {
            output.push_str(&format!(r#"w:name="{name}_{count}""#));
        }
        last = whole.end();
    }
    output.push_str(&xml[last..]);
    output
}

/// `(prefix before _Toc_, slug, trailing donor index such as "_0")`.
fn split_heading_bookmark(name: &str) -> Option<(String, String, String)> {
    let toc = name.find("_Toc_")?;
    let prefix = name[..toc].to_string();
    let body = name[toc + "_Toc_".len()..].to_string();
    let (core, suffix) = if prefix.is_empty() {
        (body.as_str(), String::new())
    } else {
        match body.rfind('_') {
            Some(pos)
                if !body[pos + 1..].is_empty()
                    && body[pos + 1..].bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                (&body[..pos], body[pos..].to_string())
            }
            _ => (body.as_str(), String::new()),
        }
    };
    let (slug, counter) = core.rsplit_once('_')?;
    if slug.is_empty() || counter.is_empty() || !counter.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((prefix, slug.to_string(), suffix))
}

fn renumber_caption_fields(xml: &str) -> String {
    let fields = Regex::new(r"(?s)<w:fldSimple\b[^>]*>.*?</w:fldSimple>").expect("field");
    let text = Regex::new(r"(<w:t\b[^>]*>)\d+(</w:t>)").expect("cached number");
    let mut tables = 0_u32;
    let mut figures = 0_u32;
    let numbered = fields
        .replace_all(xml, |caps: &regex::Captures| {
            let block = &caps[0];
            let number = if block.contains("MdToDocxTable") {
                tables += 1;
                Some(tables)
            } else if block.contains("MdToDocxFigure") {
                figures += 1;
                Some(figures)
            } else {
                None
            };
            let Some(number) = number else {
                return block.to_string();
            };
            text.replace(block, format!("${{1}}{number}${{2}}"))
                .into_owned()
        })
        .into_owned();
    let numbered = renumber_cached_labels(&numbered, "Table", tables);
    renumber_cached_labels(&numbered, "Figure", figures)
}

/// Cross-references cache `Table 1` in one `w:t`. Number those caches in
/// document order so the second copy reads `Table 2`.
fn renumber_cached_labels(xml: &str, label: &str, count: u32) -> String {
    if count == 0 {
        return xml.to_string();
    }
    let pattern = Regex::new(&format!(r"(>{label} )\d+(</w:t>)")).expect("caption cache");
    let mut number = 0_u32;
    pattern
        .replace_all(xml, |caps: &regex::Captures| {
            number += 1;
            let shown = number.min(count);
            format!("{}{shown}{}", &caps[1], &caps[2])
        })
        .into_owned()
}

fn merge_patch_metadata(
    template: &[u8],
    bytes: &[u8],
    metadata: &DocumentMetadata,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let normalized = normalize_document_metadata(metadata)?;
    let mut parts = package::unpack(bytes)?;
    let core = extra_text(&parts, "docProps/core.xml").unwrap_or_default();
    let template_core = zip_text(template, "docProps/core.xml").unwrap_or_default();
    let mut merged = read_core_metadata(&core);
    fill_absent_metadata(&mut merged, &read_core_metadata(&template_core));
    overlay_metadata(&mut merged, &normalized);
    if normalized.custom.is_some() {
        let patched_custom = extra_text(&parts, "docProps/custom.xml").unwrap_or_default();
        let template_custom = zip_text(template, "docProps/custom.xml").unwrap_or_default();
        let existing = if parse_custom_properties(&patched_custom).is_empty() {
            template_custom
        } else {
            patched_custom
        };
        merged.custom = Some(merge_custom_properties(
            &existing,
            normalized.custom.as_ref(),
        ));
    }
    if let Some(language) = merged.language.clone() {
        let document = String::from_utf8(parts.document.clone())
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
        parts.document = insert_once(
            &document,
            "<w:rPr>",
            &format!(r#"<w:lang w:val="{language}"/>"#),
        )
        .into_bytes();
    }
    parts.metadata = Some(merged);
    package::pack(&parts)
}

fn read_core_metadata(core: &str) -> PackageMetadata {
    PackageMetadata {
        title: element_text(core, "dc:title"),
        subject: element_text(core, "dc:subject"),
        description: element_text(core, "dc:description"),
        creator: element_text(core, "dc:creator"),
        keywords: element_text(core, "cp:keywords"),
        category: element_text(core, "cp:category"),
        company: None,
        language: element_text(core, "dc:language"),
        created: element_text(core, "dcterms:created"),
        modified: element_text(core, "dcterms:modified"),
        custom: None,
    }
}

fn fill_absent_metadata(base: &mut PackageMetadata, fallback: &PackageMetadata) {
    if base.title.is_none() {
        base.title = fallback.title.clone();
    }
    if base.subject.is_none() {
        base.subject = fallback.subject.clone();
    }
    if base.description.is_none() {
        base.description = fallback.description.clone();
    }
    if base.creator.is_none() {
        base.creator = fallback.creator.clone();
    }
    if base.keywords.is_none() {
        base.keywords = fallback.keywords.clone();
    }
    if base.category.is_none() {
        base.category = fallback.category.clone();
    }
    if base.language.is_none() {
        base.language = fallback.language.clone();
    }
    if base.created.is_none() {
        base.created = fallback.created.clone();
    }
    if base.modified.is_none() {
        base.modified = fallback.modified.clone();
    }
}

fn overlay_metadata(base: &mut PackageMetadata, extra: &NormalizedMetadata) {
    if extra.title.is_some() {
        base.title = extra.title.clone();
    }
    if extra.subject.is_some() {
        base.subject = extra.subject.clone();
    }
    if extra.description.is_some() {
        base.description = extra.description.clone();
    }
    if extra.creator.is_some() {
        base.creator = extra.creator.clone();
    }
    if extra.keywords.is_some() {
        base.keywords = extra.keywords.clone();
    }
    if extra.category.is_some() {
        base.category = extra.category.clone();
    }
    if extra.company.is_some() {
        base.company = extra.company.clone();
    }
    if extra.language.is_some() {
        base.language = extra.language.clone();
    }
    if extra.created.is_some() {
        base.created = extra.created.clone();
    }
    if extra.modified.is_some() {
        base.modified = extra.modified.clone();
    }
}

fn merge_custom_properties(
    existing_xml: &str,
    supplied: Option<&BTreeMap<String, String>>,
) -> Vec<(String, String)> {
    let mut ordered = parse_custom_properties(existing_xml);
    let Some(supplied) = supplied else {
        return ordered;
    };
    for (name, value) in supplied {
        if let Some(slot) = ordered
            .iter_mut()
            .find(|(current, _)| current.eq_ignore_ascii_case(name))
        {
            slot.1 = value.clone();
        } else {
            ordered.push((name.clone(), value.clone()));
        }
    }
    ordered
}

fn parse_custom_properties(xml: &str) -> Vec<(String, String)> {
    let Ok(pattern) = Regex::new(r"(?s)<property\b([^>]*)>(.*?)</property>") else {
        return Vec::new();
    };
    let Ok(value_pattern) = Regex::new(r"(?s)<vt:lpwstr\b[^>]*>(.*?)</vt:lpwstr>") else {
        return Vec::new();
    };
    pattern
        .captures_iter(xml)
        .filter_map(|caps| {
            let open = caps.get(1).map(|value| value.as_str()).unwrap_or("");
            let name = xml_attr(&format!("<property{open}>"), "name")?;
            let body = caps.get(2).map(|value| value.as_str()).unwrap_or("");
            let value = value_pattern
                .captures(body)
                .map(|found| decode_xml(&found[1]))
                .unwrap_or_default();
            Some((name, value))
        })
        .collect()
}

struct SectionPaint {
    language: Option<String>,
    rtl: bool,
}

fn paint_accessibility(
    parts: &mut ooxml::Parts,
    options: &ConvertOptions,
) -> Result<(), MarkdownConversionError> {
    let document_language = options
        .style
        .as_ref()
        .and_then(|style| style.language.clone())
        .or_else(|| {
            options
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.language.clone())
        });
    let document_rtl = matches!(
        options.style.as_ref().and_then(|style| style.direction),
        Some(Direction::Rtl)
    );
    let section_paints = section_paints(options, document_language.clone(), document_rtl);
    let needs_paint = document_language.is_some()
        || document_rtl
        || section_paints
            .iter()
            .any(|section| section.language.is_some() || section.rtl);
    if !needs_paint {
        return Ok(());
    }
    let styles = String::from_utf8(parts.styles.clone())
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    parts.styles =
        paint_style_language(&styles, document_language.as_deref(), document_rtl).into_bytes();
    let document = String::from_utf8(parts.document.clone())
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    parts.document = paint_document_language(&document, &section_paints).into_bytes();
    Ok(())
}

fn section_paints(
    options: &ConvertOptions,
    document_language: Option<String>,
    document_rtl: bool,
) -> Vec<SectionPaint> {
    let Some(sections) = &options.sections else {
        return vec![SectionPaint {
            language: document_language,
            rtl: document_rtl,
        }];
    };
    if sections.is_empty() {
        return vec![SectionPaint {
            language: document_language,
            rtl: document_rtl,
        }];
    }
    sections
        .iter()
        .map(|section| {
            let style = section.config.style.as_ref();
            SectionPaint {
                language: style
                    .and_then(|style| style.language.clone())
                    .or_else(|| document_language.clone()),
                rtl: match style.and_then(|style| style.direction) {
                    Some(Direction::Rtl) => true,
                    Some(Direction::Ltr) => false,
                    None => document_rtl,
                },
            }
        })
        .collect()
}

fn paint_style_language(styles: &str, language: Option<&str>, rtl: bool) -> String {
    let mut styles = styles.to_string();
    if let Some(language) = language {
        let element = lang_element(language, rtl);
        styles = insert_once(&styles, "<w:rPr>", &element);
        if !styles.contains("<w:outlineLvl w:val=\"1\"/>") {
            styles = insert_once(&styles, "<w:pPr>", "<w:outlineLvl w:val=\"1\"/>");
        }
    }
    styles
}

fn paint_document_language(document: &str, sections: &[SectionPaint]) -> String {
    let mut chunks = Vec::new();
    let mut rest = document;
    while let Some(index) = rest.find("</w:sectPr>") {
        let end = index + "</w:sectPr>".len();
        chunks.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        chunks.push(rest.to_string());
    }
    let section_at = chunks
        .iter()
        .enumerate()
        .filter(|(_, chunk)| chunk.contains("</w:sectPr>"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if section_at.len() != sections.len() {
        return paint_chunk(document, sections.first());
    }
    for (ordinal, chunk_index) in section_at.iter().enumerate() {
        chunks[*chunk_index] = paint_chunk(&chunks[*chunk_index], sections.get(ordinal));
    }
    chunks.concat()
}

fn paint_chunk(chunk: &str, section: Option<&SectionPaint>) -> String {
    let Some(section) = section else {
        return chunk.to_string();
    };
    let mut chunk = chunk.to_string();
    if let Some(language) = &section.language {
        chunk = insert_once(&chunk, "<w:rPr>", &lang_element(language, section.rtl));
    }
    if section.rtl {
        chunk = insert_once(&chunk, "<w:pPr>", "<w:bidi/><w:rtl/>");
    }
    chunk
}

fn lang_element(language: &str, rtl: bool) -> String {
    if rtl {
        format!(r#"<w:lang w:val="{language}" w:bidi="{language}"/>"#)
    } else {
        format!(r#"<w:lang w:val="{language}"/>"#)
    }
}

fn insert_once(xml: &str, needle: &str, insertion: &str) -> String {
    if xml.contains(insertion) {
        return xml.to_string();
    }
    match xml.find(needle) {
        Some(index) => {
            let at = index + needle.len();
            let mut rewritten = String::with_capacity(xml.len() + insertion.len());
            rewritten.push_str(&xml[..at]);
            rewritten.push_str(insertion);
            rewritten.push_str(&xml[at..]);
            rewritten
        }
        None => xml.to_string(),
    }
}

fn extra_text(parts: &ooxml::Parts, name: &str) -> Option<String> {
    parts
        .extras
        .iter()
        .find(|(part, _)| part == name)
        .and_then(|(_, bytes)| String::from_utf8(bytes.clone()).ok())
}

fn element_text(xml: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}");
    let at = xml.find(&start)?;
    let relative = xml[at..].find('>')?;
    if xml.as_bytes().get(at + relative - 1) == Some(&b'/') {
        return None;
    }
    let content_at = at + relative + 1;
    let close = format!("</{tag}>");
    let end = xml[content_at..].find(&close)?;
    Some(decode_xml(&xml[content_at..content_at + end]))
}

fn decode_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn zip_bytes(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name(name).ok()?;
    let mut body = Vec::new();
    file.read_to_end(&mut body).ok()?;
    Some(body)
}

fn zip_text(bytes: &[u8], name: &str) -> Option<String> {
    zip_bytes(bytes, name).and_then(|body| String::from_utf8(body).ok())
}

fn first_match(xml: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .find(xml)
        .map(|found| found.as_str().to_string())
}

fn last_match(xml: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .find_iter(xml)
        .last()
        .map(|found| found.as_str().to_string())
}

fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"{name}="([^"]*)""#);
    Regex::new(&pattern)
        .ok()?
        .captures(tag)
        .and_then(|caps| caps.get(1))
        .map(|value| decode_xml(value.as_str()))
}

fn xml_u32(tag: &str, name: &str) -> Option<u32> {
    xml_attr(tag, name)?.parse().ok()
}
