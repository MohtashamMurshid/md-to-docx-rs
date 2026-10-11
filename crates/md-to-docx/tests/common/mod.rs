//! Unzip a `.docx` and project the parts slice 1 and slice 2 compare.
//!
//! ZIP bytes are not compared. `python3`'s zipfile inflates the package so this
//! test crate does not need its own `zip` dependency. The XML reader accepts
//! the WordprocessingML shape both the TypeScript `docx` package and a
//! `quick-xml` writer emit.
//!
//! Slice 2 adds bookmark names, page breaks, footnote references, `fldSimple`
//! and `instrText` instructions, and user footnotes from `word/footnotes.xml`.
//! Slice 3 adds section properties, header and footer stories, `w:updateFields`,
//! paragraph shading, left borders, tab stops, `fldChar` types, run font, fill,
//! and internal hyperlink anchors, plus table layout.
//! Slice 5 adds media bytes and drawing `descr`, name, title, and extents.
//! Slice 6 adds OMML `m:oMath` tags and math run text. Chart PNGs use the
//! same media bytes. Paragraph spacing is there for Mermaid code spacers.
//! Slice 7 adds table `gridSpan`, `vMerge`, grid column widths, and tables
//! nested inside a cell.
//! Slice 8 keeps every part name, including binary media such as
//! `word/media/image1.png`. UTF-8 parts keep their text. A part that is not
//! UTF-8 is stored under its ZIP name with empty text, and its bytes stay in
//! `media` when the path is `word/media/`. Run `rStyle` and table `tblStyle`
//! are projected too. Header and footer parts nested under a reference
//! package are stories too.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

const UNZIP_PY: &str = r#"
import sys, zipfile
from io import BytesIO
blob = sys.stdin.buffer.read()
with zipfile.ZipFile(BytesIO(blob)) as package:
    names = package.namelist()
    sys.stdout.buffer.write(f"{len(names)}\n".encode())
    for name in names:
        data = package.read(name)
        sys.stdout.buffer.write(name.encode("utf-8") + b"\n")
        sys.stdout.buffer.write(f"{len(data)}\n".encode())
        sys.stdout.buffer.write(data)
"#;

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Document {
    pub blocks: Vec<Block>,
    pub drawing_count: usize,
    pub footnotes: Vec<Footnote>,
    pub footnote_rels: Vec<Relationship>,
    pub media_parts: Vec<String>,
    pub media: Vec<MediaFile>,
    pub sections: Vec<SectionProps>,
    pub stories: Vec<Story>,
    pub update_fields: bool,
    /// Package parts keyed by ZIP path. UTF-8 parts keep their text. A part
    /// that is not UTF-8 still keeps its name, with empty text. Binary media
    /// bytes also stay in `media`.
    pub parts: BTreeMap<String, String>,
}

#[derive(Debug)]
#[allow(dead_code, clippy::large_enum_variant)] // Test projection keeps values inline.
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Paragraph {
    pub text: String,
    pub heading: Option<String>,
    pub style_id: Option<String>,
    pub align: Option<String>,
    pub rule: bool,
    pub list: Option<ListInfo>,
    pub runs: Vec<Run>,
    pub page_break: bool,
    pub bookmarks: Vec<String>,
    pub footnote_refs: Vec<String>,
    pub fields: Vec<String>,
    pub field_chars: Vec<String>,
    pub shade: Option<String>,
    pub left_border_color: Option<String>,
    pub left_border_size: Option<String>,
    pub tabs: Vec<TabStop>,
    pub drawings: Vec<Drawing>,
    pub math: Vec<MathExpr>,
    pub spacing_before: Option<String>,
    pub spacing_after: Option<String>,
}

