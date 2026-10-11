use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::MarkdownConversionError;
use crate::ooxml::Parts;

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// Store the rendered parts in a `.docx` ZIP.
///
/// Each part is deflated. Footnote drawings that still say `rId{file}` are
/// rewritten to `mdImageN` relationships before the package is closed.
pub fn pack(parts: &Parts) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut repaired = repair_footnote_images(parts)?;
    repair_document_images(&mut repaired)?;
    repair_story_images(&mut repaired.extras)?;
    crate::ooxml::install_package_metadata(&mut repaired)?;
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in repaired.files() {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        archive
            .start_file(name, options)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
        archive
            .write_all(bytes)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    }
    let cursor = archive
        .finish()
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    Ok(cursor.into_inner())
}

/// Read a `.docx` ZIP back into package parts.
///
/// Property parts stay in `extras`. `metadata` is `None`, so packing the
/// result leaves those parts alone until a caller sets metadata.
pub fn unpack(bytes: &[u8]) -> Result<Parts, MarkdownConversionError> {
    let mut content_types = None;
    let mut rels = None;
    let mut document = None;
    let mut document_rels = None;
    let mut styles = None;
    let mut numbering = None;
    let mut footnotes = None;
    let mut footnote_rels = None;
    let mut settings = None;
    let mut extras = Vec::new();
    for (name, body) in read_zip(bytes)? {
        match name.as_str() {
            "[Content_Types].xml" => content_types = Some(body),
            "_rels/.rels" => rels = Some(body),
            "word/document.xml" => document = Some(body),
            "word/_rels/document.xml.rels" => document_rels = Some(body),
            "word/styles.xml" => styles = Some(body),
            "word/numbering.xml" => numbering = Some(body),
            "word/footnotes.xml" => footnotes = Some(body),
            "word/_rels/footnotes.xml.rels" => footnote_rels = Some(body),
            "word/settings.xml" => settings = Some(body),
            _ => extras.push((name, body)),
        }
    }
    let missing =
        |path: &str| MarkdownConversionError::new(format!("Package part is missing {path}"));
    Ok(Parts {
        content_types: content_types.ok_or_else(|| missing("[Content_Types].xml"))?,
        rels: rels.ok_or_else(|| missing("_rels/.rels"))?,
        document: document.ok_or_else(|| missing("word/document.xml"))?,
        document_rels: document_rels.ok_or_else(|| missing("word/_rels/document.xml.rels"))?,
        styles: styles.ok_or_else(|| missing("word/styles.xml"))?,
        numbering: numbering.unwrap_or_default(),
        footnotes,
        footnote_rels,
        settings,
        extras,
        metadata: None,
    })
}

fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, MarkdownConversionError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
        let name = file
            .name()
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?
            .into_owned();
        if name.ends_with('/') {
            continue;
        }
        let mut body = Vec::new();
        file.read_to_end(&mut body)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
        files.push((name, body));
    }
    Ok(files)
}

/// Give unresolved `r:embed="rId{file}"` tokens in `word/footnotes.xml` real
/// relationship ids. The media part must already be in the package.
fn repair_footnote_images(parts: &Parts) -> Result<Parts, MarkdownConversionError> {
    let Some(footnotes) = &parts.footnotes else {
        return Ok(parts.clone());
    };
    let Ok(source) = std::str::from_utf8(footnotes) else {
        return Ok(parts.clone());
    };
    if !source.contains("rId{") {
        return Ok(parts.clone());
    }

    let names = parts
        .files()
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect::<BTreeSet<_>>();
    let mut rels_source = parts
        .footnote_rels
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or("")
        .to_string();
    let mut used_ids = relationship_ids(&rels_source);
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    let mut next_id = 1_u32;
    let mut rewritten = String::new();
    let mut cursor = 0;
    let mut changed = false;
    while let Some(offset) = source[cursor..].find("rId{") {
        let start = cursor + offset;
        let embed =
            source[..start].ends_with("r:embed=\"") || source[..start].ends_with("r:embed='");
        if !embed {
            rewritten.push_str(&source[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let name_start = start + 4;
        let Some(end) = source[name_start..].find('}') else {
            return Err(MarkdownConversionError::new(
                "Generated footnote image is missing from the package",
            ));
        };
        let name = &source[name_start..name_start + end];
        let media = format!("word/media/{name}");
        if !names.contains(&media) {
            return Err(MarkdownConversionError::new(
                "Generated footnote image is missing from the package",
            ));
        }
        let id = if let Some(id) = assigned.get(name) {
            id.clone()
        } else {
            while used_ids.contains(&format!("mdImage{next_id}")) {
                next_id = next_id.saturating_add(1);
            }
            let id = format!("mdImage{next_id}");
            next_id = next_id.saturating_add(1);
            used_ids.insert(id.clone());
            assigned.insert(name.to_string(), id.clone());
            id
        };
        rewritten.push_str(&source[cursor..start]);
        rewritten.push_str(&id);
        cursor = name_start + end + 1;
        changed = true;
    }
    if !changed {
        return Ok(parts.clone());
    }
    rewritten.push_str(&source[cursor..]);
    if rels_source.is_empty() {
        rels_source = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_NS}"></Relationships>"#
        );
    }
    let mut relationships = String::new();
    for (name, id) in &assigned {
        relationships.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{IMAGE_REL}" Target="media/{name}"/>"#
        ));
    }
    let rels = if let Some(close) = rels_source.rfind("</Relationships>") {
        let mut out = String::new();
        out.push_str(&rels_source[..close]);
        out.push_str(&relationships);
        out.push_str(&rels_source[close..]);
        out
    } else {
        rels_source.push_str(&relationships);
        rels_source
    };
    let mut updated = parts.clone();
    updated.footnotes = Some(rewritten.into_bytes());
    updated.footnote_rels = Some(rels.into_bytes());
    Ok(updated)
}

