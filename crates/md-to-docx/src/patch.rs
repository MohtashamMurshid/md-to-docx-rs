//! Placeholder patch for an existing `.docx` package.
//!
//! Default delimiters are `{{` and `}}`. Each name replaces the first
//! paragraph that contains it. Markup inserted for that name is not scanned
//! again. Donor boundary markers come from a counter.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use regex::Regex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::convert::convert_with_options;
use crate::model::Block;
use crate::options::ConvertOptions;
use crate::parse::parse_markdown;

const START_DELIM: &str = "{{";
const END_DELIM: &str = "}}";
const MAX_COMPRESSED: usize = 16 * 1024 * 1024;
const MAX_UNCOMPRESSED: u64 = 64 * 1024 * 1024;
const MAX_ENTRY: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NUMBERING_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
const FOOTNOTES_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";

struct Package {
    order: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
}

struct Hit {
    part: String,
    start: usize,
    end: usize,
    patch_index: usize,
}

struct Offsets {
    num_id: u64,
    abstract_num_id: u64,
    footnote: u64,
    bookmark: u64,
}

/// Replace `{{name}}` paragraphs in `template` with Markdown from `patches`.
///
/// `patches` is `(placeholder name, markdown bytes)`. Only the first match of
/// each name is replaced. A name that never appears leaves that template part
/// byte-for-byte unchanged.
pub fn patch_placeholders(
    template: &[u8],
    patches: &[(&str, &[u8])],
) -> Result<Vec<u8>, crate::error::MarkdownConversionError> {
    patch_placeholders_with_options(template, patches, &ConvertOptions::default())
}

pub(crate) fn patch_placeholders_with_options(
    template: &[u8],
    patches: &[(&str, &[u8])],
    options: &ConvertOptions,
) -> Result<Vec<u8>, crate::error::MarkdownConversionError> {
    options.validate()?;
    validate_patches(patches)?;
    let markdown = patch_markdown(patches)?;
    for (name, text) in &markdown {
        reject_comments(name, text)?;
    }

    let package = read_package(template)?;
    let hits = find_hits(&package, patches);
    if hits.is_empty() {
        return Ok(template.to_vec());
    }

    let mut package = package;
    let mut originals = BTreeMap::new();
    for hit in &hits {
        if !originals.contains_key(&hit.part) {
            originals.insert(hit.part.clone(), xml_owned(&package, &hit.part)?);
        }
    }

    let mut counter = 1_u64;
    let mut ranges: BTreeMap<String, Vec<(usize, usize, String)>> = BTreeMap::new();
    let mut xmlns_donor: BTreeMap<String, String> = BTreeMap::new();

    for hit in &hits {
        let marker = boundary_marker(hit.patch_index, &mut counter);
        let text = &markdown[hit.patch_index].1;
        let donor_bytes = convert_with_options(text, options)?;
        let donor = read_package(&donor_bytes)?;
        let prefix = unused_prefix(&package);
        let offsets = offsets_from(&package);
        let fragment = donor_fragment(&donor, &marker, &prefix, &offsets)?;
        ranges
            .entry(hit.part.clone())
            .or_default()
            .push((hit.start, hit.end, fragment));
        xmlns_donor
            .entry(hit.part.clone())
            .or_insert(donor_document(&donor)?);
        merge_donor(&mut package, &donor, &prefix, &offsets, &hit.part)?;
    }

    for (part, part_ranges) in ranges {
        let original = originals.get(&part).map(String::as_str).unwrap_or("");
        let mut updated = apply_ranges(original, part_ranges);
        if let Some(donor) = xmlns_donor.get(&part) {
            updated = merge_xmlns(&updated, donor);
        }
        put(&mut package, &part, updated.into_bytes());
    }

    write_package(&package)
}

fn validate_patches(
    patches: &[(&str, &[u8])],
) -> Result<(), crate::error::MarkdownConversionError> {
    if patches.is_empty() {
        return Err(crate::error::MarkdownConversionError::new(
            "Invalid patches: At least one placeholder patch is required",
        ));
    }
    for (name, _) in patches {
        if name.trim().is_empty() {
            return Err(crate::error::MarkdownConversionError::new(
                "Invalid patch placeholder: Must be a non-empty string",
            ));
        }
    }
    Ok(())
}

fn patch_markdown(
    patches: &[(&str, &[u8])],
) -> Result<Vec<(String, String)>, crate::error::MarkdownConversionError> {
    let mut markdown = Vec::with_capacity(patches.len());
    for (name, bytes) in patches {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            crate::error::MarkdownConversionError::new("Patch markdown is not valid UTF-8")
        })?;
        markdown.push(((*name).to_string(), text.to_string()));
    }
    Ok(markdown)
}