/// One `m:oMath`. `tags` are the OMML locals the TeX subset emits, and `text`
/// is the concatenated `m:t` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathExpr {
    pub tags: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    pub name: Option<String>,
    pub descr: Option<String>,
    pub title: Option<String>,
    pub cx: Option<u64>,
    pub cy: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Footnote {
    pub id: String,
    pub paragraphs: Vec<Paragraph>,
    pub drawing_count: usize,
    pub embeds: Vec<String>,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInfo {
    pub num_id: String,
    pub level: u32,
    pub start: u32,
    pub ordered: bool,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    pub anchor: Option<String>,
    pub size: Option<u32>,
    pub color: Option<String>,
    pub font: Option<String>,
    pub fill: Option<String>,
    pub style_id: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Table {
    pub columns: usize,
    pub width: Option<String>,
    pub width_type: Option<String>,
    pub layout: Option<String>,
    pub column_widths: Vec<String>,
    pub style_id: Option<String>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabStop {
    pub val: String,
    pub pos: String,
    pub leader: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryRef {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionProps {
    pub break_type: Option<String>,
    pub page_width: Option<u32>,
    pub page_height: Option<u32>,
    pub orientation: Option<String>,
    pub margin_top: Option<u32>,
    pub margin_right: Option<u32>,
    pub margin_bottom: Option<u32>,
    pub margin_left: Option<u32>,
    pub margin_header: Option<u32>,
    pub margin_footer: Option<u32>,
    pub gutter: Option<u32>,
    pub title_page: bool,
    pub page_start: Option<u32>,
    pub page_format: Option<String>,
    pub page_separator: Option<String>,
    pub header_refs: Vec<StoryRef>,
    pub footer_refs: Vec<StoryRef>,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Story {
    pub path: String,
    pub blocks: Vec<Block>,
    pub drawing_count: usize,
}

#[derive(Debug)]
#[allow(dead_code)] // Each parity suite reads a different subset of the shared projection.
pub struct Cell {
    pub paragraphs: Vec<Paragraph>,
    pub tables: Vec<Table>,
    pub grid_span: Option<u32>,
    pub v_merge: Option<String>,
}

impl Document {
    pub fn paragraphs(&self) -> Vec<&Paragraph> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph),
                Block::Table(_) => None,
            })
            .collect()
    }

    #[allow(dead_code)] // Used by some, but not all, parity suites.
    pub fn tables(&self) -> Vec<&Table> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Table(table) => Some(table),
                Block::Paragraph(_) => None,
            })
            .collect()
    }

    #[allow(dead_code)] // Used by some, but not all, parity suites.
    pub fn paragraph_texts(&self) -> Vec<&str> {
        self.paragraphs()
            .into_iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect()
    }

    #[allow(dead_code)] // Used by some, but not all, parity suites.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Paragraph(paragraph) => lines.push(render_paragraph(paragraph, "")),
                Block::Table(table) => lines.push(render_table(table)),
            }
        }
        if self.drawing_count > 0 {
            lines.push(format!("drawings={}", self.drawing_count));
        }
        if !self.footnotes.is_empty() {
            lines.push("FOOTNOTES".to_string());
            for note in &self.footnotes {
                lines.push(format!(
                    "  id={} drawings={} embeds={}",
                    note.id,
                    note.drawing_count,
                    note.embeds.join(",")
                ));
                for paragraph in &note.paragraphs {
                    lines.push(render_paragraph(paragraph, "  "));
                }
            }
        }
        for rel in &self.footnote_rels {
            lines.push(format!("FNREL {} {} {}", rel.id, rel.rel_type, rel.target));
        }
        if !self.media_parts.is_empty() {
            lines.push(format!("media={}", self.media_parts.join(",")));
        }
        if !self.media.is_empty() {
            lines.push(format!(
                "media-bytes={}",
                self.media
                    .iter()
                    .map(|file| format!("{}:{}", file.path, file.bytes.len()))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if self.update_fields {
            lines.push("updateFields".to_string());
        }
        for section in &self.sections {
            lines.push(format!(
                "SECT type={} {}x{} {} margins={}/{}/{}/{} title={} start={:?} fmt={:?} sep={:?} headers={} footers={}",
                section.break_type.as_deref().unwrap_or("-"),
                section.page_width.unwrap_or(0),
                section.page_height.unwrap_or(0),
                section.orientation.as_deref().unwrap_or("-"),
                section.margin_top.unwrap_or(0),
                section.margin_right.unwrap_or(0),
                section.margin_bottom.unwrap_or(0),
                section.margin_left.unwrap_or(0),
                section.title_page,
                section.page_start,
                section.page_format,
                section.page_separator,
                section.header_refs.len(),
                section.footer_refs.len(),
            ));
        }
        if !self.parts.is_empty() {
            lines.push(format!("parts={}", self.parts.len()));
        }
        for story in &self.stories {
            lines.push(format!(
                "STORY {} drawings={}",
                story.path, story.drawing_count
            ));
            for block in &story.blocks {
                match block {
                    Block::Paragraph(paragraph) => lines.push(render_paragraph(paragraph, "  ")),
                    Block::Table(table) => lines.push(render_table(table)),
                }
            }
        }
        lines.join("\n")
    }
}

pub fn project_docx(bytes: &[u8]) -> Document {
    let files = unzip(bytes);
    let document_xml = require_part(&files, "word/document.xml");
    let document = parse_xml(document_xml);
    let numbering = files
        .get("word/numbering.xml")
        .map(|xml| {
            let xml = std::str::from_utf8(xml).expect("word/numbering.xml is not utf-8");
            load_numbering(&parse_xml(xml))
        })
        .unwrap_or_default();
    let rels = files
        .get("word/_rels/document.xml.rels")
        .map(|xml| {
            let xml = std::str::from_utf8(xml).expect("document.xml.rels is not utf-8");
            load_rels(&parse_xml(xml))
        })
        .unwrap_or_default();
    let body = find_first(&document, "body").unwrap_or(&document);
    let footnote_rels = files
        .get("word/_rels/footnotes.xml.rels")
        .map(|xml| {
            let xml = std::str::from_utf8(xml).expect("footnotes.xml.rels is not utf-8");
            load_relationships(&parse_xml(xml))
        })
        .unwrap_or_default();
    let footnote_targets = footnote_rels
        .iter()
        .map(|rel| (rel.id.clone(), rel.target.clone()))
        .collect::<BTreeMap<_, _>>();
    let footnotes = files
        .get("word/footnotes.xml")
        .map(|xml| {
            let xml = std::str::from_utf8(xml).expect("word/footnotes.xml is not utf-8");
            project_footnotes(&parse_xml(xml), &numbering, &footnote_targets)
        })
        .unwrap_or_default();
    let media = files
        .iter()
        .filter(|(name, _)| name.starts_with("word/media/") && !name.ends_with('/'))
        .map(|(name, bytes)| MediaFile {
            path: name.clone(),
            bytes: bytes.clone(),
        })
        .collect::<Vec<_>>();
    let media_parts = media.iter().map(|file| file.path.clone()).collect();
    let stories = project_stories(&files, &numbering);
    let update_fields = files
        .get("word/settings.xml")
        .map(|xml| {
            let xml = std::str::from_utf8(xml).expect("word/settings.xml is not utf-8");
            settings_update_fields(&parse_xml(xml))
        })
        .unwrap_or(false);
    Document {
        blocks: project_container(body, &numbering, &rels),
        drawing_count: count_named(&document, "drawing"),
        footnotes,
        footnote_rels,
        media_parts,
        media,
        sections: project_sections(body),
        stories,
        update_fields,
        parts: files
            .iter()
            .map(|(name, bytes)| {
                let text = std::str::from_utf8(bytes).unwrap_or("").to_string();
                (name.clone(), text)
            })
            .collect(),
    }
}

#[allow(dead_code)] // Snapshot rendering is optional for each parity suite.
fn render_paragraph(paragraph: &Paragraph, indent: &str) -> String {
    let mut head = String::new();
    if let Some(heading) = &paragraph.heading {
        head.push_str(" style=");
        head.push_str(heading);
    } else if let Some(style_id) = &paragraph.style_id {
        head.push_str(" style=");
        head.push_str(style_id);
    }
    if let Some(list) = &paragraph.list {
        let kind = if list.ordered { "decimal" } else { "bullet" };
        head.push_str(&format!(
            " list={kind} start={} level={} id={}",
            list.start, list.level, list.num_id
        ));
    }
    if let Some(align) = &paragraph.align {
        head.push_str(" align=");
        head.push_str(align);
    }
    if paragraph.rule {
        head.push_str(" rule");
    }
    if paragraph.page_break {
        head.push_str(" page-break");
    }
    if !paragraph.bookmarks.is_empty() {
        head.push_str(" bookmarks=");
        head.push_str(&paragraph.bookmarks.join(","));
    }
    if !paragraph.footnote_refs.is_empty() {
        head.push_str(" footnotes=");
        head.push_str(&paragraph.footnote_refs.join(","));
    }
    if !paragraph.fields.is_empty() {
        head.push_str(" fields=");
        head.push_str(&paragraph.fields.join(" || "));
    }
    if !paragraph.field_chars.is_empty() {
        head.push_str(" fldChar=");
        head.push_str(&paragraph.field_chars.join(","));
    }
    if let Some(shade) = &paragraph.shade {
        head.push_str(" shade=");
        head.push_str(shade);
    }
    if let Some(color) = &paragraph.left_border_color {
        head.push_str(" border=");
        head.push_str(color);
        if let Some(size) = &paragraph.left_border_size {
            head.push(' ');
            head.push_str(size);
        }
    }
    for drawing in &paragraph.drawings {
        head.push_str(&format!(
            " drawing name={} descr={} title={} {}x{}",
            drawing.name.as_deref().unwrap_or("-"),
            drawing.descr.as_deref().unwrap_or("-"),
            drawing.title.as_deref().unwrap_or("-"),
            drawing.cx.unwrap_or(0),
            drawing.cy.unwrap_or(0)
        ));
    }
    for math in &paragraph.math {
        head.push_str(" math ");
        head.push_str(&math.tags.join(","));
        head.push(':');
        head.push_str(&math.text);
    }
    if paragraph.spacing_before.is_some() || paragraph.spacing_after.is_some() {
        head.push_str(&format!(
            " spacing={}/{}",
            paragraph.spacing_before.as_deref().unwrap_or("-"),
            paragraph.spacing_after.as_deref().unwrap_or("-")
        ));
    }
    if !paragraph.tabs.is_empty() {
        head.push_str(" tabs=");
        head.push_str(
            &paragraph
                .tabs
                .iter()
                .map(|tab| {
                    format!(
                        "{}:{}:{}",
                        tab.val,
                        tab.pos,
                        tab.leader.as_deref().unwrap_or("-")
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    let mut lines = vec![format!("{indent}P{head} |{}|", paragraph.text)];
    for run in &paragraph.runs {
        if run.text.trim().is_empty() && run.link.is_none() && run.style_id.is_none() {
            continue;
        }
        let mut flags = String::new();
        if let Some(style_id) = &run.style_id {
            flags.push_str(" rStyle=");
            flags.push_str(style_id);
        }
        if run.bold {
            flags.push_str(" b");
        }
        if run.italic {
            flags.push_str(" i");
        }
        if run.underline {
            flags.push_str(" u");
        }
        if run.strike {
            flags.push_str(" s");
        }
        if run.code {
            flags.push_str(" code");
        }
        if let Some(link) = &run.link {
            flags.push_str(" -> ");
            flags.push_str(link);
        }
        if let Some(anchor) = &run.anchor {
            flags.push_str(" anchor=");
            flags.push_str(anchor);
        }
        if let Some(font) = &run.font {
            flags.push_str(" font=");
            flags.push_str(font);
        }
        if let Some(fill) = &run.fill {
            flags.push_str(" fill=");
            flags.push_str(fill);
        }
        if let Some(size) = run.size {
            flags.push_str(&format!(" sz={size}"));
        }
        if let Some(color) = &run.color {
            flags.push_str(" color=");
            flags.push_str(color);
        }
        if !flags.is_empty() {
            lines.push(format!("{indent}  R |{}|{flags}", run.text));
        }
    }
    lines.join("\n")
}

#[allow(dead_code)] // Snapshot rendering is optional for each parity suite.
fn render_table(table: &Table) -> String {
    render_table_at(table, "")
}

#[allow(dead_code)] // Snapshot rendering is optional for each parity suite.
fn render_table_at(table: &Table, indent: &str) -> String {
    let grid = if table.column_widths.is_empty() {
        "-".to_string()
    } else {
        table.column_widths.join(",")
    };
    let mut lines = vec![format!(
        "{indent}TABLE cols={} width={} type={} layout={} grid={grid} style={}",
        table.columns,
        table.width.as_deref().unwrap_or("-"),
        table.width_type.as_deref().unwrap_or("-"),
        table.layout.as_deref().unwrap_or("-"),
        table.style_id.as_deref().unwrap_or("-"),
    )];
    for row in &table.rows {
        let mut cells = Vec::new();
        for cell in row {
            let text = cell
                .paragraphs
                .iter()
                .map(|paragraph| paragraph.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let align = cell
                .paragraphs
                .first()
                .and_then(|paragraph| paragraph.align.as_deref())
                .unwrap_or("-");
            cells.push(format!(
                "{align}:{text} span={} vmerge={}",
                cell.grid_span
                    .map(|span| span.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                cell.v_merge.as_deref().unwrap_or("-"),
            ));
        }
        lines.push(format!("{indent}  | {} |", cells.join(" | ")));
        for cell in row {
            for nested in &cell.tables {
                lines.push(render_table_at(nested, &format!("{indent}  ")));
            }
        }
    }
    lines.join("\n")
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(UNZIP_PY)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 is required to unzip docx fixtures");
    child
        .stdin
        .take()
        .expect("python stdin")
        .write_all(bytes)
        .expect("write docx bytes to python");
    let output = child.wait_with_output().expect("wait for python unzip");
    if !output.status.success() {
        panic!(
            "unzip failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut reader = BufReader::new(output.stdout.as_slice());
    let count = read_line(&mut reader)
        .parse::<usize>()
        .expect("zip entry count");
    let mut files = BTreeMap::new();
    for _ in 0..count {
        let name = normalize_zip_name(&read_line(&mut reader));
        let len = read_line(&mut reader)
            .parse::<usize>()
            .expect("zip entry length");
        let mut data = vec![0_u8; len];
        reader.read_exact(&mut data).expect("zip entry bytes");
        files.insert(name, data);
    }
    files
}

fn read_line(reader: &mut impl BufRead) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read unzip record");
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    line
}

fn normalize_zip_name(name: &str) -> String {
    let name = name.replace('\\', "/");
    name.trim_start_matches('/').to_string()
}

fn require_part<'a>(files: &'a BTreeMap<String, Vec<u8>>, name: &str) -> &'a str {
    let bytes = files.get(name).unwrap_or_else(|| {
        let names = files.keys().cloned().collect::<Vec<_>>().join(", ");
        panic!("missing {name} in docx ({names})");
    });
    std::str::from_utf8(bytes).unwrap_or_else(|err| panic!("{name} is not utf-8: {err}"))
}

#[derive(Clone, Debug, Default)]
struct LevelDef {
    start: u32,
    format: String,
}

fn load_numbering(root: &Elem) -> BTreeMap<String, BTreeMap<u32, LevelDef>> {
    let mut abstracts: BTreeMap<String, BTreeMap<u32, LevelDef>> = BTreeMap::new();
    for abstract_num in children(root, "abstractNum") {
        let Some(id) = attr(abstract_num, "abstractNumId") else {
            continue;
        };
        let mut levels = BTreeMap::new();
        for level in children(abstract_num, "lvl") {
            let Some(ilvl) = attr(level, "ilvl").and_then(|value| value.parse().ok()) else {
                continue;
            };
            let start = child(level, "start")
                .and_then(|start| attr(start, "val"))
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let format = child(level, "numFmt")
                .and_then(|fmt| attr(fmt, "val"))
                .unwrap_or("decimal")
                .to_string();
            levels.insert(ilvl, LevelDef { start, format });
        }
        abstracts.insert(id.to_string(), levels);
    }

    let mut nums = BTreeMap::new();
    for num in children(root, "num") {
        let Some(num_id) = attr(num, "numId") else {
            continue;
        };
        let abstract_id = child(num, "abstractNumId").and_then(|node| attr(node, "val"));
        let mut levels = abstract_id
            .and_then(|id| abstracts.get(id))
            .cloned()
            .unwrap_or_default();
        for override_level in children(num, "lvlOverride") {
            let Some(ilvl) = attr(override_level, "ilvl").and_then(|value| value.parse().ok())
            else {
                continue;
            };
            let Some(start) = child(override_level, "startOverride")
                .and_then(|start| attr(start, "val"))
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            levels.entry(ilvl).or_default().start = start;
        }
        nums.insert(num_id.to_string(), levels);
    }
    nums
}

fn load_rels(root: &Elem) -> BTreeMap<String, String> {
    load_relationships(root)
        .into_iter()
        .map(|rel| (rel.id, rel.target))
        .collect()
}

fn load_relationships(root: &Elem) -> Vec<Relationship> {
    children(root, "Relationship")
        .into_iter()
        .filter_map(|rel| {
            Some(Relationship {
                id: attr(rel, "Id")?.to_string(),
                rel_type: attr(rel, "Type").unwrap_or("").to_string(),
                target: attr(rel, "Target").unwrap_or("").to_string(),
            })
        })
        .collect()
}

fn project_container(
    container: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
    rels: &BTreeMap<String, String>,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for node in elem_children(container) {
        match local(&node.name) {
            "p" => blocks.push(Block::Paragraph(project_paragraph(node, numbering, rels))),
            "tbl" => blocks.push(Block::Table(project_table(node, numbering, rels))),
            "sdt" => {
                if let Some(content) = child(node, "sdtContent") {
                    blocks.extend(project_container(content, numbering, rels));
                }
            }
            _ => {}
        }
    }
    blocks
}

fn project_paragraph(
    paragraph: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
    rels: &BTreeMap<String, String>,
) -> Paragraph {
    let props = child(paragraph, "pPr");
    let heading = props.and_then(heading_style);
    let style_id = props
        .and_then(|props| child(props, "pStyle"))
        .and_then(|style| attr(style, "val"))
        .map(str::to_string);
    let align = props
        .and_then(|props| child(props, "jc"))
        .and_then(|jc| attr(jc, "val"))
        .map(str::to_string);
    let rule = props.map(is_thematic_rule).unwrap_or(false);
    let list = props.and_then(|props| list_info(props, numbering));
    let shade = props
        .and_then(|props| child(props, "shd"))
        .and_then(|shading| attr(shading, "fill"))
        .map(str::to_string);
    let left_border = props
        .and_then(|props| child(props, "pBdr"))
        .and_then(|border| child(border, "left"));
    let left_border_color = left_border
        .and_then(|border| attr(border, "color"))
        .map(str::to_string);
    let left_border_size = left_border
        .and_then(|border| attr(border, "sz"))
        .map(str::to_string);
    let spacing = props.and_then(|props| child(props, "spacing"));
    let spacing_before = spacing
        .and_then(|node| attr(node, "before"))
        .map(str::to_string);
    let spacing_after = spacing
        .and_then(|node| attr(node, "after"))
        .map(str::to_string);
    let tabs = props.map(tab_stops).unwrap_or_default();
    let runs = paragraph_runs(paragraph, rels);
    let text = runs.iter().map(|run| run.text.as_str()).collect::<String>();
    let marks = paragraph_marks(paragraph);
    Paragraph {
        text,
        heading,
        style_id,
        align,
        rule,
        list,
        runs,
        page_break: marks.page_break,
        bookmarks: marks.bookmarks,
        footnote_refs: marks.footnote_refs,
        fields: marks.fields,
        field_chars: marks.field_chars,
        shade,
        left_border_color,
        left_border_size,
        tabs,
        drawings: collect_drawings(paragraph),
        math: collect_math(paragraph),
        spacing_before,
        spacing_after,
    }
}

fn collect_math(paragraph: &Elem) -> Vec<MathExpr> {
    let mut math = Vec::new();
    collect_math_nodes(paragraph, &mut math);
    math
}

fn collect_math_nodes(element: &Elem, math: &mut Vec<MathExpr>) {
    if local(&element.name) == "oMath" {
        math.push(project_math(element));
        return;
    }
    for child_node in elem_children(element) {
        collect_math_nodes(child_node, math);
    }
}

fn project_math(math: &Elem) -> MathExpr {
    let mut tags = Vec::new();
    let mut text = String::new();
    collect_math_parts(math, &mut tags, &mut text);
    MathExpr { tags, text }
}

fn collect_math_parts(element: &Elem, tags: &mut Vec<String>, text: &mut String) {
    let name = local(&element.name);
    if matches!(name, "sSup" | "sSub" | "sSubSup" | "f" | "rad" | "t") {
        tags.push(name.to_string());
    }
    if name == "t" {
        text.push_str(&direct_text(element));
    }
    for child_node in elem_children(element) {
        collect_math_parts(child_node, tags, text);
    }
}

fn collect_drawings(paragraph: &Elem) -> Vec<Drawing> {
    let mut drawings = Vec::new();
    collect_drawing_nodes(paragraph, &mut drawings);
    drawings
}

fn collect_drawing_nodes(element: &Elem, drawings: &mut Vec<Drawing>) {
    if local(&element.name) == "drawing" {
        drawings.push(project_drawing(element));
        return;
    }
    for child_node in elem_children(element) {
        collect_drawing_nodes(child_node, drawings);
    }
}

fn project_drawing(drawing: &Elem) -> Drawing {
    let doc_pr = find_first(drawing, "docPr");
    let extent = find_first(drawing, "extent");
    Drawing {
        name: doc_pr
            .and_then(|node| attr(node, "name"))
            .map(str::to_string),
        descr: doc_pr
            .and_then(|node| attr(node, "descr"))
            .map(str::to_string),
        title: doc_pr
            .and_then(|node| attr(node, "title"))
            .map(str::to_string),
        cx: extent
            .and_then(|node| attr(node, "cx"))
            .and_then(|value| value.parse().ok()),
        cy: extent
            .and_then(|node| attr(node, "cy"))
            .and_then(|value| value.parse().ok()),
    }
}

struct ParagraphMarks {
    page_break: bool,
    bookmarks: Vec<String>,
    footnote_refs: Vec<String>,
    fields: Vec<String>,
    field_chars: Vec<String>,
}

fn paragraph_marks(paragraph: &Elem) -> ParagraphMarks {
    let mut marks = ParagraphMarks {
        page_break: false,
        bookmarks: Vec::new(),
        footnote_refs: Vec::new(),
        fields: Vec::new(),
        field_chars: Vec::new(),
    };
    collect_marks(paragraph, &mut marks);
    marks
}

fn collect_marks(element: &Elem, marks: &mut ParagraphMarks) {
    match local(&element.name) {
        "bookmarkStart" => {
            if let Some(name) = attr(element, "name") {
                marks.bookmarks.push(name.to_string());
            }
        }
        "footnoteReference" => {
            if let Some(id) = attr(element, "id") {
                marks.footnote_refs.push(id.to_string());
            }
        }
        "fldSimple" => {
            if let Some(instr) = attr(element, "instr") {
                marks.fields.push(instr.to_string());
            }
        }
        "instrText" => {
            let text = direct_text(element);
            if !text.is_empty() {
                marks.fields.push(text);
            }
        }
        "fldChar" => {
            if let Some(kind) = attr(element, "fldCharType") {
                marks.field_chars.push(kind.to_string());
            }
        }
        "br" if attr(element, "type") == Some("page") => marks.page_break = true,
        _ => {}
    }
    for child_node in elem_children(element) {
        collect_marks(child_node, marks);
    }
}

fn project_footnotes(
    root: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
    rels: &BTreeMap<String, String>,
) -> Vec<Footnote> {
    children(root, "footnote")
        .into_iter()
        .filter(|note| attr(note, "type").is_none())
        .map(|note| Footnote {
            id: attr(note, "id").unwrap_or("").to_string(),
            paragraphs: children(note, "p")
                .into_iter()
                .map(|paragraph| project_paragraph(paragraph, numbering, rels))
                .collect(),
            drawing_count: count_named(note, "drawing"),
            embeds: collect_attr_values(note, "embed"),
        })
        .collect()
}

fn collect_attr_values(element: &Elem, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(value) = attr(element, name) {
        values.push(value.to_string());
    }
    for child_node in elem_children(element) {
        values.extend(collect_attr_values(child_node, name));
    }
    values
}

fn heading_style(props: &Elem) -> Option<String> {
    children(props, "pStyle").into_iter().find_map(|style| {
        let value = attr(style, "val")?;
        if matches!(
            value,
            "Heading1" | "Heading2" | "Heading3" | "Heading4" | "Heading5" | "Heading6"
        ) {
            Some(value.to_string())
        } else {
            None
        }
    })
}

fn is_thematic_rule(props: &Elem) -> bool {
    let Some(bottom) = child(props, "pBdr").and_then(|border| child(border, "bottom")) else {
        return false;
    };
    attr(bottom, "color") == Some("A6A6A6") && attr(bottom, "sz") == Some("6")
}

fn list_info(
    props: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
) -> Option<ListInfo> {
    let num_pr = child(props, "numPr")?;
    let level = child(num_pr, "ilvl")
        .and_then(|ilvl| attr(ilvl, "val"))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let num_id = child(num_pr, "numId")
        .and_then(|num_id| attr(num_id, "val"))?
        .to_string();
    let defined = numbering.get(&num_id).and_then(|levels| levels.get(&level));
    let start = defined.map(|level| level.start).unwrap_or(1);
    let ordered = defined
        .map(|level| !level.format.eq_ignore_ascii_case("bullet"))
        .unwrap_or(true);
    Some(ListInfo {
        num_id,
        level,
        start,
        ordered,
    })
}

fn paragraph_runs(paragraph: &Elem, rels: &BTreeMap<String, String>) -> Vec<Run> {
    let mut runs = Vec::new();
    collect_runs(paragraph, rels, None, None, &mut runs);
    runs
}

fn collect_runs(
    node: &Elem,
    rels: &BTreeMap<String, String>,
    link: Option<String>,
    anchor: Option<String>,
    runs: &mut Vec<Run>,
) {
    for child_node in elem_children(node) {
        match local(&child_node.name) {
            "pPr" | "rPr" | "oMath" | "oMathPara" => {}
            "r" => runs.push(project_run(child_node, link.clone(), anchor.clone())),
            "hyperlink" => {
                let target = attr(child_node, "id")
                    .and_then(|id| rels.get(id))
                    .cloned()
                    .or_else(|| link.clone());
                let next_anchor = attr(child_node, "anchor")
                    .map(str::to_string)
                    .or_else(|| anchor.clone());
                collect_runs(child_node, rels, target, next_anchor, runs);
            }
            _ => collect_runs(child_node, rels, link.clone(), anchor.clone(), runs),
        }
    }
}

fn project_run(run: &Elem, link: Option<String>, anchor: Option<String>) -> Run {
    let props = child(run, "rPr");
    let ascii = props
        .and_then(|props| child(props, "rFonts"))
        .and_then(|fonts| attr(fonts, "ascii").or_else(|| attr(fonts, "hAnsi")));
    let fill = props
        .and_then(|props| child(props, "shd"))
        .and_then(|shading| attr(shading, "fill"));
    let code = ascii == Some("Courier New")
        || fill
            .map(|fill| fill.eq_ignore_ascii_case("F5F5F5"))
            .unwrap_or(false);
    Run {
        text: run_text(run),
        bold: marked(props, "b"),
        italic: marked(props, "i"),
        underline: marked(props, "u"),
        strike: marked(props, "strike") || marked(props, "dstrike"),
        code,
        link,
        anchor,
        size: props
            .and_then(|props| child(props, "sz"))
            .and_then(|size| attr(size, "val"))
            .and_then(|value| value.parse().ok()),
        color: props
            .and_then(|props| child(props, "color"))
            .and_then(|color| attr(color, "val"))
            .map(str::to_string),
        font: ascii.map(str::to_string),
        fill: fill.map(str::to_string),
        style_id: props
            .and_then(|props| child(props, "rStyle"))
            .and_then(|style| attr(style, "val"))
            .map(str::to_string),
    }
}

fn marked(props: Option<&Elem>, name: &str) -> bool {
    let Some(props) = props else {
        return false;
    };
    let Some(node) = child(props, name) else {
        return false;
    };
    !matches!(attr(node, "val"), Some("0" | "false" | "off" | "none"))
}

fn run_text(run: &Elem) -> String {
    let mut text = String::new();
    let mut breaks = 0_u32;
    for node in elem_children(run) {
        match local(&node.name) {
            "t" => text.push_str(&direct_text(node)),
            "br" if attr(node, "type") == Some("page") => {}
            "br" | "cr" => breaks += 1,
            "tab" => text.push('\t'),
            _ => {}
        }
    }
    if breaks > 0 {
        while text.ends_with('\n') {
            text.pop();
        }
        for _ in 0..breaks {
            text.push('\n');
        }
    }
    text
}

fn project_table(
    table: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
    rels: &BTreeMap<String, String>,
) -> Table {
    let tbl_pr = child(table, "tblPr");
    let tbl_w = tbl_pr.and_then(|props| child(props, "tblW"));
    let layout = tbl_pr
        .and_then(|props| child(props, "tblLayout"))
        .and_then(|layout| attr(layout, "type"))
        .map(str::to_string);
    let column_widths = child(table, "tblGrid")
        .map(|grid| {
            children(grid, "gridCol")
                .into_iter()
                .filter_map(|column| attr(column, "w").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let rows = children(table, "tr")
        .into_iter()
        .map(|row| {
            children(row, "tc")
                .into_iter()
                .map(|cell| project_cell(cell, numbering, rels))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    Table {
        columns,
        width: tbl_w.and_then(|width| attr(width, "w")).map(str::to_string),
        width_type: tbl_w
            .and_then(|width| attr(width, "type"))
            .map(str::to_string),
        layout,
        column_widths,
        style_id: tbl_pr
            .and_then(|props| child(props, "tblStyle"))
            .and_then(|style| attr(style, "val"))
            .map(str::to_string),
        rows,
    }
}

fn project_cell(
    cell: &Elem,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
    rels: &BTreeMap<String, String>,
) -> Cell {
    let props = child(cell, "tcPr");
    let grid_span = props
        .and_then(|props| child(props, "gridSpan"))
        .and_then(|span| attr_u32(span, "val"));
    let v_merge = props
        .and_then(|props| child(props, "vMerge"))
        .map(|merge| attr(merge, "val").unwrap_or("continue").to_string());
    Cell {
        paragraphs: children(cell, "p")
            .into_iter()
            .map(|paragraph| project_paragraph(paragraph, numbering, rels))
            .collect(),
        tables: children(cell, "tbl")
            .into_iter()
            .map(|nested| project_table(nested, numbering, rels))
            .collect(),
        grid_span,
        v_merge,
    }
}

fn tab_stops(props: &Elem) -> Vec<TabStop> {
    let Some(tabs) = child(props, "tabs") else {
        return Vec::new();
    };
    children(tabs, "tab")
        .into_iter()
        .filter_map(|tab| {
            Some(TabStop {
                val: attr(tab, "val")?.to_string(),
                pos: attr(tab, "pos")?.to_string(),
                leader: attr(tab, "leader").map(str::to_string),
            })
        })
        .collect()
}

fn project_sections(body: &Elem) -> Vec<SectionProps> {
    let mut sections = Vec::new();
    for node in elem_children(body) {
        match local(&node.name) {
            "sectPr" => sections.push(project_sect_pr(node)),
            "p" => {
                if let Some(sect) = child(node, "pPr").and_then(|props| child(props, "sectPr")) {
                    sections.push(project_sect_pr(sect));
                }
            }
            _ => {}
        }
    }
    sections
}

fn project_sect_pr(sect: &Elem) -> SectionProps {
    let page = child(sect, "pgSz");
    let margin = child(sect, "pgMar");
    let numbering = child(sect, "pgNumType");
    let title = child(sect, "titlePg");
    SectionProps {
        break_type: child(sect, "type").and_then(|kind| attr(kind, "val").map(str::to_string)),
        page_width: page.and_then(|page| attr_u32(page, "w")),
        page_height: page.and_then(|page| attr_u32(page, "h")),
        orientation: page.and_then(|page| attr(page, "orient").map(str::to_string)),
        margin_top: margin.and_then(|margin| attr_u32(margin, "top")),
        margin_right: margin.and_then(|margin| attr_u32(margin, "right")),
        margin_bottom: margin.and_then(|margin| attr_u32(margin, "bottom")),
        margin_left: margin.and_then(|margin| attr_u32(margin, "left")),
        margin_header: margin.and_then(|margin| attr_u32(margin, "header")),
        margin_footer: margin.and_then(|margin| attr_u32(margin, "footer")),
        gutter: margin.and_then(|margin| attr_u32(margin, "gutter")),
        title_page: title
            .is_some_and(|node| !matches!(attr(node, "val"), Some("0" | "false" | "off" | "none"))),
        page_start: numbering.and_then(|numbering| attr_u32(numbering, "start")),
        page_format: numbering.and_then(|numbering| attr(numbering, "fmt").map(str::to_string)),
        page_separator: numbering
            .and_then(|numbering| attr(numbering, "chapSep").map(str::to_string)),
        header_refs: story_refs(sect, "headerReference"),
        footer_refs: story_refs(sect, "footerReference"),
    }
}

fn story_refs(sect: &Elem, name: &str) -> Vec<StoryRef> {
    children(sect, name)
        .into_iter()
        .filter_map(|reference| {
            Some(StoryRef {
                kind: attr(reference, "type")?.to_string(),
                id: attr(reference, "id")?.to_string(),
            })
        })
        .collect()
}

fn attr_u32(element: &Elem, name: &str) -> Option<u32> {
    attr(element, name)?.parse().ok()
}

fn project_stories(
    files: &BTreeMap<String, Vec<u8>>,
    numbering: &BTreeMap<String, BTreeMap<u32, LevelDef>>,
) -> Vec<Story> {
    files
        .iter()
        .filter(|(name, _)| is_story_part(name))
        .map(|(name, bytes)| {
            let xml = std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{name} is not utf-8"));
            let root = parse_xml(xml);
            let rels_path = story_rels_path(name);
            let rels = files
                .get(&rels_path)
                .map(|xml| {
                    let xml = std::str::from_utf8(xml)
                        .unwrap_or_else(|_| panic!("{rels_path} is not utf-8"));
                    load_rels(&parse_xml(xml))
                })
                .unwrap_or_default();
            Story {
                path: name.clone(),
                blocks: project_container(&root, numbering, &rels),
                drawing_count: count_named(&root, "drawing"),
            }
        })
        .collect()
}

fn is_story_part(name: &str) -> bool {
    if !name.starts_with("word/") || !name.ends_with(".xml") {
        return false;
    }
    let file = name.rsplit('/').next().unwrap_or(name);
    let stem = file.trim_end_matches(".xml");
    let kind = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    kind == "header" || kind == "footer"
}

fn story_rels_path(story: &str) -> String {
    let Some((dir, file)) = story.rsplit_once('/') else {
        return format!("_rels/{story}.rels");
    };
    format!("{dir}/_rels/{file}.rels")
}

fn settings_update_fields(root: &Elem) -> bool {
    let Some(node) = find_first(root, "updateFields") else {
        return false;
    };
    !matches!(attr(node, "val"), Some("0" | "false" | "off" | "none"))
}

fn direct_text(element: &Elem) -> String {
    element
        .children
        .iter()
        .filter_map(|node| match node {
            Node::Text(text) => Some(text.as_str()),
            Node::Elem(_) => None,
        })
        .collect()
}

fn count_named(element: &Elem, name: &str) -> usize {
    let mut count = 0;
    if local(&element.name) == name {
        count += 1;
    }
    for child_node in elem_children(element) {
        count += count_named(child_node, name);
    }
    count
}

fn find_first<'a>(element: &'a Elem, name: &str) -> Option<&'a Elem> {
    if local(&element.name) == name {
        return Some(element);
    }
    for child_node in elem_children(element) {
        if let Some(found) = find_first(child_node, name) {
            return Some(found);
        }
    }
    None
}

fn child<'a>(element: &'a Elem, name: &str) -> Option<&'a Elem> {
    elem_children(element).find(|child_node| local(&child_node.name) == name)
}

fn children<'a>(element: &'a Elem, name: &str) -> Vec<&'a Elem> {
    elem_children(element)
        .filter(|child_node| local(&child_node.name) == name)
        .collect()
}

fn elem_children(element: &Elem) -> impl Iterator<Item = &Elem> {
    element.children.iter().filter_map(|node| match node {
        Node::Elem(child_node) => Some(child_node),
        Node::Text(_) => None,
    })
}

fn attr<'a>(element: &'a Elem, name: &str) -> Option<&'a str> {
    element
        .attrs
        .iter()
        .find(|(key, _)| local(key) == name)
        .map(|(_, value)| value.as_str())
}

fn local(name: &str) -> &str {
    match name.rfind([':', '}']) {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

#[derive(Debug)]
enum Node {
    Elem(Elem),
    Text(String),
}

#[derive(Debug)]
struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

fn parse_xml(input: &str) -> Elem {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut parser = Parser { src: input, pos: 0 };
    parser.skip_misc();
    parser.parse_element()
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn skip_misc(&mut self) {
        loop {
            self.skip_ws();
            if self.rest().starts_with("<?") {
                self.skip_until("?>");
                continue;
            }
            if self.rest().starts_with("<!--") {
                self.skip_until("-->");
                continue;
            }
            if self.rest().starts_with("<!DOCTYPE") || self.rest().starts_with("<!doctype") {
                self.skip_until(">");
                continue;
            }
            break;
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.rest().chars().next(), Some(ch) if ch.is_whitespace()) {
            self.pos += self.rest().chars().next().unwrap().len_utf8();
        }
    }

    fn skip_until(&mut self, marker: &str) {
        if let Some(index) = self.rest().find(marker) {
            self.pos += index + marker.len();
        } else {
            self.pos = self.src.len();
        }
    }

    fn parse_element(&mut self) -> Elem {
        self.skip_ws();
        self.expect("<");
        if self.rest().starts_with("/") {
            panic!("unexpected end tag near {}", self.snippet());
        }
        let name = self.read_name();
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            if self.rest().starts_with("/>") {
                self.pos += 2;
                return Elem {
                    name,
                    attrs,
                    children: Vec::new(),
                };
            }
            if self.rest().starts_with('>') {
                self.pos += 1;
                break;
            }
            let key = self.read_name();
            self.skip_ws();
            self.expect("=");
            self.skip_ws();
            attrs.push((key, self.read_quoted()));
        }
        let mut children = Vec::new();
        loop {
            if self.rest().starts_with("</") {
                self.pos += 2;
                let end = self.read_name();
                self.skip_ws();
                self.expect(">");
                if local(&end) != local(&name) {
                    panic!(
                        "end tag {end} does not close {name} near {}",
                        self.snippet()
                    );
                }
                break;
            }
            if self.rest().starts_with("<![CDATA[") {
                self.pos += "<![CDATA[".len();
                let start = self.pos;
                self.skip_until("]]>");
                let end = self.pos - "]]>".len();
                children.push(Node::Text(self.src[start..end].to_string()));
                continue;
            }
            if self.rest().starts_with('<') {
                self.skip_misc_inside();
                if self.rest().starts_with("</") || self.rest().is_empty() {
                    continue;
                }
                if self.rest().starts_with('<') {
                    children.push(Node::Elem(self.parse_element()));
                }
                continue;
            }
            if self.rest().is_empty() {
                panic!("unclosed <{name}>");
            }
            let start = self.pos;
            let Some(index) = self.rest().find('<') else {
                children.push(Node::Text(decode_entities(self.rest())));
                self.pos = self.src.len();
                panic!("unclosed <{name}>");
            };
            self.pos += index;
            children.push(Node::Text(decode_entities(&self.src[start..self.pos])));
        }
        Elem {
            name,
            attrs,
            children,
        }
    }

    fn skip_misc_inside(&mut self) {
        if self.rest().starts_with("<?") || self.rest().starts_with("<!--") {
            self.skip_misc();
        }
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.rest().chars().next() {
            if ch.is_whitespace() || matches!(ch, '/' | '>' | '=') {
                break;
            }
            self.pos += ch.len_utf8();
        }
        if self.pos == start {
            panic!("expected a name near {}", self.snippet());
        }
        self.src[start..self.pos].to_string()
    }

    fn read_quoted(&mut self) -> String {
        let quote = self.rest().chars().next().unwrap_or('\0');
        if quote != '"' && quote != '\'' {
            panic!("expected a quoted attribute near {}", self.snippet());
        }
        self.pos += quote.len_utf8();
        let start = self.pos;
        while let Some(ch) = self.rest().chars().next() {
            self.pos += ch.len_utf8();
            if ch == quote {
                return decode_entities(&self.src[start..self.pos - quote.len_utf8()]);
            }
        }
        panic!("unclosed attribute near {}", self.snippet());
    }

    fn expect(&mut self, token: &str) {
        if !self.rest().starts_with(token) {
            panic!("expected {token} near {}", self.snippet());
        }
        self.pos += token.len();
    }

    fn snippet(&self) -> String {
        let end = (self.pos + 40).min(self.src.len());
        self.src[self.pos..end].replace('\n', " ")
    }
}

fn decode_entities(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let entity = &rest[1..end];
        rest = &rest[end + 1..];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            other if other.starts_with('#') => match decode_numeric(other) {
                Some(ch) => out.push(ch),
                None => {
                    out.push('&');
                    out.push_str(other);
                    out.push(';');
                }
            },
            other => {
                out.push('&');
                out.push_str(other);
                out.push(';');
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_numeric(entity: &str) -> Option<char> {
    let code = if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        entity.strip_prefix('#')?.parse().ok()?
    };
    char::from_u32(code)
}