/// Body, header, and footer drawings use the same `rId{file}` token as
/// footnotes. Those parts get the next numeric `rId` instead of `mdImageN`.
fn repair_document_images(parts: &mut Parts) -> Result<(), MarkdownConversionError> {
    let media = media_names(parts);
    let document = std::str::from_utf8(&parts.document)
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?
        .to_string();
    let rels = std::str::from_utf8(&parts.document_rels)
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?
        .to_string();
    let (document, rels) = rewrite_image_embeds(&document, &rels, &media, "rId")?;
    parts.document = document.into_bytes();
    parts.document_rels = rels.into_bytes();
    Ok(())
}

fn repair_story_images(extras: &mut Vec<(String, Vec<u8>)>) -> Result<(), MarkdownConversionError> {
    let media = extras
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    let mut index = 0;
    while index < extras.len() {
        let path = extras[index].0.clone();
        if !is_story_xml(&path) {
            index += 1;
            continue;
        }
        let xml = std::str::from_utf8(&extras[index].1)
            .map_err(|err| MarkdownConversionError::new(err.to_string()))?
            .to_string();
        if !xml.contains("rId{") {
            index += 1;
            continue;
        }
        let rels_path = story_rels_path(&path);
        let rels_index = extras.iter().position(|(name, _)| name == &rels_path);
        let rels = rels_index
            .and_then(|rels_index| std::str::from_utf8(&extras[rels_index].1).ok())
            .unwrap_or("")
            .to_string();
        let (xml, rels) = rewrite_image_embeds(&xml, &rels, &media, "rId")?;
        extras[index].1 = xml.into_bytes();
        if let Some(rels_index) = rels_index {
            extras[rels_index].1 = rels.into_bytes();
        } else {
            extras.push((rels_path, rels.into_bytes()));
        }
        index += 1;
    }
    Ok(())
}

fn is_story_xml(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    (name.starts_with("header") || name.starts_with("footer"))
        && name.ends_with(".xml")
        && !name.ends_with(".xml.rels")
}

fn story_rels_path(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    format!("word/_rels/{name}.rels")
}

fn media_names(parts: &Parts) -> BTreeSet<String> {
    parts
        .files()
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

fn rewrite_image_embeds(
    source: &str,
    rels_source: &str,
    names: &BTreeSet<String>,
    id_prefix: &str,
) -> Result<(String, String), MarkdownConversionError> {
    if !source.contains("rId{") {
        return Ok((source.to_string(), rels_source.to_string()));
    }
    let mut rels_source = rels_source.to_string();
    let mut used_ids = relationship_ids(&rels_source);
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    let mut next_id = next_relationship_id(&used_ids);
    let mut rewritten = String::new();
    let mut cursor = 0;
    let mut changed = false;
    while let Some(offset) = source[cursor..].find("rId{") {
        let start = cursor + offset;
        let embed =
            source[..start].ends_with("r:embed=\"") || source[..start].ends_with("r:embed='");
        if !embed {
            rewritten.push_str(&source[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let name_start = start + 4;
        let Some(end) = source[name_start..].find('}') else {
            return Err(MarkdownConversionError::new(
                "Generated image is missing from the package",
            ));
        };
        let name = &source[name_start..name_start + end];
        let media = format!("word/media/{name}");
        if !names.contains(&media) {
            return Err(MarkdownConversionError::new(
                "Generated image is missing from the package",
            ));
        }
        let id = if let Some(id) = assigned.get(name) {
            id.clone()
        } else {
            let id = loop {
                let candidate = format!("{id_prefix}{next_id}");
                next_id = next_id.saturating_add(1);
                if !used_ids.contains(&candidate) {
                    break candidate;
                }
            };
            used_ids.insert(id.clone());
            assigned.insert(name.to_string(), id.clone());
            id
        };
        rewritten.push_str(&source[cursor..start]);
        rewritten.push_str(&id);
        cursor = name_start + end + 1;
        changed = true;
    }
    if !changed {
        return Ok((source.to_string(), rels_source));
    }
    rewritten.push_str(&source[cursor..]);
    if rels_source.is_empty() {
        rels_source = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{REL_NS}"></Relationships>"#
        );
    }
    let mut relationships = String::new();
    for (name, id) in &assigned {
        relationships.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{IMAGE_REL}" Target="media/{name}"/>"#
        ));
    }
    let rels = if let Some(close) = rels_source.rfind("</Relationships>") {
        let mut out = String::new();
        out.push_str(&rels_source[..close]);
        out.push_str(&relationships);
        out.push_str(&rels_source[close..]);
        out
    } else {
        rels_source.push_str(&relationships);
        rels_source
    };
    Ok((rewritten, rels))
}

fn next_relationship_id(used: &BTreeSet<String>) -> u32 {
    let mut max = 0_u32;
    for id in used {
        let Some(rest) = id.strip_prefix("rId") else {
            continue;
        };
        if let Ok(number) = rest.parse::<u32>() {
            max = max.max(number);
        }
    }
    max.saturating_add(1)
}

fn relationship_ids(rels: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut rest = rels;
    while let Some(index) = rest.find("Id=\"") {
        rest = &rest[index + 4..];
        let Some(end) = rest.find('"') else {
            break;
        };
        ids.insert(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    ids
}