fn reject_comments(
    placeholder: &str,
    markdown: &str,
) -> Result<(), crate::error::MarkdownConversionError> {
    let document = parse_markdown(markdown)?;
    if document.children.iter().any(block_has_comment) {
        let mut context = BTreeMap::new();
        context.insert("placeholder".to_string(), placeholder.to_string());
        return Err(crate::error::MarkdownConversionError::with_context(
            "Patch markdown does not support Word comments yet",
            context,
        ));
    }
    Ok(())
}

fn block_has_comment(block: &Block) -> bool {
    match block {
        Block::Comment(_) => true,
        Block::List(list) => list
            .children
            .iter()
            .any(|item| item.children.iter().any(block_has_comment)),
        Block::Blockquote(quote) => quote.children.iter().any(block_has_comment),
        Block::FootnoteDefinition { children, .. } => children.iter().any(block_has_comment),
        Block::Table(table) => table
            .header
            .iter()
            .chain(table.rows.iter().flatten())
            .any(|cell| cell.blocks.iter().any(block_has_comment)),
        Block::Paragraph(_)
        | Block::Heading(_)
        | Block::CodeBlock(_)
        | Block::Custom { .. }
        | Block::ThematicBreak
        | Block::PageBreak
        | Block::Caption(_)
        | Block::Image(_)
        | Block::DisplayMath { .. }
        | Block::ChartImage { .. } => false,
    }
}

fn boundary_marker(index: usize, counter: &mut u64) -> String {
    let value = *counter;
    *counter = counter.saturating_add(1);
    format!("MDPATCH_BOUNDARY_{index}_{value}")
}

fn find_hits(package: &Package, patches: &[(&str, &[u8])]) -> Vec<Hit> {
    let mut used = BTreeSet::new();
    let mut hits = Vec::new();
    for name in &package.order {
        if !is_story_part(name) {
            continue;
        }
        let Ok(xml) = xml_owned(package, name) else {
            continue;
        };
        for (start, end) in find_elements(&xml, "w:p") {
            let text = paragraph_text(&xml[start..end]);
            for (patch_index, (placeholder, _)) in patches.iter().enumerate() {
                if used.contains(placeholder) {
                    continue;
                }
                if text.contains(&placeholder_token(placeholder)) {
                    used.insert(*placeholder);
                    hits.push(Hit {
                        part: name.clone(),
                        start,
                        end,
                        patch_index,
                    });
                    break;
                }
            }
        }
    }
    hits
}

fn placeholder_token(name: &str) -> String {
    format!("{START_DELIM}{name}{END_DELIM}")
}

fn is_story_part(name: &str) -> bool {
    let Some(file) = name.strip_prefix("word/") else {
        return false;
    };
    if file.contains('/') {
        return false;
    }
    if file == "document.xml" || file == "footnotes.xml" || file == "endnotes.xml" {
        return true;
    }
    let rest = file
        .strip_prefix("header")
        .or_else(|| file.strip_prefix("footer"));
    let Some(rest) = rest else {
        return false;
    };
    let Some(number) = rest.strip_suffix(".xml") else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn donor_document(donor: &Package) -> Result<String, crate::error::MarkdownConversionError> {
    xml_owned(donor, "word/document.xml")
}

fn donor_fragment(
    donor: &Package,
    marker: &str,
    prefix: &str,
    offsets: &Offsets,
) -> Result<String, crate::error::MarkdownConversionError> {
    let document = donor_document(donor)?;
    let body = body_inner(&document)?;
    let marked = format!("<w:p><w:r><w:t xml:space=\"preserve\">{marker}</w:t></w:r></w:p>{body}");
    let end = find_elements(&marked, "w:p")
        .into_iter()
        .find(|(start, end)| paragraph_text(&marked[*start..*end]) == marker)
        .map(|(_, end)| end)
        .ok_or_else(|| crate::error::MarkdownConversionError::new("Malformed package XML"))?;
    let fragment = strip_tag(&marked[end..], "w:sectPr");
    Ok(retarget_story(&fragment, prefix, offsets))
}

fn body_inner(document: &str) -> Result<String, crate::error::MarkdownConversionError> {
    let start = document
        .find("<w:body")
        .ok_or_else(|| crate::error::MarkdownConversionError::new("Malformed package XML"))?;
    let open_end = document[start..]
        .find('>')
        .ok_or_else(|| crate::error::MarkdownConversionError::new("Malformed package XML"))?
        + start
        + 1;
    let close = document
        .rfind("</w:body>")
        .ok_or_else(|| crate::error::MarkdownConversionError::new("Malformed package XML"))?;
    if close < open_end {
        return Err(crate::error::MarkdownConversionError::new(
            "Malformed package XML",
        ));
    }
    Ok(document[open_end..close].to_string())
}

fn retarget_story(xml: &str, prefix: &str, offsets: &Offsets) -> String {
    let mut output = xml.to_string();
    output = add_offset(&output, r#"w:numId="(\d+)""#, offsets.num_id, |_| true);
    output = add_offset(
        &output,
        r#"<w:numId\b[^>]*w:val="(\d+)""#,
        offsets.num_id,
        |_| true,
    );
    output = add_offset(
        &output,
        r#"w:abstractNumId="(\d+)""#,
        offsets.abstract_num_id,
        |_| true,
    );
    output = add_offset(
        &output,
        r#"<w:abstractNumId\b[^>]*w:val="(\d+)""#,
        offsets.abstract_num_id,
        |_| true,
    );
    output = add_offset(
        &output,
        r#"<w:footnote\b[^>]*w:id="(\d+)""#,
        offsets.footnote,
        |id| id > 0,
    );
    output = add_offset(
        &output,
        r#"<w:footnoteReference\b[^>]*w:id="(\d+)""#,
        offsets.footnote,
        |id| id > 0,
    );
    output = add_offset(
        &output,
        r#"<w:bookmarkStart\b[^>]*w:id="(\d+)""#,
        offsets.bookmark,
        |_| true,
    );
    output = add_offset(
        &output,
        r#"<w:bookmarkEnd\b[^>]*w:id="(\d+)""#,
        offsets.bookmark,
        |_| true,
    );
    output = prefix_bookmarks(&output, prefix);
    prefix_rel_attrs(&output, prefix)
}

fn merge_donor(
    package: &mut Package,
    donor: &Package,
    prefix: &str,
    offsets: &Offsets,
    part: &str,
) -> Result<(), crate::error::MarkdownConversionError> {
    if let Some(styles) = optional_xml(donor, "word/styles.xml") {
        let target = xml_owned(package, "word/styles.xml").unwrap_or_else(|_| {
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:styles>"#.to_string()
        });
        put(
            package,
            "word/styles.xml",
            merge_styles(&target, &styles).into_bytes(),
        );
    }
    if let Some(numbering) = optional_xml(donor, "word/numbering.xml") {
        let shifted = retarget_story(&numbering, prefix, offsets);
        merge_numbered_part(
            package,
            "word/numbering.xml",
            &shifted,
            &["w:abstractNum", "w:num"],
            "w:numbering",
        );
        ensure_override(package, "/word/numbering.xml", NUMBERING_TYPE);
        ensure_part_relationship(package, prefix, "numbering");
    }
    if let Some(footnotes) = optional_xml(donor, "word/footnotes.xml") {
        let shifted = retarget_story(&footnotes, prefix, offsets);
        merge_numbered_part(
            package,
            "word/footnotes.xml",
            &shifted,
            &["w:footnote"],
            "w:footnotes",
        );
        ensure_override(package, "/word/footnotes.xml", FOOTNOTES_TYPE);
        ensure_part_relationship(package, prefix, "footnotes");
        import_media_rels(
            package,
            donor,
            "word/_rels/footnotes.xml.rels",
            "word/_rels/footnotes.xml.rels",
            prefix,
        )?;
    }
    import_media_rels(
        package,
        donor,
        "word/_rels/document.xml.rels",
        &rels_path(part),
        prefix,
    )?;
    if let Some(types) = optional_xml(donor, "[Content_Types].xml") {
        copy_missing_defaults(package, &types);
    }
    Ok(())
}

fn merge_numbered_part(
    package: &mut Package,
    name: &str,
    donor_xml: &str,
    children: &[&str],
    root: &str,
) {
    let Some(mut target) = optional_xml(package, name) else {
        put(package, name, donor_xml.as_bytes().to_vec());
        return;
    };
    for child in children {
        let id_attr = match *child {
            "w:abstractNum" => "w:abstractNumId",
            "w:footnote" => "w:id",
            _ => "w:numId",
        };
        let mut present = existing_ids(&target, child, id_attr);
        for (start, end) in find_elements(donor_xml, child) {
            let element = &donor_xml[start..end];
            if let Some(id) = attr(element, id_attr) {
                if !present.insert(id) {
                    continue;
                }
            }
            target = insert_before_close(&target, &format!("</{root}>"), element);
        }
    }
    put(package, name, target.into_bytes());
}

fn existing_ids(xml: &str, tag: &str, id_attr: &str) -> BTreeSet<String> {
    find_elements(xml, tag)
        .into_iter()
        .filter_map(|(start, end)| attr(&xml[start..end], id_attr))
        .collect()
}

fn merge_styles(target: &str, donor: &str) -> String {
    let mut output = target.to_string();
    for (start, end) in find_elements(donor, "w:style") {
        let element = &donor[start..end];
        let Some(id) = attr(element, "w:styleId") else {
            continue;
        };
        if output.contains(&format!(r#"w:styleId="{id}""#)) {
            continue;
        }
        output = insert_before_close(&output, "</w:styles>", element);
    }
    output
}

fn ensure_override(package: &mut Package, part_name: &str, content_type: &str) {
    let Some(mut types) = optional_xml(package, "[Content_Types].xml") else {
        return;
    };
    if types.contains(&format!(r#"PartName="{part_name}""#)) {
        return;
    }
    let element = format!(r#"<Override PartName="{part_name}" ContentType="{content_type}"/>"#);
    types = insert_before_close(&types, "</Types>", &element);
    put(package, "[Content_Types].xml", types.into_bytes());
}

fn ensure_part_relationship(package: &mut Package, prefix: &str, local: &str) {
    let key = "word/_rels/document.xml.rels";
    let rel_type = format!("{OFFICE_REL}/{local}");
    let mut rels = optional_xml(package, key).unwrap_or_else(|| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_NS}"></Relationships>"#
        )
    });
    if rels.contains(&rel_type) {
        return;
    }
    let element =
        format!(r#"<Relationship Id="{prefix}{local}" Type="{rel_type}" Target="{local}.xml"/>"#);
    rels = insert_before_close(&rels, "</Relationships>", &element);
    put(package, key, rels.into_bytes());
}

fn copy_missing_defaults(package: &mut Package, donor_types: &str) {
    let Some(mut types) = optional_xml(package, "[Content_Types].xml") else {
        return;
    };
    for (start, end) in find_elements(donor_types, "Default") {
        let element = &donor_types[start..end];
        let Some(extension) = attr(element, "Extension") else {
            continue;
        };
        if types.contains(&format!(r#"Extension="{extension}""#)) {
            continue;
        }
        types = insert_before_close(&types, "</Types>", element);
    }
    put(package, "[Content_Types].xml", types.into_bytes());
}

fn import_media_rels(
    package: &mut Package,
    donor: &Package,
    donor_rels_name: &str,
    target_rels_name: &str,
    prefix: &str,
) -> Result<(), crate::error::MarkdownConversionError> {
    let Some(rels) = optional_xml(donor, donor_rels_name) else {
        return Ok(());
    };
    let mut additions = String::new();
    for (start, end) in find_elements(&rels, "Relationship") {
        let element = &rels[start..end];
        let rel_type = attr(element, "Type").unwrap_or_default();
        if !rel_type.ends_with("/image") && !rel_type.ends_with("/hyperlink") {
            continue;
        }
        let id = attr(element, "Id").unwrap_or_default();
        let target = attr(element, "Target").unwrap_or_default();
        let mode = attr(element, "TargetMode");
        let new_target = if mode.as_deref() == Some("External") {
            target
        } else {
            if !target.starts_with("media/") {
                return Err(crate::error::MarkdownConversionError::new(
                    "Missing generated media dependency",
                ));
            }
            let source = format!("word/{target}");
            let Some(bytes) = donor.files.get(&source) else {
                return Err(crate::error::MarkdownConversionError::new(
                    "Missing generated media dependency",
                ));
            };
            let renamed = format!("media/{prefix}{}", &target["media/".len()..]);
            put(package, &format!("word/{renamed}"), bytes.clone());
            renamed
        };
        let mode_attr = match &mode {
            Some(mode) => format!(r#" TargetMode="{mode}""#),
            None => String::new(),
        };
        additions.push_str(&format!(
            r#"<Relationship Id="{prefix}{id}" Type="{rel_type}" Target="{new_target}"{mode_attr}/>"#
        ));
    }
    if additions.is_empty() {
        return Ok(());
    }
    let current = optional_xml(package, target_rels_name).unwrap_or_else(|| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_NS}"></Relationships>"#
        )
    });
    let updated = insert_before_close(&current, "</Relationships>", &additions);
    put(package, target_rels_name, updated.into_bytes());
    Ok(())
}

fn rels_path(part: &str) -> String {
    let file = part.rsplit('/').next().unwrap_or(part);
    format!("word/_rels/{file}.rels")
}

fn offsets_from(package: &Package) -> Offsets {
    let text = package_xml_text(package);
    Offsets {
        num_id: max_id(&text, r#"w:numId="(\d+)""#)
            + max_id(&text, r#"<w:numId\b[^>]*w:val="(\d+)""#)
            + 1,
        abstract_num_id: max_id(&text, r#"w:abstractNumId="(\d+)""#) + 1,
        footnote: max_id(&text, r#"<w:footnote\b[^>]*w:id="(\d+)""#) + 1,
        bookmark: max_id(&text, r#"<w:bookmarkStart\b[^>]*w:id="(\d+)""#),
    }
}

fn package_xml_text(package: &Package) -> String {
    let mut text = String::new();
    for (name, bytes) in &package.files {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            if let Ok(xml) = std::str::from_utf8(bytes) {
                text.push_str(xml);
            }
        }
    }
    text
}

fn unused_prefix(package: &Package) -> String {
    let mut index = 1_u32;
    loop {
        let token = format!("mdp{index}_");
        let used = package.files.iter().any(|(name, bytes)| {
            name.contains(&token)
                || std::str::from_utf8(bytes).is_ok_and(|text| text.contains(&token))
        });
        if !used {
            return token;
        }
        index = index.saturating_add(1);
    }
}

fn max_id(text: &str, pattern: &str) -> u64 {
    let re = Regex::new(pattern).expect("id pattern");
    re.captures_iter(text)
        .filter_map(|caps| caps.get(1)?.as_str().parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

fn add_offset(xml: &str, pattern: &str, offset: u64, shift: impl Fn(u64) -> bool) -> String {
    let re = Regex::new(pattern).expect("offset pattern");
    re.replace_all(xml, |caps: &regex::Captures| {
        let id: u64 = caps
            .get(1)
            .and_then(|value| value.as_str().parse().ok())
            .unwrap_or(0);
        let full = caps.get(0).map(|value| value.as_str()).unwrap_or("");
        if !shift(id) {
            return full.to_string();
        }
        let number = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        match full.rfind(number) {
            Some(start) => format!(
                "{}{}{}",
                &full[..start],
                id.saturating_add(offset),
                &full[start + number.len()..]
            ),
            None => full.to_string(),
        }
    })
    .into_owned()
}

fn prefix_bookmarks(xml: &str, prefix: &str) -> String {
    let mut names = Vec::new();
    for (start, end) in find_elements(xml, "w:bookmarkStart") {
        if let Some(name) = attr(&xml[start..end], "w:name") {
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    let mut map = BTreeMap::new();
    for (index, name) in names.iter().enumerate() {
        let stem = name.chars().take(24).collect::<String>();
        map.insert(name.clone(), format!("{prefix}{stem}_{index}"));
    }
    let mut output = xml.to_string();
    for (name, renamed) in &map {
        output = output.replace(
            &format!(r#"w:name="{name}""#),
            &format!(r#"w:name="{renamed}""#),
        );
        output = output.replace(
            &format!(r#"w:anchor="{name}""#),
            &format!(r#"w:anchor="{renamed}""#),
        );
    }
    let re = Regex::new(r"\b(REF|PAGEREF|NOTEREF)\s+(\S+)").expect("field pattern");
    re.replace_all(&output, |caps: &regex::Captures| {
        let field = caps.get(1).map(|value| value.as_str()).unwrap_or("");
        let name = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        let renamed = map.get(name).map(String::as_str).unwrap_or(name);
        format!("{field} {renamed}")
    })
    .into_owned()
}

fn prefix_rel_attrs(xml: &str, prefix: &str) -> String {
    let re = Regex::new(r#"\br:(id|embed|link)="([^"]*)""#).expect("rel pattern");
    re.replace_all(xml, |caps: &regex::Captures| {
        let kind = caps.get(1).map(|value| value.as_str()).unwrap_or("id");
        let value = caps.get(2).map(|value| value.as_str()).unwrap_or("");
        format!(r#"r:{kind}="{prefix}{value}""#)
    })
    .into_owned()
}

fn apply_ranges(xml: &str, mut ranges: Vec<(usize, usize, String)>) -> String {
    ranges.sort_by_key(|(start, _, _)| *start);
    let mut output = String::new();
    let mut cursor = 0;
    for (start, end, replacement) in ranges {
        if start < cursor || end > xml.len() || start > end {
            continue;
        }
        output.push_str(&xml[cursor..start]);
        output.push_str(&replacement);
        cursor = end;
    }
    output.push_str(&xml[cursor..]);
    output
}

fn merge_xmlns(target: &str, donor: &str) -> String {
    let Some(donor_start) = donor.find("<w:document") else {
        return target.to_string();
    };
    let Some(donor_rel) = donor[donor_start..].find('>') else {
        return target.to_string();
    };
    let donor_open = &donor[donor_start..donor_start + donor_rel];
    let Some(target_start) = target.find("<w:document") else {
        return target.to_string();
    };
    let Some(target_rel) = target[target_start..].find('>') else {
        return target.to_string();
    };
    let target_end = target_start + target_rel;
    let re = Regex::new(r#"xmlns:[A-Za-z0-9]+="[^"]*""#).expect("xmlns pattern");
    let mut insertion = String::new();
    for caps in re.captures_iter(donor_open) {
        let attr = caps.get(0).map(|value| value.as_str()).unwrap_or("");
        let name = attr.split('=').next().unwrap_or("");
        if name.is_empty() || target[..target_end].contains(&format!("{name}=")) {
            continue;
        }
        insertion.push(' ');
        insertion.push_str(attr);
    }
    if insertion.is_empty() {
        return target.to_string();
    }
    let mut output = String::new();
    output.push_str(&target[..target_end]);
    output.push_str(&insertion);
    output.push_str(&target[target_end..]);
    output
}

fn strip_tag(xml: &str, tag: &str) -> String {
    let mut output = xml.to_string();
    while let Some((start, end)) = find_elements(&output, tag).into_iter().next() {
        if end <= start || end > output.len() {
            break;
        }
        output.replace_range(start..end, "");
    }
    output
}

fn find_elements(xml: &str, tag: &str) -> Vec<(usize, usize)> {
    let open = format!("<{tag}");
    let mut spans = Vec::new();
    let mut index = 0;
    while index < xml.len() {
        let Some(relative) = xml[index..].find(&open) else {
            break;
        };
        let start = index + relative;
        let after = start + open.len();
        let boundary = xml
            .as_bytes()
            .get(after)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>'));
        if !boundary {
            index = after;
            continue;
        }
        let Some(end) = element_end(xml, start, tag) else {
            index = after;
            continue;
        };
        if end <= start {
            index = after;
            continue;
        }
        spans.push((start, end));
        index = end;
    }
    spans
}

fn element_end(xml: &str, start: usize, tag: &str) -> Option<usize> {
    let open_rel = xml[start..].find('>')?;
    let open_end = start + open_rel;
    if xml.as_bytes().get(open_end.wrapping_sub(1)) == Some(&b'/') {
        return Some(open_end + 1);
    }
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1_i32;
    let mut index = open_end + 1;
    while depth > 0 {
        let next_open = xml[index..]
            .find(&open)
            .map(|relative| index + relative)
            .filter(|position| {
                let after = position + open.len();
                xml.as_bytes()
                    .get(after)
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>'))
            });
        let next_close = xml[index..].find(&close).map(|relative| index + relative);
        match (next_open, next_close) {
            (Some(open_at), Some(close_at)) if open_at < close_at => {
                let open_tag_end = xml[open_at..].find('>')? + open_at;
                let self_closing = xml.as_bytes().get(open_tag_end.wrapping_sub(1)) == Some(&b'/');
                if !self_closing {
                    depth += 1;
                }
                index = open_tag_end + 1;
            }
            (_, Some(close_at)) => {
                depth -= 1;
                index = close_at + close.len();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => return None,
        }
    }
    None
}

fn paragraph_text(paragraph: &str) -> String {
    let mut text = String::new();
    for (start, end) in find_elements(paragraph, "w:t") {
        let element = &paragraph[start..end];
        let Some(open_end) = element.find('>') else {
            continue;
        };
        if element.as_bytes().get(open_end.wrapping_sub(1)) == Some(&b'/') {
            continue;
        }
        let inner = element[open_end + 1..].strip_suffix("</w:t>").unwrap_or("");
        text.push_str(&decode_xml(inner));
    }
    text
}

fn decode_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn attr(element: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"{name}="([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    Some(re.captures(element)?.get(1)?.as_str().to_string())
}

fn insert_before_close(xml: &str, close: &str, insertion: &str) -> String {
    let Some(position) = xml.rfind(close) else {
        let mut output = xml.to_string();
        output.push_str(insertion);
        return output;
    };
    let mut output = String::new();
    output.push_str(&xml[..position]);
    output.push_str(insertion);
    output.push_str(&xml[position..]);
    output
}

fn optional_xml(package: &Package, name: &str) -> Option<String> {
    let bytes = package.files.get(name)?;
    String::from_utf8(bytes.clone()).ok()
}

fn xml_owned(
    package: &Package,
    name: &str,
) -> Result<String, crate::error::MarkdownConversionError> {
    optional_xml(package, name)
        .ok_or_else(|| crate::error::MarkdownConversionError::new("Malformed package XML"))
}

fn put(package: &mut Package, name: &str, bytes: Vec<u8>) {
    if !package.files.contains_key(name) {
        package.order.push(name.to_string());
    }
    package.files.insert(name.to_string(), bytes);
}

fn read_package(bytes: &[u8]) -> Result<Package, crate::error::MarkdownConversionError> {
    if bytes.len() > MAX_COMPRESSED {
        return Err(reference_error(
            "Reference DOCX exceeds the compressed package limit",
            "PACKAGE_LIMIT_EXCEEDED",
            &[
                ("limit", MAX_COMPRESSED.to_string()),
                ("actual", bytes.len().to_string()),
            ],
        ));
    }
    if bytes.len() < 4 {
        return Err(reference_error(
            "Reference DOCX is not a valid ZIP package",
            "INVALID_ZIP",
            &[],
        ));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        reference_error(
            "Reference DOCX is not a valid ZIP package",
            "INVALID_ZIP",
            &[],
        )
    })?;
    if archive.len() > MAX_ENTRIES {
        return Err(reference_error(
            format!(
                "Reference DOCX contains {} ZIP entries; limit is {MAX_ENTRIES}",
                archive.len()
            ),
            "PACKAGE_LIMIT_EXCEEDED",
            &[
                ("limit", MAX_ENTRIES.to_string()),
                ("actual", archive.len().to_string()),
            ],
        ));
    }
    let mut package = Package {
        order: Vec::new(),
        files: BTreeMap::new(),
    };
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| {
            reference_error(
                "Reference DOCX is not a valid ZIP package",
                "INVALID_ZIP",
                &[],
            )
        })?;
        let name = file.name().map_err(|_| {
            reference_error(
                "Reference DOCX is not a valid ZIP package",
                "INVALID_ZIP",
                &[],
            )
        })?;
        let name = name.into_owned();
        if unsafe_entry(&name) {
            return Err(reference_error(
                format!("Reference DOCX contains an unsafe ZIP entry path: {name}"),
                "UNSAFE_ENTRY_PATH",
                &[("entry", name)],
            ));
        }
        let size = file.size();
        if size > MAX_ENTRY {
            return Err(reference_error(
                format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
                "PACKAGE_LIMIT_EXCEEDED",
                &[
                    ("entry", name),
                    ("limit", MAX_ENTRY.to_string()),
                    ("actual", size.to_string()),
                ],
            ));
        }
        total = total.saturating_add(size);
        if total > MAX_UNCOMPRESSED {
            return Err(reference_error(
                "Reference DOCX exceeds the total uncompressed package limit",
                "PACKAGE_LIMIT_EXCEEDED",
                &[
                    ("limit", MAX_UNCOMPRESSED.to_string()),
                    ("actual", total.to_string()),
                ],
            ));
        }
        if file.is_dir() {
            continue;
        }
        let mut payload = Vec::new();
        file.by_ref()
            .take(MAX_ENTRY + 1)
            .read_to_end(&mut payload)
            .map_err(|_| {
                reference_error(
                    format!("Reference DOCX package part {name} could not be decompressed"),
                    "INVALID_ZIP",
                    &[("entry", name.clone())],
                )
            })?;
        if payload.len() as u64 > MAX_ENTRY {
            return Err(reference_error(
                format!("Reference DOCX entry {name} exceeds the uncompressed entry limit"),
                "PACKAGE_LIMIT_EXCEEDED",
                &[
                    ("entry", name),
                    ("limit", MAX_ENTRY.to_string()),
                    ("actual", payload.len().to_string()),
                ],
            ));
        }
        if !package.files.contains_key(&name) {
            package.order.push(name.clone());
        }
        package.files.insert(name, payload);
    }
    for entry in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/document.xml",
        "word/_rels/document.xml.rels",
        "word/styles.xml",
    ] {
        if !package.files.contains_key(entry) {
            return Err(reference_error(
                format!("Reference DOCX is missing required package part {entry}"),
                "MISSING_PACKAGE_PART",
                &[("entry", entry.to_string())],
            ));
        }
    }
    Ok(package)
}

fn unsafe_entry(name: &str) -> bool {
    if name.contains('\0') || name.contains('\\') {
        return true;
    }
    let normalized = name.replace('\\', "/");
    normalized.starts_with('/')
        || normalized.split('/').any(|segment| segment == "..")
        || normalized
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && normalized.chars().nth(1) == Some(':')
}

fn reference_error(
    message: impl Into<String>,
    code: &str,
    extra: &[(&str, String)],
) -> crate::error::MarkdownConversionError {
    let mut context = BTreeMap::new();
    context.insert("phase".to_string(), "reference-docx".to_string());
    context.insert("code".to_string(), code.to_string());
    for (key, value) in extra {
        context.insert((*key).to_string(), value.clone());
    }
    crate::error::MarkdownConversionError::with_context(message, context)
}

fn write_package(package: &Package) -> Result<Vec<u8>, crate::error::MarkdownConversionError> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut seen = BTreeSet::new();
    for name in &package.order {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(bytes) = package.files.get(name) else {
            continue;
        };
        archive
            .start_file(name, options)
            .map_err(|err| crate::error::MarkdownConversionError::new(err.to_string()))?;
        archive
            .write_all(bytes)
            .map_err(|err| crate::error::MarkdownConversionError::new(err.to_string()))?;
    }
    for (name, bytes) in &package.files {
        if !seen.insert(name.clone()) {
            continue;
        }
        archive
            .start_file(name, options)
            .map_err(|err| crate::error::MarkdownConversionError::new(err.to_string()))?;
        archive
            .write_all(bytes)
            .map_err(|err| crate::error::MarkdownConversionError::new(err.to_string()))?;
    }
    let cursor = archive
        .finish()
        .map_err(|err| crate::error::MarkdownConversionError::new(err.to_string()))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_with_document(document: &str) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let files = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/document.xml", document),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#,
            ),
            (
                "word/styles.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:styles>"#,
            ),
        ];
        for (name, body) in files {
            archive.start_file(name, options).unwrap();
            archive.write_all(body.as_bytes()).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    fn document(paragraphs: &[&str]) -> String {
        let mut body = String::new();
        for paragraph in paragraphs {
            body.push_str("<w:p><w:r><w:t>");
            body.push_str(paragraph);
            body.push_str("</w:t></w:r></w:p>");
        }
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
        )
    }

    fn part(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut file = archive.by_name(name).expect(name);
        let mut out = Vec::new();
        file.read_to_end(&mut out).expect("read");
        out
    }

    #[test]
    fn missing_placeholder_leaves_template_part_unchanged() {
        let document = document(&["Keep this paragraph."]);
        let template = template_with_document(&document);
        let patched = patch_placeholders(&template, &[("body", b"Inserted text")]).unwrap();
        assert_eq!(part(&patched, "word/document.xml"), document.as_bytes());
    }

    #[test]
    fn boundary_markers_come_from_a_counter() {
        let mut counter = 1_u64;
        assert_eq!(boundary_marker(0, &mut counter), "MDPATCH_BOUNDARY_0_1");
        assert_eq!(boundary_marker(1, &mut counter), "MDPATCH_BOUNDARY_1_2");
    }

    #[test]
    fn first_match_is_replaced_and_inserted_text_is_not_scanned() {
        let source = document(&["{{body}}", "{{body}}", "{{second}}"]);
        let template = template_with_document(&source);
        let patched = patch_placeholders(
            &template,
            &[("body", b"Literal {{second}}"), ("second", b"Replaced")],
        )
        .unwrap();
        let xml = String::from_utf8(part(&patched, "word/document.xml")).unwrap();
        assert!(xml.contains("Literal {{second}}"));
        assert!(xml.contains("Replaced"));
        assert!(xml.contains("{{body}}"));
        assert!(!xml.contains("MDPATCH_BOUNDARY_"));
        let again = patch_placeholders(
            &template,
            &[("body", b"Literal {{second}}"), ("second", b"Replaced")],
        )
        .unwrap();
        assert_eq!(
            part(&again, "word/document.xml"),
            part(&patched, "word/document.xml")
        );
    }

    #[test]
    fn patch_comments_are_rejected() {
        let template = template_with_document(&document(&["{{body}}"]));
        let error = patch_placeholders(&template, &[("body", b"<!-- COMMENT: reviewer note -->")])
            .unwrap_err();
        assert_eq!(
            error.message(),
            "Patch markdown does not support Word comments yet"
        );
    }
}
