use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use pulldown_cmark::{Event as MdEvent, Options, Parser, Tag, TagEnd};
use quick_xml::escape::{escape, partial_escape};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::bookmarks::HeadingBookmarks;
use crate::error::MarkdownConversionError;
use crate::model::{
    Block, Blockquote, CalloutKind, Caption, CaptionKind, CodeBlock, CrossRef, Document,
    FootnoteMark, HeaderFooterBody, Heading, Image, Inline, List, ListItem, Paragraph,
    PreparedDocument, PreparedSection, ResolvedHeaderFooter, SectionBlock, Table, TableAlign,
    TableCell,
};
use crate::options::{
    normalize_document_metadata, run_language, Alignment, CaptionOptions, ConvertOptions,
    Direction, DocumentMetadata, Orientation, PageNumberDisplay, PageNumberFormat,
    PageNumberSeparator, ResolvedPageNumbering, ResolvedStyle, SectionBreak, TocMode, TocSettings,
};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MATH_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const PAGE_WIDTH: &str = "11906";
const PAGE_HEIGHT: &str = "16838";
const CONTENT_WIDTH: u32 = 9746;
const BULLETS: [&str; 9] = ["●", "○", "■", "●", "○", "■", "●", "●", "●"];
/// Marks an inline run whose text is a footnote image URL, not visible text.
const IMAGE_MARK: char = '\u{1}';

/// OOXML parts for one `.docx` package.
///
/// Slice 1 writes the six main parts. Footnote parts are added only when the
/// document has footnote definitions. `extras` holds parts such as
/// `word/media/...` that the footnote image repair has to find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    pub content_types: Vec<u8>,
    pub rels: Vec<u8>,
    pub document: Vec<u8>,
    pub document_rels: Vec<u8>,
    pub styles: Vec<u8>,
    pub numbering: Vec<u8>,
    pub footnotes: Option<Vec<u8>>,
    pub footnote_rels: Option<Vec<u8>>,
    pub settings: Option<Vec<u8>>,
    pub extras: Vec<(String, Vec<u8>)>,
    /// Set only when conversion options include `metadata`. `None` leaves the
    /// legacy core properties alone. A set value rewrites those parts.
    pub metadata: Option<PackageMetadata>,
}

impl Parts {
    /// Part name and bytes, in the order stored in the ZIP.
    pub fn files(&self) -> Vec<(&str, &[u8])> {
        let mut files = vec![
            ("[Content_Types].xml", self.content_types.as_slice()),
            ("_rels/.rels", self.rels.as_slice()),
            ("word/document.xml", self.document.as_slice()),
            (
                "word/_rels/document.xml.rels",
                self.document_rels.as_slice(),
            ),
            ("word/styles.xml", self.styles.as_slice()),
            ("word/numbering.xml", self.numbering.as_slice()),
        ];
        if let Some(footnotes) = &self.footnotes {
            files.push(("word/footnotes.xml", footnotes.as_slice()));
        }
        if let Some(rels) = &self.footnote_rels {
            files.push(("word/_rels/footnotes.xml.rels", rels.as_slice()));
        }
        if let Some(settings) = &self.settings {
            files.push(("word/settings.xml", settings.as_slice()));
        }
        for (name, bytes) in &self.extras {
            files.push((name.as_str(), bytes.as_slice()));
        }
        files
    }
}

/// Turn a document model into WordprocessingML parts.
pub fn render(doc: &Document) -> Result<Parts, MarkdownConversionError> {
    let footnotes = gather_footnotes(&doc.children);
    let mut state = State::default();
    state.content_width = CONTENT_WIDTH;
    state.toc_native = true;
    state.toc_page_numbers = true;
    state.toc_dot_leaders = true;
    state.toc_update_on_open = true;
    state.toc_title = "Table of Contents".to_string();
    state.toc_min_depth = 1;
    state.toc_max_depth = 6;
    state.toc_paragraph_size = 24;
    state.footnote_order = footnotes.iter().map(|(id, _)| id.clone()).collect();
    let document = state.write_document(doc)?;
    let (footnotes_xml, footnote_rels) = if footnotes.is_empty() {
        (None, None)
    } else {
        (
            Some(state.write_footnotes(&footnotes)?),
            Some(write_footnote_rels(&state.footnote_links)?),
        )
    };
    let media_types = state.media_types.clone();
    let has_footnotes = footnotes_xml.is_some();
    Ok(Parts {
        content_types: write_content_types(has_footnotes, &media_types, &state.story_parts, true)?,
        rels: write_package_rels()?,
        document,
        document_rels: write_document_rels(&state.links, has_footnotes, &state.story_rels)?,
        styles: write_styles(None)?,
        numbering: write_numbering(&state.ordered_starts)?,
        footnotes: footnotes_xml,
        footnote_rels,
        settings: Some(write_settings(state.update_fields)?),
        extras: {
            let mut extras = state.media;
            extras.extend(
                state
                    .story_parts
                    .iter()
                    .map(|part| (part.path.clone(), part.bytes.clone())),
            );
            push_legacy_core(&mut extras)?;
            extras
        },
        metadata: None,
    })
}

/// Render a document after `parse::prepare` has resolved styles, sections, and TOC.
pub fn render_prepared(
    doc: &PreparedDocument,
    markdown: &str,
    options: &ConvertOptions,
) -> Result<Parts, MarkdownConversionError> {
    let mut doc = doc.clone();
    plant_prepared_images(&mut doc, markdown, options);
    let blocks = prepared_body_blocks(&doc);
    let footnotes = gather_footnotes(&blocks);
    let mut state = State::default();
    state.content_width = CONTENT_WIDTH;
    state.toc_native = doc.toc.mode != TocMode::Links;
    state.toc_page_numbers = doc.toc.page_numbers;
    state.toc_dot_leaders = doc.toc.dot_leaders;
    state.toc_update_on_open = doc.toc.update_fields_on_open;
    state.toc_title = doc.toc.title.clone();
    state.toc_min_depth = doc.toc.min_depth;
    state.toc_max_depth = doc.toc.max_depth;
    state.toc_paragraph_size = 24;
    state.update_fields = doc.update_fields;
    state.caption_options = options.captions.clone();
    state.footnote_order = footnotes.iter().map(|(id, _)| id.clone()).collect();
    let document = state.write_prepared(&doc)?;
    let (footnotes_xml, footnote_rels) = if footnotes.is_empty() {
        (None, None)
    } else {
        (
            Some(state.write_footnotes(&footnotes)?),
            Some(write_footnote_rels(&state.footnote_links)?),
        )
    };
    let media_types = state.media_types.clone();
    let has_footnotes = footnotes_xml.is_some();
    let document_style = doc.sections.first().map(|section| section.style.clone());
    Ok(Parts {
        content_types: write_content_types(has_footnotes, &media_types, &state.story_parts, true)?,
        rels: write_package_rels()?,
        document,
        document_rels: write_document_rels(&state.links, has_footnotes, &state.story_rels)?,
        styles: write_styles(document_style.as_ref())?,
        numbering: write_numbering(&state.ordered_starts)?,
        footnotes: footnotes_xml,
        footnote_rels,
        settings: Some(write_settings(state.update_fields)?),
        extras: {
            let mut extras = state.media;
            extras.extend(
                state
                    .story_parts
                    .iter()
                    .map(|part| (part.path.clone(), part.bytes.clone())),
            );
            push_legacy_core(&mut extras)?;
            extras
        },
        metadata: package_metadata(options)?,
    })
}

/// Put footnote images back onto the model.
///
/// The parser drops `Tag::Image`. A footnote whose body is only an image would
/// otherwise be an empty paragraph, and `package::pack` would have nothing to
/// repair into an `mdImageN` relationship. The run text is `\u{1}` plus the
/// image URL. `render` turns that into a drawing and a `word/media` part.
pub fn plant_footnote_images(doc: &mut Document, markdown: &str) {
    let groups = footnote_image_groups(markdown);
    let mut groups = groups.into_iter();
    for child in &mut doc.children {
        let Block::FootnoteDefinition { children, .. } = child else {
            continue;
        };
        let Some(urls) = groups.next() else {
            break;
        };
        insert_footnote_images(children, &urls);
    }
    stamp_image_labels(&mut doc.children, markdown);
}

#[derive(Default)]
struct State {
    /// Ordered lists as `(sequence_id, start)`. Bullets stay on numId 1.
    ordered_starts: Vec<(u32, u64)>,
    links: Vec<String>,
    footnote_links: Vec<String>,
    /// Definition ids in document order. XML ids are 1-based positions.
    footnote_order: Vec<String>,
    bookmark_id: u32,
    in_footnote: bool,
    pending_footnote_ref: bool,
    media: Vec<(String, Vec<u8>)>,
    media_types: Vec<(&'static str, &'static str)>,
    /// Usable page width in DXA twips. Default A4 portrait is 9746.
    content_width: u32,
    toc_native: bool,
    toc_page_numbers: bool,
    toc_dot_leaders: bool,
    toc_update_on_open: bool,
    toc_title: String,
    toc_min_depth: u8,
    toc_max_depth: u8,
    toc_paragraph_size: u32,
    headings: Vec<TocHeading>,
    toc_placed: bool,
    update_fields: bool,
    in_header_footer: bool,
    story_parts: Vec<StoryPart>,
    story_rels: Vec<StoryRel>,
    story_links: Vec<String>,
    active_style: Option<ResolvedStyle>,
    /// Slot alignment, used while a header or footer is written.
    paragraph_align_override: Option<Alignment>,
    header_count: u32,
    footer_count: u32,
    /// GitHub slug to the heading bookmark the parser already stored.
    anchors: HeadingBookmarks,
    /// Caption bookmark to its 1-based SEQ number. Figure and table counts are separate.
    caption_numbers: BTreeMap<String, u32>,
    figure_seq: u32,
    table_seq: u32,
    /// Labels, alignment, italic, and size from `ConvertOptions.captions`.
    caption_options: Option<CaptionOptions>,
    /// `wp:docPr` ids. The first drawing is 1.
    drawing_id: u32,
}

#[derive(Clone)]
struct TocHeading {
    text: String,
    level: u8,
    bookmark: String,
}

struct StoryPart {
    path: String,
    bytes: Vec<u8>,
    content_type: Option<&'static str>,
}

struct StoryRel {
    id: String,
    rel_type: &'static str,
    target: String,
    /// `default`, `first`, or `even` for a header or footer reference.
    reference: Option<&'static str>,
}

struct DrawnSection {
    body: Vec<u8>,
    rel_start: usize,
    rel_end: usize,
    page_width: u32,
    page_height: u32,
    orientation: Orientation,
    margin_top: u32,
    margin_right: u32,
    margin_bottom: u32,
    margin_left: u32,
    margin_header: u32,
    margin_footer: u32,
    margin_gutter: u32,
    numbering: Option<ResolvedPageNumbering>,
    title_page: Option<bool>,
    break_type: Option<SectionBreak>,
    last: bool,
}

struct QuoteCtx {
    level: u32,
    callout: Option<CalloutKind>,
    /// Direct paragraphs of a plain quote are italic. CalloutKind paragraphs are not.
    italic_paragraphs: bool,
}

struct Look {
    size: Option<u32>,
    force_bold: bool,
    force_italic: bool,
    color: Option<&'static str>,
}

impl Look {
    fn body() -> Self {
        Self {
            size: None,
            force_bold: false,
            force_italic: false,
            color: None,
        }
    }

    fn sized(size: u32) -> Self {
        Self {
            size: Some(size),
            force_bold: false,
            force_italic: false,
            color: None,
        }
    }
}

struct Xml {
    writer: Writer<Cursor<Vec<u8>>>,
}

impl Xml {
    fn new() -> Result<Self, MarkdownConversionError> {
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        writer
            .write_event(Event::Decl(BytesDecl::new(
                "1.0",
                Some("UTF-8"),
                Some("yes"),
            )))
            .map_err(conv)?;
        Ok(Self { writer })
    }

    fn fragment() -> Self {
        Self {
            writer: Writer::new(Cursor::new(Vec::new())),
        }
    }

    fn finish(self) -> Vec<u8> {
        self.writer.into_inner().into_inner()
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        let cursor = self.writer.get_mut();
        cursor.get_mut().extend_from_slice(bytes);
        let end = u64::try_from(cursor.get_ref().len()).unwrap_or(u64::MAX);
        cursor.set_position(end);
    }

    fn start(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), MarkdownConversionError> {
        let mut elem = BytesStart::new(name);
        for (key, value) in attrs {
            elem.push_attribute((*key, *value));
        }
        self.writer.write_event(Event::Start(elem)).map_err(conv)
    }

    fn end(&mut self, name: &str) -> Result<(), MarkdownConversionError> {
        self.writer
            .write_event(Event::End(BytesEnd::new(name)))
            .map_err(conv)
    }

    fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), MarkdownConversionError> {
        let mut elem = BytesStart::new(name);
        for (key, value) in attrs {
            elem.push_attribute((*key, *value));
        }
        self.writer.write_event(Event::Empty(elem)).map_err(conv)
    }

    fn text(&mut self, raw: &str) -> Result<(), MarkdownConversionError> {
        let escaped = escape(raw);
        self.writer
            .write_event(Event::Text(BytesText::from_escaped(escaped)))
            .map_err(conv)
    }

    /// Element text. Quotes stay as characters, matching the metadata serializer.
    fn plain_text(&mut self, raw: &str) -> Result<(), MarkdownConversionError> {
        let escaped = partial_escape(raw);
        self.writer
            .write_event(Event::Text(BytesText::from_escaped(escaped)))
            .map_err(conv)
    }

    fn text_elem(&mut self, raw: &str) -> Result<(), MarkdownConversionError> {
        self.start("w:t", &[("xml:space", "preserve")])?;
        if !raw.is_empty() {
            self.text(raw)?;
        }
        self.end("w:t")
    }
}

fn conv(err: impl std::fmt::Display) -> MarkdownConversionError {
    MarkdownConversionError::new(err.to_string())
}

fn alignment_name(align: Alignment) -> &'static str {
    match align {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::Justified => "both",
    }
}

fn section_break_name(kind: SectionBreak) -> &'static str {
    match kind {
        SectionBreak::NextPage => "nextPage",
        SectionBreak::NextColumn => "nextColumn",
        SectionBreak::Continuous => "continuous",
        SectionBreak::EvenPage => "evenPage",
        SectionBreak::OddPage => "oddPage",
    }
}

fn page_format_name(format: PageNumberFormat) -> &'static str {
    match format {
        PageNumberFormat::Decimal => "decimal",
        PageNumberFormat::UpperRoman => "upperRoman",
        PageNumberFormat::LowerRoman => "lowerRoman",
        PageNumberFormat::UpperLetter => "upperLetter",
        PageNumberFormat::LowerLetter => "lowerLetter",
    }
}

fn page_separator_name(separator: PageNumberSeparator) -> &'static str {
    match separator {
        PageNumberSeparator::Hyphen => "hyphen",
        PageNumberSeparator::Period => "period",
        PageNumberSeparator::Colon => "colon",
        PageNumberSeparator::EmDash => "emDash",
        PageNumberSeparator::EnDash => "endash",
    }
}

fn story_wants_number_paragraph(
    header: bool,
    body: &HeaderFooterBody,
    fallback_display: PageNumberDisplay,
) -> bool {
    let rich = !body.blocks.is_empty();
    let text = body.text.as_deref().unwrap_or("");
    if header {
        let numbered = matches!(
            body.page_number_display,
            Some(display) if display != PageNumberDisplay::None
        );
        return !rich || !text.is_empty() || numbered;
    }
    let display = body.page_number_display.unwrap_or(fallback_display);
    !rich || !text.is_empty() || display != PageNumberDisplay::None
}

fn splice_sect_pr(body: &[u8], sect: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(body).ok()?;
    let trimmed = text.trim_end();
    if !trimmed.ends_with("</w:p>") {
        return None;
    }
    let p_start = trimmed.rfind("<w:p>")?;
    let ppr_rel = trimmed[p_start..].find("</w:pPr>")?;
    let insert_at = p_start + ppr_rel;
    let mut merged = Vec::with_capacity(body.len() + sect.len());
    merged.extend_from_slice(&body[..insert_at]);
    merged.extend_from_slice(sect);
    merged.extend_from_slice(&body[insert_at..]);
    Some(merged)
}

fn prepared_body_blocks(doc: &PreparedDocument) -> Vec<Block> {
    let mut blocks = Vec::new();
    for section in &doc.sections {
        for child in &section.children {
            if let SectionBlock::Body(block) = child {
                blocks.push(block.clone());
            }
        }
    }
    blocks
}

fn plant_prepared_images(doc: &mut PreparedDocument, markdown: &str, options: &ConvertOptions) {
    let resolved = crate::options::resolve_sections(markdown, options);
    for (section, input) in doc.sections.iter_mut().zip(resolved) {
        plant_section_footnotes(section);
        plant_slot(
            section.headers.default.as_mut(),
            input.headers.default.as_ref(),
        );
        plant_slot(section.headers.first.as_mut(), input.headers.first.as_ref());
        plant_slot(section.headers.even.as_mut(), input.headers.even.as_ref());
        plant_slot(
            section.footers.default.as_mut(),
            input.footers.default.as_ref(),
        );
        plant_slot(section.footers.first.as_mut(), input.footers.first.as_ref());
        plant_slot(section.footers.even.as_mut(), input.footers.even.as_ref());
    }
    let _ = markdown;
}

fn plant_slot(
    slot: Option<&mut HeaderFooterBody>,
    input: Option<&crate::options::ResolvedSlotInput>,
) {
    let Some(body) = slot else {
        return;
    };
    let Some(markdown) = input.and_then(|slot| slot.content.markdown.as_deref()) else {
        return;
    };
    plant_markdown_images(&mut body.blocks, markdown);
}

fn plant_section_footnotes(section: &mut PreparedSection) {
    let mut children = Vec::new();
    for block in &section.children {
        if let SectionBlock::Body(block) = block {
            children.push(block.clone());
        }
    }
    let mut document = Document { children };
    plant_footnote_images(&mut document, &section.markdown);
    let mut planted = document.children.into_iter();
    for block in &mut section.children {
        if let SectionBlock::Body(body) = block {
            if let Some(next) = planted.next() {
                *body = next;
            }
        }
    }
}

fn plant_markdown_images(blocks: &mut [Block], markdown: &str) {
    let urls = markdown_image_urls(markdown);
    if !urls.is_empty() {
        let mut urls = urls.into_iter();
        insert_blank_images(blocks, &mut urls);
    }
    stamp_image_labels(blocks, markdown);
}

fn insert_blank_images(blocks: &mut [Block], urls: &mut impl Iterator<Item = String>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                let blank = paragraph
                    .children
                    .iter()
                    .all(|run| run.text.is_empty() && run.footnote.is_none() && run.link.is_none());
                if blank {
                    if let Some(url) = urls.next() {
                        paragraph.children = vec![Inline {
                            text: format!("{IMAGE_MARK}{url}"),
                            bold: false,
                            italic: false,
                            underline: false,
                            strike: false,
                            code: false,
                            link: None,
                            footnote: None,
                            math_omml: None,
                            cross_ref: None,
                        }];
                    }
                }
            }
            Block::List(list) => {
                for item in &mut list.children {
                    insert_blank_images(&mut item.children, urls);
                }
            }
            Block::Blockquote(quote) => insert_blank_images(&mut quote.children, urls),
            Block::Heading(_)
            | Block::CodeBlock(_)
            | Block::Table(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Caption(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. }
            | Block::FootnoteDefinition { .. } => {}
        }
    }
}

fn markdown_image_urls(markdown: &str) -> Vec<String> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let mut urls = Vec::new();
    let mut footnote = 0_u32;
    for event in Parser::new_ext(markdown, options) {
        match event {
            MdEvent::Start(Tag::FootnoteDefinition(_)) => footnote = footnote.saturating_add(1),
            MdEvent::End(TagEnd::FootnoteDefinition) if footnote > 0 => {
                footnote = footnote.saturating_sub(1);
            }
            MdEvent::Start(Tag::Image { dest_url, .. }) if footnote == 0 => {
                urls.push(dest_url.to_string());
            }
            _ => {}
        }
    }
    urls
}

impl State {
    fn write_document(&mut self, doc: &Document) -> Result<Vec<u8>, MarkdownConversionError> {
        self.headings.clear();
        self.index_captions(&doc.children);
        collect_headings(&doc.children, &mut self.headings);
        let has_footnotes = !self.footnote_order.is_empty();
        self.attach_default_footer()?;
        let mut xml = Xml::new()?;
        xml.start(
            "w:document",
            &[("xmlns:w", W_NS), ("xmlns:r", R_NS), ("xmlns:m", MATH_NS)],
        )?;
        xml.start("w:body", &[])?;
        if doc.children.is_empty() {
            self.write_body_paragraph(&mut xml, &[], Look::body())?;
        } else {
            let mut previous_code = false;
            for block in &doc.children {
                if matches!(block, Block::FootnoteDefinition { .. }) {
                    self.write_block(&mut xml, block, None)?;
                    continue;
                }
                let code = matches!(block, Block::CodeBlock(_));
                if code && previous_code {
                    write_code_spacer(&mut xml)?;
                }
                self.write_block(&mut xml, block, None)?;
                previous_code = code;
            }
        }
        self.assign_story_ids(has_footnotes);
        self.write_section(&mut xml)?;
        xml.end("w:body")?;
        xml.end("w:document")?;
        Ok(xml.finish())
    }

    fn write_prepared(
        &mut self,
        doc: &PreparedDocument,
    ) -> Result<Vec<u8>, MarkdownConversionError> {
        self.index_heading_anchors(doc);
        self.index_prepared_captions(doc);
        let mut drawn = Vec::new();
        let count = doc.sections.len();
        for (index, section) in doc.sections.iter().enumerate() {
            self.content_width = section.page.content_width.max(1);
            self.active_style = Some(section.style.clone());
            self.paragraph_align_override = None;
            let rel_start = self.story_rels.len();
            self.attach_section_stories(section)?;
            let rel_end = self.story_rels.len();
            let mut xml = Xml::fragment();
            let mut wrote = false;
            let mut previous_code = false;
            for child in &section.children {
                match child {
                    SectionBlock::Body(Block::FootnoteDefinition { .. }) => {}
                    SectionBlock::Body(block) => {
                        let code = matches!(block, Block::CodeBlock(_));
                        if code && previous_code {
                            write_code_spacer(&mut xml)?;
                        }
                        self.write_block(&mut xml, block, None)?;
                        previous_code = code;
                        wrote = true;
                    }
                    SectionBlock::TableOfContents(toc) => {
                        if self.write_prepared_toc(&mut xml, toc)? {
                            wrote = true;
                            previous_code = false;
                        }
                    }
                }
            }
            if !wrote {
                let look = self.body_look();
                self.write_body_paragraph(&mut xml, &[], look)?;
            }
            drawn.push(DrawnSection {
                body: xml.finish(),
                rel_start,
                rel_end,
                page_width: section.page.width,
                page_height: section.page.height,
                orientation: section.page.orientation,
                margin_top: section.page.margin_top,
                margin_right: section.page.margin_right,
                margin_bottom: section.page.margin_bottom,
                margin_left: section.page.margin_left,
                margin_header: section.page.margin_header.unwrap_or(708),
                margin_footer: section.page.margin_footer.unwrap_or(708),
                margin_gutter: section.page.margin_gutter.unwrap_or(0),
                numbering: section.page_numbering.clone(),
                title_page: section.title_page,
                break_type: section.break_type,
                last: index + 1 == count,
            });
        }
        self.active_style = None;
        self.paragraph_align_override = None;
        let has_footnotes = !self.footnote_order.is_empty();
        self.assign_story_ids(has_footnotes);
        let mut xml = Xml::new()?;
        xml.start(
            "w:document",
            &[("xmlns:w", W_NS), ("xmlns:r", R_NS), ("xmlns:m", MATH_NS)],
        )?;
        xml.start("w:body", &[])?;
        if drawn.is_empty() {
            self.write_body_paragraph(&mut xml, &[], Look::body())?;
            self.write_section(&mut xml)?;
        } else {
            for section in &drawn {
                let sect = self.section_properties(section)?;
                if section.last {
                    xml.push_bytes(&section.body);
                    xml.push_bytes(&sect);
                } else if let Some(merged) = splice_sect_pr(&section.body, &sect) {
                    xml.push_bytes(&merged);
                } else {
                    xml.push_bytes(&section.body);
                    xml.start("w:p", &[])?;
                    xml.start("w:pPr", &[])?;
                    xml.push_bytes(&sect);
                    xml.end("w:pPr")?;
                    xml.end("w:p")?;
                }
            }
        }
        xml.end("w:body")?;
        xml.end("w:document")?;
        Ok(xml.finish())
    }

    /// Same registration order as `parse::prepare`: every section body, then
    /// each section's header slots, then its footer slots.
    fn index_heading_anchors(&mut self, doc: &PreparedDocument) {
        for section in &doc.sections {
            for child in &section.children {
                if let SectionBlock::Body(block) = child {
                    self.remember_block_anchor(block);
                }
            }
        }
        for section in &doc.sections {
            self.remember_story_anchors(&section.headers);
            self.remember_story_anchors(&section.footers);
        }
    }

    fn remember_story_anchors(&mut self, group: &ResolvedHeaderFooter) {
        for slot in [
            group.default.as_ref(),
            group.first.as_ref(),
            group.even.as_ref(),
        ] {
            let Some(slot) = slot else {
                continue;
            };
            for block in &slot.blocks {
                self.remember_block_anchor(block);
            }
        }
    }

    fn remember_block_anchor(&mut self, block: &Block) {
        match block {
            Block::Heading(heading) => {
                self.anchors
                    .remember(&inline_text(&heading.children), &heading.bookmark);
            }
            Block::List(list) => {
                for item in &list.children {
                    for child in &item.children {
                        self.remember_block_anchor(child);
                    }
                }
            }
            Block::Blockquote(quote) => {
                for child in &quote.children {
                    self.remember_block_anchor(child);
                }
            }
            Block::Paragraph(_)
            | Block::CodeBlock(_)
            | Block::Table(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Caption(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. }
            | Block::FootnoteDefinition { .. } => {}
        }
    }

    /// Figure and table numbers follow body order across sections.
    fn index_prepared_captions(&mut self, doc: &PreparedDocument) {
        for section in &doc.sections {
            for child in &section.children {
                if let SectionBlock::Body(block) = child {
                    self.index_caption_block(block);
                }
            }
        }
    }

    fn index_captions(&mut self, blocks: &[Block]) {
        for block in blocks {
            self.index_caption_block(block);
        }
    }

    fn index_caption_block(&mut self, block: &Block) {
        match block {
            Block::Caption(caption) => {
                let number = match caption.kind {
                    CaptionKind::Figure => {
                        self.figure_seq = self.figure_seq.saturating_add(1);
                        self.figure_seq
                    }
                    CaptionKind::Table => {
                        self.table_seq = self.table_seq.saturating_add(1);
                        self.table_seq
                    }
                };
                self.caption_numbers
                    .entry(caption.bookmark.clone())
                    .or_insert(number);
            }
            Block::List(list) => {
                for item in &list.children {
                    for child in &item.children {
                        self.index_caption_block(child);
                    }
                }
            }
            Block::Blockquote(quote) => {
                for child in &quote.children {
                    self.index_caption_block(child);
                }
            }
            Block::FootnoteDefinition { children, .. } => {
                for child in children {
                    self.index_caption_block(child);
                }
            }
            Block::Paragraph(_)
            | Block::Heading(_)
            | Block::CodeBlock(_)
            | Block::Table(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. } => {}
        }
    }

    fn write_section(&self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        xml.start("w:sectPr", &[])?;
        for rel in &self.story_rels {
            let Some(reference) = rel.reference else {
                continue;
            };
            let tag = if rel.rel_type.ends_with("/header") {
                "w:headerReference"
            } else if rel.rel_type.ends_with("/footer") {
                "w:footerReference"
            } else {
                continue;
            };
            xml.empty(tag, &[("w:type", reference), ("r:id", &rel.id)])?;
        }
        xml.empty(
            "w:pgSz",
            &[
                ("w:w", PAGE_WIDTH),
                ("w:h", PAGE_HEIGHT),
                ("w:orient", "portrait"),
            ],
        )?;
        xml.empty(
            "w:pgMar",
            &[
                ("w:top", "1440"),
                ("w:right", "1080"),
                ("w:bottom", "1440"),
                ("w:left", "1080"),
                ("w:header", "708"),
                ("w:footer", "708"),
                ("w:gutter", "0"),
            ],
        )?;
        xml.end("w:sectPr")
    }

    fn assign_story_ids(&mut self, has_footnotes: bool) {
        let mut number = 3 + self.links.len() + usize::from(has_footnotes);
        for rel in &mut self.story_rels {
            rel.id = format!("rId{number}");
            number += 1;
        }
    }

    fn attach_default_footer(&mut self) -> Result<(), MarkdownConversionError> {
        let bytes = write_page_number_story("w:ftr", "center", "current", 24)?;
        let rels = empty_relationships()?;
        self.story_parts.push(StoryPart {
            path: "word/footer1.xml".to_string(),
            bytes,
            content_type: Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml",
            ),
        });
        self.story_parts.push(StoryPart {
            path: "word/_rels/footer1.xml.rels".to_string(),
            bytes: rels,
            content_type: None,
        });
        self.story_rels.push(StoryRel {
            id: String::new(),
            rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
            target: "footer1.xml".to_string(),
            reference: Some("default"),
        });
        Ok(())
    }

    fn body_look(&self) -> Look {
        match self
            .active_style
            .as_ref()
            .and_then(|style| style.paragraph_size)
        {
            Some(size) => Look::sized(size),
            None => Look::body(),
        }
    }

    fn list_look(&self) -> Look {
        Look::sized(
            self.active_style
                .as_ref()
                .and_then(|style| style.list_item_size)
                .unwrap_or(24),
        )
    }

    fn quote_size(&self) -> u32 {
        self.active_style
            .as_ref()
            .and_then(|style| style.blockquote_size)
            .unwrap_or(24)
    }

    fn code_sizes(&self) -> (u32, u32) {
        match self
            .active_style
            .as_ref()
            .and_then(|style| style.code_block_size)
        {
            Some(size) => (size, size),
            None => (18, 20),
        }
    }

    fn body_spacing(&self) -> (u32, u32, u32) {
        let Some(style) = &self.active_style else {
            return (240, 240, 276);
        };
        let spacing = style.paragraph_spacing;
        let line = (style.line_spacing * 240.0).round() as u32;
        (spacing, spacing, line.max(1))
    }

    fn body_align(&self) -> &'static str {
        let align = self.paragraph_align_override.or_else(|| {
            self.active_style
                .as_ref()
                .map(|style| style.paragraph_alignment)
        });
        match align {
            Some(Alignment::Center) => "center",
            Some(Alignment::Right) => "right",
            Some(Alignment::Justified) => "both",
            Some(Alignment::Left) | None => "left",
        }
    }

    fn bidi_val(&self) -> &'static str {
        match self.active_style.as_ref().map(|style| style.direction) {
            Some(Direction::Rtl) => "true",
            Some(Direction::Ltr) | None => "false",
        }
    }

    fn heading_align(&self, level: u8) -> Option<&'static str> {
        let style = self.active_style.as_ref()?;
        let index = usize::from(level.clamp(1, 6) - 1);
        let align = style.heading_alignments[index].or(style.heading_alignment)?;
        Some(alignment_name(align))
    }

    fn heading_half_points(&self, level: u8) -> u32 {
        let Some(style) = &self.active_style else {
            return heading_size(level);
        };
        let index = usize::from(level.clamp(1, 6) - 1);
        style.heading_size[index]
            .map(|size| size.max(1))
            .unwrap_or_else(|| style.heading_run_size(level))
    }

    fn toc_entry_look(&self, level: u8) -> TocEntryStyle {
        let Some(style) = &self.active_style else {
            return toc_entry_style(level, self.toc_paragraph_size);
        };
        let index = usize::from(level.clamp(1, 6) - 1);
        let size = style.toc_heading_font_size[index]
            .or(style.toc_font_size)
            .unwrap_or_else(|| {
                let base = style
                    .paragraph_size
                    .unwrap_or(self.toc_paragraph_size.max(1));
                base.saturating_sub(u32::from(level.saturating_sub(1)) * 2)
                    .max(1)
            });
        TocEntryStyle {
            size,
            bold: style.toc_heading_bold[index].unwrap_or(level == 1),
            italic: style.toc_heading_italic[index].unwrap_or(false),
        }
    }

    fn write_run_font(&self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        let Some(font) = self
            .active_style
            .as_ref()
            .and_then(|style| style.font_family.as_deref())
            .filter(|font| !font.trim().is_empty())
        else {
            return Ok(());
        };
        xml.empty(
            "w:rFonts",
            &[
                ("w:ascii", font),
                ("w:cs", font),
                ("w:eastAsia", font),
                ("w:hAnsi", font),
            ],
        )
    }

    fn load_toc_settings(&mut self, settings: &TocSettings) {
        self.toc_native = settings.mode != TocMode::Links;
        self.toc_page_numbers = settings.page_numbers;
        self.toc_dot_leaders = settings.dot_leaders;
        self.toc_update_on_open = settings.update_fields_on_open;
        self.toc_title = settings.title.clone();
        self.toc_min_depth = settings.min_depth;
        self.toc_max_depth = settings.max_depth;
        if let Some(size) = self
            .active_style
            .as_ref()
            .and_then(|style| style.paragraph_size)
        {
            self.toc_paragraph_size = size;
        }
    }

    fn write_prepared_toc(
        &mut self,
        xml: &mut Xml,
        toc: &crate::model::TocBlock,
    ) -> Result<bool, MarkdownConversionError> {
        if !toc.primary || toc.headings.is_empty() {
            return Ok(false);
        }
        self.load_toc_settings(&toc.settings);
        if !self.toc_title.is_empty() {
            self.write_toc_title(xml)?;
        }
        let last = toc.headings.len() - 1;
        for (index, heading) in toc.headings.iter().enumerate() {
            let entry = TocHeading {
                text: heading.text.clone(),
                level: heading.level,
                bookmark: heading.bookmark.clone(),
            };
            self.write_toc_entry(xml, &entry, index == 0, index == last)?;
        }
        Ok(true)
    }

    fn attach_section_stories(
        &mut self,
        section: &PreparedSection,
    ) -> Result<(), MarkdownConversionError> {
        self.attach_header_slots(section)?;
        self.attach_footer_slots(section)?;
        Ok(())
    }

    fn attach_header_slots(
        &mut self,
        section: &PreparedSection,
    ) -> Result<(), MarkdownConversionError> {
        for (reference, slot) in [
            ("default", &section.headers.default),
            ("first", &section.headers.first),
            ("even", &section.headers.even),
        ] {
            if let Some(slot) = slot {
                self.write_story(
                    true,
                    reference,
                    slot,
                    PageNumberDisplay::None,
                    Alignment::Left,
                )?;
            }
        }
        Ok(())
    }

    fn attach_footer_slots(
        &mut self,
        section: &PreparedSection,
    ) -> Result<(), MarkdownConversionError> {
        let fallback_display = section
            .page_numbering
            .as_ref()
            .and_then(|numbering| numbering.display)
            .unwrap_or(PageNumberDisplay::Current);
        let fallback_align = section
            .page_numbering
            .as_ref()
            .and_then(|numbering| numbering.alignment)
            .unwrap_or(Alignment::Center);
        let any = section.footers.default.is_some()
            || section.footers.first.is_some()
            || section.footers.even.is_some();
        if !any {
            if fallback_display != PageNumberDisplay::None {
                let synthetic = HeaderFooterBody {
                    blocks: Vec::new(),
                    text: None,
                    alignment: Some(fallback_align),
                    page_number_display: Some(fallback_display),
                };
                self.write_story(
                    false,
                    "default",
                    &synthetic,
                    fallback_display,
                    fallback_align,
                )?;
            }
            return Ok(());
        }
        for (reference, slot) in [
            ("default", &section.footers.default),
            ("first", &section.footers.first),
            ("even", &section.footers.even),
        ] {
            if let Some(slot) = slot {
                self.write_story(false, reference, slot, fallback_display, fallback_align)?;
            }
        }
        Ok(())
    }

    fn write_story(
        &mut self,
        header: bool,
        reference: &'static str,
        body: &HeaderFooterBody,
        fallback_display: PageNumberDisplay,
        fallback_align: Alignment,
    ) -> Result<(), MarkdownConversionError> {
        let number = if header {
            self.header_count = self.header_count.saturating_add(1);
            self.header_count
        } else {
            self.footer_count = self.footer_count.saturating_add(1);
            self.footer_count
        };
        let stem = if header { "header" } else { "footer" };
        let root = if header { "w:hdr" } else { "w:ftr" };
        let content_type = if header {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"
        } else {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"
        };
        let rel_type = if header {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header"
        } else {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer"
        };
        let saved_align = self.paragraph_align_override;
        let saved_header = self.in_header_footer;
        self.paragraph_align_override = body.alignment;
        self.in_header_footer = true;
        self.story_links.clear();
        let mut xml = Xml::new()?;
        xml.start(
            root,
            &[("xmlns:w", W_NS), ("xmlns:r", R_NS), ("xmlns:m", MATH_NS)],
        )?;
        for block in &body.blocks {
            self.write_block(&mut xml, block, None)?;
        }
        if story_wants_number_paragraph(header, body, fallback_display) {
            let display = body.page_number_display.unwrap_or(fallback_display);
            let align = body.alignment.unwrap_or(if header {
                Alignment::Left
            } else {
                fallback_align
            });
            self.write_number_paragraph(
                &mut xml,
                body.text.as_deref().unwrap_or(""),
                display,
                align,
            )?;
        }
        xml.end(root)?;
        let bytes = xml.finish();
        let rels = self.story_relationship_bytes()?;
        self.in_header_footer = saved_header;
        self.paragraph_align_override = saved_align;
        self.story_links.clear();
        self.story_parts.push(StoryPart {
            path: format!("word/{stem}{number}.xml"),
            bytes,
            content_type: Some(content_type),
        });
        self.story_parts.push(StoryPart {
            path: format!("word/_rels/{stem}{number}.xml.rels"),
            bytes: rels,
            content_type: None,
        });
        self.story_rels.push(StoryRel {
            id: String::new(),
            rel_type,
            target: format!("{stem}{number}.xml"),
            reference: Some(reference),
        });
        Ok(())
    }

    fn story_relationship_bytes(&self) -> Result<Vec<u8>, MarkdownConversionError> {
        let mut xml = Xml::new()?;
        xml.start("Relationships", &[("xmlns", REL_NS)])?;
        for (index, target) in self.story_links.iter().enumerate() {
            let id = format!("rId{}", index + 1);
            xml.empty(
                "Relationship",
                &[
                    ("Id", &id),
                    (
                        "Type",
                        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                    ),
                    ("Target", target),
                    ("TargetMode", "External"),
                ],
            )?;
        }
        xml.end("Relationships")?;
        Ok(xml.finish())
    }

    fn write_number_paragraph(
        &mut self,
        xml: &mut Xml,
        text: &str,
        display: PageNumberDisplay,
        align: Alignment,
    ) -> Result<(), MarkdownConversionError> {
        let align = alignment_name(align);
        let bidi = self.bidi_val();
        let size = self
            .active_style
            .as_ref()
            .and_then(|style| style.paragraph_size)
            .unwrap_or(24)
            .to_string();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:bidi", &[("w:val", bidi)])?;
        xml.empty("w:jc", &[("w:val", align)])?;
        xml.end("w:pPr")?;
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        xml.empty("w:sz", &[("w:val", &size)])?;
        xml.empty("w:szCs", &[("w:val", &size)])?;
        self.write_run_font(xml)?;
        xml.empty("w:rtl", &[("w:val", bidi)])?;
        xml.end("w:rPr")?;
        if !text.is_empty() {
            xml.text_elem(text)?;
            if display != PageNumberDisplay::None {
                xml.text_elem(" ")?;
            }
        }
        let field = match display {
            PageNumberDisplay::None => "none",
            PageNumberDisplay::Current => "current",
            PageNumberDisplay::CurrentAndTotal => "currentAndTotal",
            PageNumberDisplay::CurrentAndSectionTotal => "currentAndSectionTotal",
        };
        write_page_number_fields(xml, field)?;
        if text.is_empty() && display == PageNumberDisplay::None {
            xml.text_elem("")?;
        }
        xml.end("w:r")?;
        xml.end("w:p")
    }

    fn section_properties(
        &self,
        section: &DrawnSection,
    ) -> Result<Vec<u8>, MarkdownConversionError> {
        let mut xml = Xml::fragment();
        xml.start("w:sectPr", &[])?;
        for rel in &self.story_rels[section.rel_start..section.rel_end] {
            let Some(reference) = rel.reference else {
                continue;
            };
            let tag = if rel.rel_type.ends_with("/header") {
                "w:headerReference"
            } else if rel.rel_type.ends_with("/footer") {
                "w:footerReference"
            } else {
                continue;
            };
            xml.empty(tag, &[("w:type", reference), ("r:id", &rel.id)])?;
        }
        if let Some(kind) = section.break_type {
            xml.empty("w:type", &[("w:val", section_break_name(kind))])?;
        }
        let (width, height, orient) = match section.orientation {
            Orientation::Landscape => (section.page_height, section.page_width, "landscape"),
            Orientation::Portrait => (section.page_width, section.page_height, "portrait"),
        };
        let width = width.to_string();
        let height = height.to_string();
        xml.empty(
            "w:pgSz",
            &[("w:w", &width), ("w:h", &height), ("w:orient", orient)],
        )?;
        let top = section.margin_top.to_string();
        let right = section.margin_right.to_string();
        let bottom = section.margin_bottom.to_string();
        let left = section.margin_left.to_string();
        let header = section.margin_header.to_string();
        let footer = section.margin_footer.to_string();
        let gutter = section.margin_gutter.to_string();
        xml.empty(
            "w:pgMar",
            &[
                ("w:top", &top),
                ("w:right", &right),
                ("w:bottom", &bottom),
                ("w:left", &left),
                ("w:header", &header),
                ("w:footer", &footer),
                ("w:gutter", &gutter),
            ],
        )?;
        if let Some(numbering) = &section.numbering {
            let start = numbering.start.map(|value| value.to_string());
            let format = numbering.format.map(page_format_name);
            let separator = numbering.separator.map(page_separator_name);
            let mut attrs = Vec::new();
            if let Some(start) = start.as_deref() {
                attrs.push(("w:start", start));
            }
            if let Some(format) = format {
                attrs.push(("w:fmt", format));
            }
            if let Some(separator) = separator {
                attrs.push(("w:chapSep", separator));
            }
            if !attrs.is_empty() {
                xml.empty("w:pgNumType", &attrs)?;
            }
        }
        match section.title_page {
            Some(true) => xml.empty("w:titlePg", &[])?,
            Some(false) => xml.empty("w:titlePg", &[("w:val", "false")])?,
            None => {}
        }
        xml.end("w:sectPr")?;
        Ok(xml.finish())
    }

    fn write_toc(&mut self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        if self.toc_placed {
            return Ok(());
        }
        self.toc_placed = true;
        let min_depth = self.toc_min_depth;
        let max_depth = self.toc_max_depth;
        let entries: Vec<TocHeading> = self
            .headings
            .iter()
            .filter(|heading| heading.level >= min_depth && heading.level <= max_depth)
            .cloned()
            .collect();
        if entries.is_empty() {
            return Ok(());
        }
        if self.toc_native && self.toc_update_on_open {
            self.update_fields = true;
        }
        if !self.toc_title.is_empty() {
            self.write_toc_title(xml)?;
        }
        let last = entries.len() - 1;
        for (index, heading) in entries.iter().enumerate() {
            self.write_toc_entry(xml, heading, index == 0, index == last)?;
        }
        Ok(())
    }

    fn write_toc_title(&mut self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:pStyle", &[("w:val", "TOCHeading")])?;
        xml.empty("w:bidi", &[("w:val", "false")])?;
        xml.empty("w:spacing", &[("w:after", "240")])?;
        xml.empty("w:jc", &[("w:val", "center")])?;
        xml.end("w:pPr")?;
        xml.start("w:r", &[])?;
        xml.text_elem(&self.toc_title.clone())?;
        xml.end("w:r")?;
        xml.end("w:p")
    }

    fn write_toc_entry(
        &mut self,
        xml: &mut Xml,
        heading: &TocHeading,
        first: bool,
        last: bool,
    ) -> Result<(), MarkdownConversionError> {
        let pos = self.content_width.to_string();
        let leader = if self.toc_dot_leaders { "dot" } else { "none" };
        let indent =
            (u32::from(heading.level.saturating_sub(self.toc_min_depth)) * 400).to_string();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.start("w:tabs", &[])?;
        xml.empty(
            "w:tab",
            &[("w:val", "right"), ("w:pos", &pos), ("w:leader", leader)],
        )?;
        xml.end("w:tabs")?;
        xml.empty("w:bidi", &[("w:val", "false")])?;
        xml.empty("w:spacing", &[("w:after", "120")])?;
        xml.empty("w:ind", &[("w:left", &indent)])?;
        xml.end("w:pPr")?;
        if self.toc_native && first {
            let instruction = toc_instruction(
                self.toc_min_depth,
                self.toc_max_depth,
                self.toc_page_numbers,
            );
            xml.start("w:r", &[])?;
            xml.empty(
                "w:fldChar",
                &[("w:fldCharType", "begin"), ("w:dirty", "true")],
            )?;
            xml.start("w:instrText", &[("xml:space", "preserve")])?;
            xml.text(&instruction)?;
            xml.end("w:instrText")?;
            xml.end("w:r")?;
            xml.start("w:r", &[])?;
            xml.empty("w:fldChar", &[("w:fldCharType", "separate")])?;
            xml.end("w:r")?;
        }
        let style = self.toc_entry_look(heading.level);
        xml.start(
            "w:hyperlink",
            &[("w:history", "1"), ("w:anchor", &heading.bookmark)],
        )?;
        self.write_toc_run(xml, &heading.text, style)?;
        xml.end("w:hyperlink")?;
        if self.toc_native && self.toc_page_numbers {
            xml.start("w:r", &[])?;
            xml.empty("w:tab", &[])?;
            xml.end("w:r")?;
            let instr = format!(" PAGEREF {} \\h ", heading.bookmark);
            xml.start("w:fldSimple", &[("w:instr", &instr)])?;
            xml.start("w:r", &[])?;
            xml.text_elem("?")?;
            xml.end("w:r")?;
            xml.end("w:fldSimple")?;
        }
        if self.toc_native && last {
            xml.start("w:r", &[])?;
            xml.empty("w:fldChar", &[("w:fldCharType", "end")])?;
            xml.end("w:r")?;
        }
        xml.end("w:p")
    }

    fn write_toc_run(
        &mut self,
        xml: &mut Xml,
        text: &str,
        style: TocEntryStyle,
    ) -> Result<(), MarkdownConversionError> {
        let size = style.size.to_string();
        let bold = if style.bold { "true" } else { "false" };
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        if style.bold {
            xml.empty("w:b", &[])?;
            xml.empty("w:bCs", &[])?;
        } else {
            xml.empty("w:b", &[("w:val", bold)])?;
            xml.empty("w:bCs", &[("w:val", bold)])?;
        }
        if style.italic {
            xml.empty("w:i", &[])?;
            xml.empty("w:iCs", &[])?;
        } else {
            xml.empty("w:i", &[("w:val", "false")])?;
            xml.empty("w:iCs", &[("w:val", "false")])?;
        }
        xml.empty("w:sz", &[("w:val", &size)])?;
        xml.empty("w:szCs", &[("w:val", &size)])?;
        self.write_run_font(xml)?;
        xml.empty("w:rtl", &[("w:val", self.bidi_val())])?;
        xml.end("w:rPr")?;
        xml.text_elem(text)?;
        xml.end("w:r")
    }

    fn write_block(
        &mut self,
        xml: &mut Xml,
        block: &Block,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        match block {
            Block::Paragraph(paragraph) if is_toc_paragraph(paragraph) => self.write_toc(xml),
            Block::Paragraph(paragraph) => self.write_paragraph(xml, paragraph, quote),
            Block::Heading(heading) if self.in_footnote => {
                self.write_footnote_heading(xml, heading, quote)
            }
            Block::Heading(heading) => self.write_heading(xml, heading),
            Block::List(list) => self.write_list(xml, list, 0, quote),
            Block::CodeBlock(code) => self.write_code_block(xml, code),
            Block::Blockquote(blockquote) => self.write_blockquote(xml, blockquote, quote),
            Block::Table(table) if self.in_footnote => self.write_flat_table(xml, table),
            Block::Table(table) => self.write_table(xml, table),
            Block::ThematicBreak => self.write_rule(xml, quote, None),
            Block::PageBreak => self.write_page_break(xml),
            Block::Comment(text) => self.write_comment(xml, text, quote),
            Block::Caption(caption) => self.write_caption(xml, caption),
            Block::Image(image) => self.write_image_block(xml, image, quote),
            Block::DisplayMath { omml } => self.write_display_math(xml, omml, quote),
            Block::ChartImage {
                bytes,
                width_px,
                height_px,
                alt,
            } => self.write_chart_image(xml, bytes, *width_px, *height_px, alt, quote),
            Block::FootnoteDefinition { .. } if self.in_header_footer => Err(
                MarkdownConversionError::new("Word headers and footers cannot contain footnotes"),
            ),
            Block::FootnoteDefinition { .. } => Ok(()),
        }
    }

    /// Display math is one paragraph. The parser already produced the OMML.
    fn write_display_math(
        &mut self,
        xml: &mut Xml,
        omml: &str,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        self.write_content_paragraph(xml, quote, |_, xml| emit_omml(xml, omml))
    }

    /// Chart PNG bytes use the same drawing and `word/media` path as images.
    fn write_chart_image(
        &mut self,
        xml: &mut Xml,
        bytes: &[u8],
        width_px: u32,
        height_px: u32,
        alt: &str,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        if bytes.is_empty() {
            return self.write_content_paragraph(xml, quote, |state, xml| {
                state.write_marked_run(
                    xml,
                    "[Chart could not be rendered]",
                    Marks {
                        bold: false,
                        italic: true,
                        underline: false,
                        strike: false,
                        color: "FF0000",
                        size: state.caption_size(),
                        courier: false,
                        shade: None,
                    },
                )
            });
        }
        self.write_content_paragraph(xml, quote, |state, xml| {
            state.embed_raster(
                xml,
                alt,
                None,
                bytes,
                "image/png",
                None,
                None,
                nonzero(width_px),
                nonzero(height_px),
            )
        })
    }

    fn write_content_paragraph(
        &mut self,
        xml: &mut Xml,
        quote: Option<&QuoteCtx>,
        write_body: impl FnOnce(&mut Self, &mut Xml) -> Result<(), MarkdownConversionError>,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        if let Some(quote) = quote {
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
        } else {
            let (before, after, line) = self.body_spacing();
            let align = self.body_align();
            let bidi = self.bidi_val();
            xml.empty("w:bidi", &[("w:val", bidi)])?;
            spacing(xml, before, after, Some(line), None)?;
            xml.empty("w:jc", &[("w:val", align)])?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        write_body(self, xml)?;
        xml.end("w:p")
    }

    /// `Figure N` / `Table N` as a `SEQ` field inside `caption.bookmark`.
    ///
    /// The label and the sequence sit in the bookmark. The colon and the
    /// caption text follow it. Inline code is not rewritten into a reference.
    fn write_caption(
        &mut self,
        xml: &mut Xml,
        caption: &Caption,
    ) -> Result<(), MarkdownConversionError> {
        let number = self
            .caption_numbers
            .get(&caption.bookmark)
            .copied()
            .unwrap_or(1);
        let label = self.caption_text(caption.kind);
        let sequence = caption_sequence(caption.kind);
        let size = self.caption_size();
        let marks = self.caption_marks(size);
        let look = Look {
            size: Some(size),
            force_bold: false,
            force_italic: marks.italic,
            color: None,
        };
        let (before, after, line) = self.caption_spacing();
        let bidi = self.bidi_val();
        let align = self.caption_align_name();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:pStyle", &[("w:val", "Caption")])?;
        xml.empty("w:bidi", &[("w:val", bidi)])?;
        spacing(xml, before, after, Some(line), None)?;
        xml.empty("w:jc", &[("w:val", align)])?;
        xml.end("w:pPr")?;
        let bookmark_id = self.next_bookmark_id();
        if !caption.bookmark.is_empty() {
            xml.empty(
                "w:bookmarkStart",
                &[
                    ("w:id", bookmark_id.as_str()),
                    ("w:name", caption.bookmark.as_str()),
                ],
            )?;
        }
        self.write_marked_run(xml, &format!("{label} "), marks)?;
        let instruction = format!(" SEQ {sequence} \\* ARABIC ");
        self.write_simple_field(xml, &instruction, &number.to_string(), &look)?;
        self.update_fields = true;
        if !caption.bookmark.is_empty() {
            xml.empty("w:bookmarkEnd", &[("w:id", bookmark_id.as_str())])?;
        }
        self.write_marked_run(xml, ": ", marks)?;
        self.write_runs(xml, &caption.children, &look)?;
        xml.end("w:p")
    }

    fn write_paragraph(
        &mut self,
        xml: &mut Xml,
        paragraph: &Paragraph,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        if let Some(quote) = quote {
            self.write_quote_paragraph(xml, &paragraph.children, quote, false)
        } else {
            let look = self.body_look();
            self.write_body_paragraph(xml, &paragraph.children, look)
        }
    }

    fn write_body_paragraph(
        &mut self,
        xml: &mut Xml,
        runs: &[Inline],
        look: Look,
    ) -> Result<(), MarkdownConversionError> {
        let (before, after, line) = self.body_spacing();
        let align = self.body_align();
        let bidi = self.bidi_val();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:bidi", &[("w:val", bidi)])?;
        spacing(xml, before, after, Some(line), None)?;
        xml.empty("w:jc", &[("w:val", align)])?;
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        self.write_runs(xml, runs, &look)?;
        xml.end("w:p")
    }

    fn write_heading(
        &mut self,
        xml: &mut Xml,
        heading: &Heading,
    ) -> Result<(), MarkdownConversionError> {
        let level = heading.level.clamp(1, 6);
        let numeric = level.to_string();
        let named = format!("Heading{level}");
        let before = if level == 1 { 480 } else { 240 };
        let bidi = self.bidi_val();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:pStyle", &[("w:val", &numeric)])?;
        xml.empty("w:pStyle", &[("w:val", &named)])?;
        xml.empty("w:bidi", &[("w:val", bidi)])?;
        spacing(xml, before, 120, None, None)?;
        if let Some(align) = self.heading_align(level) {
            xml.empty("w:jc", &[("w:val", align)])?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        let bookmark = heading.bookmark.as_str();
        let bookmark_id = if bookmark.is_empty() {
            None
        } else {
            Some(self.next_bookmark_id())
        };
        if let Some(id) = &bookmark_id {
            xml.empty("w:bookmarkStart", &[("w:id", id), ("w:name", bookmark)])?;
        }
        let size = self.heading_half_points(level);
        self.write_runs(xml, &heading.children, &Look::sized(size))?;
        if let Some(id) = &bookmark_id {
            xml.empty("w:bookmarkEnd", &[("w:id", id)])?;
        }
        xml.end("w:p")
    }

    fn write_list(
        &mut self,
        xml: &mut Xml,
        list: &List,
        level: u32,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        let num_id = if list.ordered {
            let sequence = list.sequence_id.unwrap_or_else(|| {
                self.ordered_starts
                    .iter()
                    .map(|(id, _)| *id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1)
            });
            let start = list.start.unwrap_or(1);
            if !self.ordered_starts.iter().any(|(id, _)| *id == sequence) {
                self.ordered_starts.push((sequence, start));
            }
            sequence.saturating_add(1)
        } else {
            1
        };
        let ilvl = level.min(8);
        for item in &list.children {
            self.write_list_item(xml, item, ilvl, num_id, quote)?;
        }
        Ok(())
    }

    fn write_list_item(
        &mut self,
        xml: &mut Xml,
        item: &ListItem,
        level: u32,
        num_id: u32,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        let mut saw_content = false;
        let mut index = 0;
        while index < item.children.len() {
            let image_count = match &item.children[index] {
                Block::Paragraph(_) => {
                    let mut next = index + 1;
                    while matches!(item.children.get(next), Some(Block::Image(_))) {
                        next += 1;
                    }
                    next - (index + 1)
                }
                _ => 0,
            };
            match &item.children[index] {
                Block::List(nested) => {
                    self.write_list(xml, nested, level + 1, quote)?;
                    saw_content = true;
                    index += 1;
                }
                Block::Paragraph(paragraph) => {
                    let images = item.children[index + 1..index + 1 + image_count]
                        .iter()
                        .map(|block| match block {
                            Block::Image(image) => image,
                            _ => unreachable!("image count only covers image blocks"),
                        })
                        .collect::<Vec<_>>();
                    if saw_content {
                        self.write_continuation(xml, &paragraph.children, level, quote, &images)?;
                    } else {
                        self.write_list_paragraph(
                            xml,
                            &paragraph.children,
                            level,
                            num_id,
                            quote,
                            &images,
                        )?;
                    }
                    saw_content = true;
                    index += 1 + image_count;
                }
                Block::Image(image) => {
                    self.write_listed_image(xml, image, level, num_id, quote, saw_content)?;
                    saw_content = true;
                    index += 1;
                }
                Block::ThematicBreak => {
                    if !saw_content {
                        self.write_list_paragraph(xml, &[], level, num_id, quote, &[])?;
                        saw_content = true;
                    }
                    self.write_rule(xml, quote, Some(720 * (level + 1)))?;
                    index += 1;
                }
                Block::FootnoteDefinition { .. } => {
                    self.write_block(xml, &item.children[index], quote)?;
                    index += 1;
                }
                _ => {
                    if !saw_content {
                        self.write_list_paragraph(xml, &[], level, num_id, quote, &[])?;
                        saw_content = true;
                    }
                    self.write_block(xml, &item.children[index], quote)?;
                    index += 1;
                }
            }
        }
        if !saw_content {
            self.write_list_paragraph(xml, &[], level, num_id, quote, &[])?;
        }
        Ok(())
    }

    fn write_list_paragraph(
        &mut self,
        xml: &mut Xml,
        runs: &[Inline],
        level: u32,
        num_id: u32,
        quote: Option<&QuoteCtx>,
        images: &[&Image],
    ) -> Result<(), MarkdownConversionError> {
        let ilvl = level.to_string();
        let num = num_id.to_string();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:pStyle", &[("w:val", "ListParagraph")])?;
        xml.start("w:numPr", &[])?;
        xml.empty("w:ilvl", &[("w:val", &ilvl)])?;
        xml.empty("w:numId", &[("w:val", &num)])?;
        xml.end("w:numPr")?;
        if let Some(quote) = quote {
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
        } else {
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 120, 120, None, None)?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        let look = self.list_look();
        self.write_runs(xml, runs, &look)?;
        for image in images {
            self.write_image_value(xml, image)?;
        }
        xml.end("w:p")
    }

    fn write_continuation(
        &mut self,
        xml: &mut Xml,
        runs: &[Inline],
        level: u32,
        quote: Option<&QuoteCtx>,
        images: &[&Image],
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        if let Some(quote) = quote {
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
        } else {
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 120, 120, None, None)?;
            indent(xml, 720 * (level + 1))?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        let look = self.list_look();
        self.write_runs(xml, runs, &look)?;
        for image in images {
            self.write_image_value(xml, image)?;
        }
        xml.end("w:p")
    }

    fn write_quote_paragraph(
        &mut self,
        xml: &mut Xml,
        runs: &[Inline],
        quote: &QuoteCtx,
        label: bool,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        write_quote_frame(xml, quote)?;
        spacing(xml, 240, 240, None, None)?;
        indent(xml, 720 * quote.level)?;
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        if label {
            let text = callout_label(quote.callout.as_ref()).unwrap_or("Note");
            self.write_marked_run(
                xml,
                text,
                plain_marks(
                    self.quote_size(),
                    callout_title(quote.callout.as_ref()),
                    true,
                ),
            )?;
        } else {
            let mut look = Look::sized(self.quote_size());
            look.force_italic = quote.italic_paragraphs;
            self.write_runs(xml, runs, &look)?;
        }
        xml.end("w:p")
    }

    fn write_blockquote(
        &mut self,
        xml: &mut Xml,
        block: &Blockquote,
        parent: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        let ctx = QuoteCtx {
            level: parent.map(|quote| quote.level).unwrap_or(0) + 1,
            callout: block
                .callout
                .as_ref()
                .map(copy_callout)
                .or_else(|| parent.and_then(|quote| quote.callout.as_ref().map(copy_callout))),
            italic_paragraphs: block.callout.is_none(),
        };
        if block.callout.is_some() {
            self.write_quote_paragraph(xml, &[], &ctx, true)?;
        }
        if block.children.is_empty() {
            if block.callout.is_none() {
                self.write_quote_paragraph(xml, &[], &ctx, false)?;
            }
            return Ok(());
        }
        for child in &block.children {
            self.write_block(xml, child, Some(&ctx))?;
        }
        Ok(())
    }

    fn write_rule(
        &mut self,
        xml: &mut Xml,
        quote: Option<&QuoteCtx>,
        list_indent: Option<u32>,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.start("w:pBdr", &[])?;
        let space = "1";
        xml.empty(
            "w:bottom",
            &[
                ("w:val", "single"),
                ("w:color", "A6A6A6"),
                ("w:sz", "6"),
                ("w:space", space),
            ],
        )?;
        if quote.is_some() {
            let color = quote_border(quote);
            xml.empty(
                "w:left",
                &[("w:val", "single"), ("w:color", color), ("w:sz", "3")],
            )?;
        }
        xml.end("w:pBdr")?;
        if let Some(quote) = quote {
            if let Some(fill) = callout_fill(quote.callout.as_ref()) {
                xml.empty("w:shd", &[("w:fill", fill)])?;
            }
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 240, 240, None, None)?;
            let left = quote.level * 720 + list_indent.unwrap_or(0);
            indent(xml, left)?;
        } else {
            spacing(xml, 120, 120, None, None)?;
            if let Some(left) = list_indent {
                indent(xml, left)?;
            }
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        xml.end("w:p")
    }

    fn write_code_block(
        &mut self,
        xml: &mut Xml,
        code: &CodeBlock,
    ) -> Result<(), MarkdownConversionError> {
        let chrome = code_chrome(code);
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.start("w:pBdr", &[])?;
        for side in ["w:top", "w:bottom", "w:left", "w:right"] {
            xml.empty(
                side,
                &[
                    ("w:val", "single"),
                    ("w:color", chrome.border.as_str()),
                    ("w:sz", "1"),
                ],
            )?;
        }
        xml.end("w:pBdr")?;
        xml.empty("w:shd", &[("w:fill", chrome.fill.as_str())])?;
        xml.empty("w:bidi", &[("w:val", "false")])?;
        spacing(xml, 240, 240, Some(360), Some("exact"))?;
        indent(xml, 360)?;
        xml.empty("w:jc", &[("w:val", "left")])?;
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        let (label_size, line_size) = self.code_sizes();
        if chrome.show_label {
            if let Some(language) = code
                .language
                .as_deref()
                .filter(|language| !language.is_empty())
            {
                self.write_code_run(
                    xml,
                    language,
                    label_size,
                    Some(&chrome.label_color),
                    true,
                    false,
                )?;
                self.write_code_run(xml, "\n", label_size, None, false, true)?;
            }
        }
        let highlights = code_highlights(code);
        if highlights.is_empty() {
            let normalized = code.value.replace("\r\n", "\n").replace('\r', "\n");
            let lines: Vec<&str> = normalized.split('\n').collect();
            for (index, line) in lines.iter().enumerate() {
                self.write_code_run(
                    xml,
                    &leading_nbsp(line),
                    line_size,
                    Some(&chrome.plain_color),
                    false,
                    false,
                )?;
                if index + 1 < lines.len() {
                    self.write_code_run(xml, "\n", line_size, None, false, true)?;
                }
            }
        } else {
            self.write_highlight_spans(xml, &highlights, line_size)?;
        }
        xml.end("w:p")
    }

    /// Parser highlight spans already carry the theme color. Empty spans keep
    /// the plain code color, so highlighting that is off adds no token color.
    fn write_highlight_spans(
        &mut self,
        xml: &mut Xml,
        spans: &[HighlightPaint],
        size: u32,
    ) -> Result<(), MarkdownConversionError> {
        let mut line_start = true;
        for span in spans {
            let parts: Vec<&str> = span.text.split('\n').collect();
            for (index, segment) in parts.iter().enumerate() {
                if !segment.is_empty() {
                    let text = if line_start {
                        leading_nbsp(segment)
                    } else {
                        (*segment).to_string()
                    };
                    self.write_code_run(xml, &text, size, span.color.as_deref(), false, false)?;
                    line_start = false;
                }
                if index + 1 < parts.len() {
                    self.write_code_run(xml, "\n", size, None, false, true)?;
                    line_start = true;
                }
            }
        }
        Ok(())
    }

    fn write_code_run(
        &mut self,
        xml: &mut Xml,
        text: &str,
        size: u32,
        color: Option<&str>,
        bold: bool,
        is_break: bool,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        if bold {
            xml.empty("w:b", &[])?;
            xml.empty("w:bCs", &[])?;
        }
        if let Some(color) = color {
            xml.empty("w:color", &[("w:val", color)])?;
        }
        let size = size.to_string();
        xml.empty("w:sz", &[("w:val", &size)])?;
        xml.empty("w:szCs", &[("w:val", &size)])?;
        xml.empty("w:rtl", &[("w:val", "false")])?;
        courier(xml)?;
        xml.empty("w:shd", &[("w:fill", "F5F5F5")])?;
        xml.end("w:rPr")?;
        if is_break {
            xml.empty("w:br", &[])?;
        }
        xml.text_elem(text)?;
        xml.end("w:r")
    }

    fn write_table(&mut self, xml: &mut Xml, table: &Table) -> Result<(), MarkdownConversionError> {
        let placed = place_table(table);
        let columns = placed
            .iter()
            .flat_map(|(_, cells)| cells.iter())
            .map(|cell| cell.column + cell.column_span as usize)
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return Ok(());
        }
        let widths = column_widths(table);
        let table_width = widths
            .map(|widths| widths.iter().take(columns).sum::<u32>())
            .filter(|width| *width > 0)
            .unwrap_or(self.content_width);
        let layout = if widths.is_some() { "fixed" } else { "autofit" };
        xml.start("w:tbl", &[])?;
        xml.start("w:tblPr", &[])?;
        let width = table_width.to_string();
        xml.empty("w:tblW", &[("w:type", "dxa"), ("w:w", &width)])?;
        xml.start("w:tblBorders", &[])?;
        for side in [
            "w:top",
            "w:left",
            "w:bottom",
            "w:right",
            "w:insideH",
            "w:insideV",
        ] {
            xml.empty(
                side,
                &[("w:val", "single"), ("w:color", "auto"), ("w:sz", "4")],
            )?;
        }
        xml.end("w:tblBorders")?;
        xml.empty("w:tblLayout", &[("w:type", layout)])?;
        xml.start("w:tblCellMar", &[])?;
        for side in ["w:top", "w:left", "w:bottom", "w:right"] {
            xml.empty(side, &[("w:type", "dxa"), ("w:w", "100")])?;
        }
        xml.end("w:tblCellMar")?;
        xml.end("w:tblPr")?;
        xml.start("w:tblGrid", &[])?;
        for index in 0..columns {
            let column = widths
                .and_then(|widths| widths.get(index).copied())
                .unwrap_or(100);
            let column = column.to_string();
            xml.empty("w:gridCol", &[("w:w", &column)])?;
        }
        xml.end("w:tblGrid")?;
        let split = table_row_split(table);
        for (header, cells) in &placed {
            self.write_placed_row(xml, cells, &table.align, *header, widths, split)?;
        }
        xml.end("w:tbl")
    }

    fn write_placed_row(
        &mut self,
        xml: &mut Xml,
        cells: &[PlacedCell<'_>],
        align: &[TableAlign],
        header: bool,
        widths: Option<&[u32]>,
        split: bool,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:tr", &[])?;
        xml.start("w:trPr", &[])?;
        if split {
            xml.empty("w:cantSplit", &[("w:val", "false")])?;
        } else {
            xml.empty("w:cantSplit", &[])?;
        }
        if header {
            xml.empty("w:tblHeader", &[])?;
        } else {
            xml.empty("w:tblHeader", &[("w:val", "false")])?;
        }
        xml.end("w:trPr")?;
        let header_fill = "F2F2F2";
        for cell in cells {
            let jc = jc_val(align.get(cell.column).copied());
            xml.start("w:tc", &[])?;
            xml.start("w:tcPr", &[])?;
            if let Some(widths) = widths {
                let span = cell.column_span as usize;
                let size = widths.iter().skip(cell.column).take(span).sum::<u32>();
                if size > 0 {
                    let size = size.to_string();
                    xml.empty("w:tcW", &[("w:type", "dxa"), ("w:w", &size)])?;
                }
            }
            let span = cell.column_span.max(1).to_string();
            xml.empty("w:gridSpan", &[("w:val", &span)])?;
            match cell.merge {
                CellMerge::Restart => {
                    xml.empty("w:vMerge", &[("w:val", "restart")])?;
                }
                CellMerge::Continue => {
                    xml.empty("w:vMerge", &[("w:val", "continue")])?;
                }
                CellMerge::None => {}
            }
            if header {
                xml.empty("w:shd", &[("w:fill", header_fill)])?;
            }
            xml.end("w:tcPr")?;
            if let Some(source) = cell.source.filter(|source| !source.blocks.is_empty()) {
                for block in &source.blocks {
                    self.write_block(xml, block, None)?;
                }
                if matches!(source.blocks.last(), Some(Block::Table(_))) {
                    self.write_cell_paragraph(xml, jc, &[], &Look::body())?;
                }
            } else {
                let runs = cell
                    .source
                    .map(|source| source.children.as_slice())
                    .unwrap_or(&[]);
                let mut look = Look::body();
                look.force_bold = header;
                self.write_cell_paragraph(xml, jc, runs, &look)?;
            }
            xml.end("w:tc")?;
        }
        xml.end("w:tr")
    }

    fn write_cell_paragraph(
        &mut self,
        xml: &mut Xml,
        jc: &str,
        runs: &[Inline],
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:bidi", &[("w:val", "false")])?;
        xml.empty("w:jc", &[("w:val", jc)])?;
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        self.write_runs(xml, runs, look)?;
        xml.end("w:p")
    }

    fn write_runs(
        &mut self,
        xml: &mut Xml,
        runs: &[Inline],
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        for run in runs {
            self.write_run(xml, run, look)?;
        }
        Ok(())
    }

    fn write_run(
        &mut self,
        xml: &mut Xml,
        run: &Inline,
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        if let Some(mark) = &run.footnote {
            if self.in_header_footer {
                return Err(MarkdownConversionError::new(
                    "Word headers and footers cannot contain footnotes",
                ));
            }
            return self.write_footnote_mark(xml, mark);
        }
        if let Some(omml) = run.math_omml.as_deref().filter(|omml| !omml.is_empty()) {
            return emit_omml(xml, omml);
        }
        if let Some(rest) = run.text.strip_prefix(IMAGE_MARK) {
            let (url, alt, title) = split_image_mark(rest);
            return self.write_image(xml, url, alt, title);
        }
        if let Some(cross) = &run.cross_ref {
            if !run.code {
                return self.write_cross_ref(xml, cross, look);
            }
        }
        match link_kind(run.link.as_deref()) {
            LinkKind::External(target) => {
                let id = self.remember_link(target);
                xml.start("w:hyperlink", &[("w:history", "1"), ("r:id", &id)])?;
                self.write_marked_run(xml, &run.text, run_marks(run, look))?;
                xml.end("w:hyperlink")
            }
            LinkKind::Anchor(anchor) => {
                let resolved = self
                    .anchors
                    .resolve(&anchor)
                    .map(str::to_string)
                    .unwrap_or(anchor);
                xml.start(
                    "w:hyperlink",
                    &[("w:history", "1"), ("w:anchor", &resolved)],
                )?;
                if run.code {
                    self.write_inline_code(xml, &run.text, look)?;
                } else {
                    self.write_marked_run(xml, &run.text, run_marks(run, look))?;
                }
                xml.end("w:hyperlink")
            }
            LinkKind::Plain => {
                if run.code {
                    self.write_inline_code(xml, &run.text, look)
                } else {
                    self.write_marked_run(xml, &run.text, run_marks(run, look))
                }
            }
        }
    }

    fn write_inline_code(
        &mut self,
        xml: &mut Xml,
        text: &str,
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        let size = look.size.unwrap_or(20);
        self.write_marked_run(
            xml,
            text,
            Marks {
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                color: "444444",
                size,
                courier: true,
                shade: Some("F5F5F5"),
            },
        )
    }

    fn write_marked_run(
        &mut self,
        xml: &mut Xml,
        text: &str,
        marks: Marks,
    ) -> Result<(), MarkdownConversionError> {
        if !text.contains('\n') {
            return self.write_marked_piece(xml, text, marks, false);
        }
        let parts: Vec<&str> = text.split('\n').collect();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                self.write_marked_piece(xml, "\n", marks, true)?;
            }
            if !part.is_empty() {
                self.write_marked_piece(xml, part, marks, false)?;
            }
        }
        Ok(())
    }

    fn write_marked_piece(
        &mut self,
        xml: &mut Xml,
        text: &str,
        marks: Marks,
        is_break: bool,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        if marks.bold {
            xml.empty("w:b", &[])?;
            xml.empty("w:bCs", &[])?;
        }
        if marks.italic {
            xml.empty("w:i", &[])?;
            xml.empty("w:iCs", &[])?;
        }
        if marks.underline {
            xml.empty("w:u", &[("w:val", "single")])?;
        }
        if marks.strike {
            xml.empty("w:strike", &[])?;
        }
        xml.empty("w:color", &[("w:val", marks.color)])?;
        let size = marks.size.to_string();
        xml.empty("w:sz", &[("w:val", &size)])?;
        xml.empty("w:szCs", &[("w:val", &size)])?;
        let rtl = self.bidi_val();
        xml.empty("w:rtl", &[("w:val", rtl)])?;
        write_proofing_language(xml, self.active_style.as_ref())?;
        if marks.courier {
            courier(xml)?;
        } else {
            self.write_run_font(xml)?;
        }
        if let Some(fill) = marks.shade {
            xml.empty("w:shd", &[("w:fill", fill)])?;
        }
        xml.end("w:rPr")?;
        if is_break {
            xml.empty("w:br", &[])?;
        }
        xml.text_elem(text)?;
        xml.end("w:r")
    }

    fn remember_link(&mut self, target: &str) -> String {
        if self.in_header_footer {
            remember_link(&mut self.story_links, target, 1)
        } else if self.in_footnote {
            remember_link(&mut self.footnote_links, target, 2)
        } else {
            remember_link(&mut self.links, target, 3)
        }
    }

    fn footnote_xml_id(&self, id: &str) -> String {
        match self.footnote_order.iter().position(|seen| seen == id) {
            Some(index) => (index + 1).to_string(),
            None => id.to_string(),
        }
    }

    fn next_bookmark_id(&mut self) -> String {
        let id = self.bookmark_id.to_string();
        self.bookmark_id = self.bookmark_id.saturating_add(1);
        id
    }

    fn write_pending_ref(&mut self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        if !self.pending_footnote_ref {
            return Ok(());
        }
        self.pending_footnote_ref = false;
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        xml.empty("w:rStyle", &[("w:val", "FootnoteReference")])?;
        xml.empty("w:vertAlign", &[("w:val", "superscript")])?;
        xml.end("w:rPr")?;
        xml.empty("w:footnoteRef", &[])?;
        xml.end("w:r")
    }

    fn write_footnote_mark(
        &mut self,
        xml: &mut Xml,
        mark: &FootnoteMark,
    ) -> Result<(), MarkdownConversionError> {
        let id = self.footnote_xml_id(&mark.id);
        if mark.first {
            let name = format!("mdfootnote_{id}");
            let bookmark = self.next_bookmark_id();
            xml.empty(
                "w:bookmarkStart",
                &[("w:id", bookmark.as_str()), ("w:name", name.as_str())],
            )?;
            self.write_footnote_reference_run(xml, &id)?;
            xml.empty("w:bookmarkEnd", &[("w:id", bookmark.as_str())])
        } else {
            let instr = format!(" NOTEREF mdfootnote_{id} \\h \\f ");
            xml.start("w:fldSimple", &[("w:instr", instr.as_str())])?;
            self.write_marked_run(xml, &id, plain_marks(24, "000000", false))?;
            xml.end("w:fldSimple")
        }
    }

    fn write_footnote_reference_run(
        &mut self,
        xml: &mut Xml,
        id: &str,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:r", &[])?;
        xml.start("w:rPr", &[])?;
        xml.empty("w:rStyle", &[("w:val", "FootnoteReference")])?;
        xml.empty("w:vertAlign", &[("w:val", "superscript")])?;
        xml.end("w:rPr")?;
        xml.empty("w:footnoteReference", &[("w:id", id)])?;
        xml.end("w:r")
    }

    fn write_cross_ref(
        &mut self,
        xml: &mut Xml,
        cross: &CrossRef,
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        let number = self
            .caption_numbers
            .get(&cross.bookmark)
            .copied()
            .unwrap_or(1);
        let cached = format!("{} {number}", self.caption_text(cross.kind));
        let instruction = format!(" REF {} \\h ", cross.bookmark);
        self.update_fields = true;
        self.write_simple_field(xml, &instruction, &cached, look)
    }

    fn write_simple_field(
        &mut self,
        xml: &mut Xml,
        instruction: &str,
        cached: &str,
        look: &Look,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:fldSimple", &[("w:instr", instruction)])?;
        self.write_marked_run(
            xml,
            cached,
            Marks {
                bold: look.force_bold,
                italic: look.force_italic,
                underline: false,
                strike: false,
                color: look.color.unwrap_or("000000"),
                size: look.size.unwrap_or(24),
                courier: false,
                shade: None,
            },
        )?;
        xml.end("w:fldSimple")
    }

    fn caption_text(&self, kind: CaptionKind) -> String {
        let custom = self
            .caption_options
            .as_ref()
            .and_then(|options| match kind {
                CaptionKind::Figure => options.figure_label.as_deref(),
                CaptionKind::Table => options.table_label.as_deref(),
            });
        match custom {
            Some(label) => label.to_string(),
            None => caption_label(kind).to_string(),
        }
    }

    fn caption_align_name(&self) -> &'static str {
        self.caption_options
            .as_ref()
            .and_then(|options| options.alignment)
            .map(alignment_name)
            .unwrap_or("center")
    }

    fn caption_marks(&self, size: u32) -> Marks {
        Marks {
            bold: false,
            italic: self
                .caption_options
                .as_ref()
                .and_then(|options| options.italic)
                .unwrap_or(false),
            underline: false,
            strike: false,
            color: "000000",
            size,
            courier: false,
            shade: None,
        }
    }

    fn caption_size(&self) -> u32 {
        if let Some(size) = self
            .caption_options
            .as_ref()
            .and_then(|options| options.size)
        {
            return size.max(1);
        }
        self.active_style
            .as_ref()
            .and_then(|style| style.paragraph_size)
            .unwrap_or(24)
            .max(1)
    }

    fn caption_spacing(&self) -> (u32, u32, u32) {
        let Some(style) = &self.active_style else {
            return (120, 240, 276);
        };
        let after = style.paragraph_spacing;
        let line = (style.line_spacing * 240.0).round() as u32;
        (after / 2, after, line.max(1))
    }

    fn write_image_block(
        &mut self,
        xml: &mut Xml,
        image: &Image,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        if let Some(quote) = quote {
            xml.start("w:p", &[])?;
            xml.start("w:pPr", &[])?;
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
            xml.end("w:pPr")?;
            self.write_pending_ref(xml)?;
            self.write_image_value(xml, image)?;
            return xml.end("w:p");
        }
        let (before, after, line) = self.body_spacing();
        let align = self.body_align();
        let bidi = self.bidi_val();
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:bidi", &[("w:val", bidi)])?;
        spacing(xml, before, after, Some(line), None)?;
        xml.empty("w:jc", &[("w:val", align)])?;
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        self.write_image_value(xml, image)?;
        xml.end("w:p")
    }

    fn write_listed_image(
        &mut self,
        xml: &mut Xml,
        image: &Image,
        level: u32,
        num_id: u32,
        quote: Option<&QuoteCtx>,
        continuation: bool,
    ) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        if !continuation {
            let ilvl = level.to_string();
            let num = num_id.to_string();
            xml.empty("w:pStyle", &[("w:val", "ListParagraph")])?;
            xml.start("w:numPr", &[])?;
            xml.empty("w:ilvl", &[("w:val", &ilvl)])?;
            xml.empty("w:numId", &[("w:val", &num)])?;
            xml.end("w:numPr")?;
        }
        if let Some(quote) = quote {
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
        } else if continuation {
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 120, 120, None, None)?;
            indent(xml, 720 * (level + 1))?;
        } else {
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 120, 120, None, None)?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        self.write_image_value(xml, image)?;
        xml.end("w:p")
    }

    /// A loaded raster becomes a drawing and a `word/media` part. Missing bytes
    /// stay the red placeholder. SVG and WebP are already PNG when they arrive.
    fn write_image_value(
        &mut self,
        xml: &mut Xml,
        image: &Image,
    ) -> Result<(), MarkdownConversionError> {
        let Some(bytes) = image.bytes.as_deref().filter(|bytes| !bytes.is_empty()) else {
            return self.write_missing_image(xml, &image.alt);
        };
        let (hint_width, hint_height) = image_hints(&image.url);
        self.embed_raster(
            xml,
            image.alt.as_str(),
            image_title(image),
            bytes,
            &image.content_type,
            nonzero(image.width_px),
            nonzero(image.height_px),
            hint_width,
            hint_height,
        )
    }

    fn write_image(
        &mut self,
        xml: &mut Xml,
        url: &str,
        alt: &str,
        title: Option<&str>,
    ) -> Result<(), MarkdownConversionError> {
        let (hint_width, hint_height) = image_hints(url);
        let (bytes, _, content_type) = match decode_image(url) {
            Ok(image) => image,
            Err(err) if self.in_footnote => return Err(err),
            Err(_) => return self.write_missing_image(xml, alt),
        };
        self.embed_raster(
            xml,
            alt,
            title,
            &bytes,
            content_type,
            None,
            None,
            hint_width,
            hint_height,
        )
    }

    fn embed_raster(
        &mut self,
        xml: &mut Xml,
        alt: &str,
        title: Option<&str>,
        bytes: &[u8],
        content_type: &str,
        known_width: Option<u32>,
        known_height: Option<u32>,
        hint_width: Option<u32>,
        hint_height: Option<u32>,
    ) -> Result<(), MarkdownConversionError> {
        let sniffed = sniff_raster(bytes);
        let extension = sniffed
            .map(|raster| raster.extension)
            .unwrap_or_else(|| extension_for_content_type(content_type));
        self.store_and_draw(
            xml,
            alt,
            title,
            bytes,
            extension,
            hint_width,
            hint_height,
            sniffed,
            known_width,
            known_height,
        )
    }

    fn store_and_draw(
        &mut self,
        xml: &mut Xml,
        alt: &str,
        title: Option<&str>,
        bytes: &[u8],
        extension: &str,
        hint_width: Option<u32>,
        hint_height: Option<u32>,
        sniffed: Option<RasterKind>,
        known_width: Option<u32>,
        known_height: Option<u32>,
    ) -> Result<(), MarkdownConversionError> {
        if let Some((extension, content_type)) = static_media_type(extension) {
            if !self
                .media_types
                .iter()
                .any(|(existing, _)| *existing == extension)
            {
                self.media_types.push((extension, content_type));
            }
        }
        let name = format!("image{}.{extension}", self.media.len() + 1);
        let token = format!("rId{{{name}}}");
        self.media
            .push((format!("word/media/{name}"), bytes.to_vec()));
        let (width, height) = display_pixels(
            sniffed.and_then(|raster| raster.width).or(known_width),
            sniffed.and_then(|raster| raster.height).or(known_height),
            hint_width,
            hint_height,
        );
        self.write_drawing(xml, &token, alt, title, width, height)
    }

    fn write_drawing(
        &mut self,
        xml: &mut Xml,
        embed: &str,
        alt: &str,
        title: Option<&str>,
        width: u32,
        height: u32,
    ) -> Result<(), MarkdownConversionError> {
        self.drawing_id = self.drawing_id.saturating_add(1);
        let id = self.drawing_id.to_string();
        let name = drawing_name(alt, title);
        let cx = (u64::from(width) * EMU_PER_PIXEL).to_string();
        let cy = (u64::from(height) * EMU_PER_PIXEL).to_string();
        xml.start("w:r", &[])?;
        xml.start("w:drawing", &[])?;
        xml.start(
            "wp:inline",
            &[
                ("distT", "0"),
                ("distB", "0"),
                ("distL", "0"),
                ("distR", "0"),
                (
                    "xmlns:wp",
                    "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
                ),
            ],
        )?;
        xml.empty("wp:extent", &[("cx", cx.as_str()), ("cy", cy.as_str())])?;
        xml.empty(
            "wp:effectExtent",
            &[("l", "0"), ("t", "0"), ("r", "0"), ("b", "0")],
        )?;
        let mut doc_pr = vec![("id", id.as_str()), ("name", name.as_str())];
        if !alt.is_empty() {
            doc_pr.push(("descr", alt));
        }
        if let Some(title) = title.filter(|text| !text.is_empty()) {
            doc_pr.push(("title", title));
        }
        xml.empty("wp:docPr", &doc_pr)?;
        xml.start("wp:cNvGraphicFramePr", &[])?;
        xml.empty(
            "a:graphicFrameLocks",
            &[
                (
                    "xmlns:a",
                    "http://schemas.openxmlformats.org/drawingml/2006/main",
                ),
                ("noChangeAspect", "1"),
            ],
        )?;
        xml.end("wp:cNvGraphicFramePr")?;
        xml.start(
            "a:graphic",
            &[(
                "xmlns:a",
                "http://schemas.openxmlformats.org/drawingml/2006/main",
            )],
        )?;
        xml.start(
            "a:graphicData",
            &[(
                "uri",
                "http://schemas.openxmlformats.org/drawingml/2006/picture",
            )],
        )?;
        xml.start(
            "pic:pic",
            &[(
                "xmlns:pic",
                "http://schemas.openxmlformats.org/drawingml/2006/picture",
            )],
        )?;
        xml.start("pic:nvPicPr", &[])?;
        let mut picture_name = vec![("id", "0"), ("name", name.as_str())];
        if !alt.is_empty() {
            picture_name.push(("descr", alt));
        }
        xml.empty("pic:cNvPr", &picture_name)?;
        xml.start("pic:cNvPicPr", &[])?;
        xml.empty(
            "a:picLocks",
            &[("noChangeAspect", "1"), ("noChangeArrowheads", "1")],
        )?;
        xml.end("pic:cNvPicPr")?;
        xml.end("pic:nvPicPr")?;
        xml.start("pic:blipFill", &[])?;
        xml.empty("a:blip", &[("r:embed", embed), ("cstate", "none")])?;
        xml.start("a:stretch", &[])?;
        xml.empty("a:fillRect", &[])?;
        xml.end("a:stretch")?;
        xml.end("pic:blipFill")?;
        xml.start("pic:spPr", &[])?;
        xml.start("a:xfrm", &[])?;
        xml.empty("a:off", &[("x", "0"), ("y", "0")])?;
        xml.empty("a:ext", &[("cx", cx.as_str()), ("cy", cy.as_str())])?;
        xml.end("a:xfrm")?;
        xml.start("a:prstGeom", &[("prst", "rect")])?;
        xml.empty("a:avLst", &[])?;
        xml.end("a:prstGeom")?;
        xml.end("pic:spPr")?;
        xml.end("pic:pic")?;
        xml.end("a:graphicData")?;
        xml.end("a:graphic")?;
        xml.end("wp:inline")?;
        xml.end("w:drawing")?;
        xml.end("w:r")
    }

    fn write_missing_image(
        &mut self,
        xml: &mut Xml,
        alt: &str,
    ) -> Result<(), MarkdownConversionError> {
        let text = image_placeholder(alt);
        self.write_marked_run(
            xml,
            &text,
            Marks {
                bold: false,
                italic: true,
                underline: false,
                strike: false,
                color: "FF0000",
                size: self.caption_size(),
                courier: false,
                shade: None,
            },
        )
    }

    fn write_page_break(&mut self, xml: &mut Xml) -> Result<(), MarkdownConversionError> {
        xml.start("w:p", &[])?;
        self.write_pending_ref(xml)?;
        xml.start("w:r", &[])?;
        xml.empty("w:br", &[("w:type", "page")])?;
        xml.end("w:r")?;
        xml.end("w:p")
    }

    fn write_comment(
        &mut self,
        xml: &mut Xml,
        text: &str,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        let label = format!("Comment: {text}");
        let marks = Marks {
            bold: false,
            italic: true,
            underline: false,
            strike: false,
            color: "666666",
            size: 24,
            courier: false,
            shade: None,
        };
        xml.start("w:p", &[])?;
        xml.start("w:pPr", &[])?;
        if let Some(quote) = quote {
            write_quote_frame(xml, quote)?;
            spacing(xml, 240, 240, None, None)?;
            indent(xml, 720 * quote.level)?;
        } else {
            xml.empty("w:bidi", &[("w:val", "false")])?;
            spacing(xml, 240, 240, Some(276), None)?;
            xml.empty("w:jc", &[("w:val", "left")])?;
        }
        xml.end("w:pPr")?;
        self.write_pending_ref(xml)?;
        self.write_marked_run(xml, &label, marks)?;
        xml.end("w:p")
    }

    fn write_footnote_heading(
        &mut self,
        xml: &mut Xml,
        heading: &Heading,
        quote: Option<&QuoteCtx>,
    ) -> Result<(), MarkdownConversionError> {
        if let Some(quote) = quote {
            self.write_quote_paragraph(xml, &heading.children, quote, false)
        } else {
            self.write_body_paragraph(xml, &heading.children, Look::body())
        }
    }

    fn write_flat_table(
        &mut self,
        xml: &mut Xml,
        table: &Table,
    ) -> Result<(), MarkdownConversionError> {
        let mut rows: Vec<&[TableCell]> = Vec::new();
        if !table.header.is_empty() {
            rows.push(table.header.as_slice());
        }
        for row in &table.rows {
            rows.push(row.as_slice());
        }
        if rows.is_empty() {
            return self.write_body_paragraph(xml, &[], Look::body());
        }
        for row in rows {
            let text = row
                .iter()
                .map(|cell| inline_text(&cell.children))
                .collect::<Vec<_>>()
                .join(" | ");
            self.write_body_paragraph(xml, &[plain_inline(text)], Look::body())?;
        }
        Ok(())
    }

    fn write_footnotes(
        &mut self,
        xml_footnotes: &[(String, &[Block])],
    ) -> Result<Vec<u8>, MarkdownConversionError> {
        self.in_footnote = true;
        let mut xml = Xml::new()?;
        xml.start(
            "w:footnotes",
            &[("xmlns:w", W_NS), ("xmlns:r", R_NS), ("xmlns:m", MATH_NS)],
        )?;
        write_separator_footnote(&mut xml, "-1", "separator", "w:separator")?;
        write_separator_footnote(
            &mut xml,
            "0",
            "continuationSeparator",
            "w:continuationSeparator",
        )?;
        for (index, (_id, children)) in xml_footnotes.iter().enumerate() {
            let id = (index + 1).to_string();
            xml.start("w:footnote", &[("w:id", id.as_str())])?;
            self.pending_footnote_ref = true;
            if children.is_empty() {
                self.write_body_paragraph(&mut xml, &[], Look::body())?;
            } else {
                for child in *children {
                    self.write_block(&mut xml, child, None)?;
                }
            }
            if self.pending_footnote_ref {
                self.write_body_paragraph(&mut xml, &[], Look::body())?;
            }
            xml.end("w:footnote")?;
        }
        xml.end("w:footnotes")?;
        self.in_footnote = false;
        Ok(xml.finish())
    }
}

#[derive(Clone, Copy)]
struct Marks {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color: &'static str,
    size: u32,
    courier: bool,
    shade: Option<&'static str>,
}

fn plain_marks(size: u32, color: &'static str, bold: bool) -> Marks {
    Marks {
        bold,
        italic: false,
        underline: false,
        strike: false,
        color,
        size,
        courier: false,
        shade: None,
    }
}

fn run_marks(run: &Inline, look: &Look) -> Marks {
    let linked = matches!(link_kind(run.link.as_deref()), LinkKind::External(_));
    Marks {
        bold: look.force_bold || run.bold,
        italic: look.force_italic || run.italic,
        underline: linked || run.underline,
        strike: run.strike,
        color: if linked {
            "0000FF"
        } else {
            look.color.unwrap_or("000000")
        },
        size: look.size.unwrap_or(24),
        courier: false,
        shade: None,
    }
}

fn heading_size(level: u8) -> u32 {
    match level {
        1 => 32,
        2 => 28,
        3 => 24,
        4 => 20,
        5 => 16,
        _ => 12,
    }
}

fn copy_callout(callout: &CalloutKind) -> CalloutKind {
    match callout {
        CalloutKind::Note => CalloutKind::Note,
        CalloutKind::Tip => CalloutKind::Tip,
        CalloutKind::Important => CalloutKind::Important,
        CalloutKind::Warning => CalloutKind::Warning,
        CalloutKind::Caution => CalloutKind::Caution,
    }
}

fn callout_label(callout: Option<&CalloutKind>) -> Option<&'static str> {
    match callout {
        Some(CalloutKind::Note) => Some("Note"),
        Some(CalloutKind::Tip) => Some("Tip"),
        Some(CalloutKind::Important) => Some("Important"),
        Some(CalloutKind::Warning) => Some("Warning"),
        Some(CalloutKind::Caution) => Some("Caution"),
        None => None,
    }
}

fn callout_title(callout: Option<&CalloutKind>) -> &'static str {
    match callout {
        Some(CalloutKind::Note) => "0969DA",
        Some(CalloutKind::Tip) => "1A7F37",
        Some(CalloutKind::Important) => "8250DF",
        Some(CalloutKind::Warning) => "9A6700",
        Some(CalloutKind::Caution) => "CF222E",
        None => "000000",
    }
}

fn callout_fill(callout: Option<&CalloutKind>) -> Option<&'static str> {
    match callout {
        Some(CalloutKind::Note) => Some("EFF6FF"),
        Some(CalloutKind::Tip) => Some("F0FFF4"),
        Some(CalloutKind::Important) => Some("F6F0FF"),
        Some(CalloutKind::Warning) => Some("FFF8C5"),
        Some(CalloutKind::Caution) => Some("FFF1F1"),
        None => None,
    }
}

fn callout_border_color(callout: Option<&CalloutKind>) -> &'static str {
    match callout {
        Some(CalloutKind::Note) => "0969DA",
        Some(CalloutKind::Tip) => "1A7F37",
        Some(CalloutKind::Important) => "8250DF",
        Some(CalloutKind::Warning) => "BF8700",
        Some(CalloutKind::Caution) => "CF222E",
        None => "AAAAAA",
    }
}

fn quote_border(quote: Option<&QuoteCtx>) -> &'static str {
    callout_border_color(quote.and_then(|quote| quote.callout.as_ref()))
}

fn write_quote_frame(xml: &mut Xml, quote: &QuoteCtx) -> Result<(), MarkdownConversionError> {
    xml.start("w:pBdr", &[])?;
    xml.empty(
        "w:left",
        &[
            ("w:val", "single"),
            ("w:color", callout_border_color(quote.callout.as_ref())),
            ("w:sz", "3"),
        ],
    )?;
    xml.end("w:pBdr")?;
    if let Some(fill) = callout_fill(quote.callout.as_ref()) {
        xml.empty("w:shd", &[("w:fill", fill)])?;
    }
    xml.empty("w:bidi", &[("w:val", "false")])
}

/// Blank paragraph between back-to-back code blocks so their borders stay apart.
fn write_code_spacer(xml: &mut Xml) -> Result<(), MarkdownConversionError> {
    xml.start("w:p", &[])?;
    xml.start("w:pPr", &[])?;
    spacing(xml, 0, 0, None, None)?;
    xml.end("w:pPr")?;
    xml.end("w:p")
}

fn spacing(
    xml: &mut Xml,
    before: u32,
    after: u32,
    line: Option<u32>,
    line_rule: Option<&str>,
) -> Result<(), MarkdownConversionError> {
    let before = before.to_string();
    let after = after.to_string();
    let line_value = line.map(|value| value.to_string());
    let mut attrs = vec![("w:before", before.as_str()), ("w:after", after.as_str())];
    if let Some(line_value) = line_value.as_deref() {
        attrs.push(("w:line", line_value));
    }
    if let Some(line_rule) = line_rule {
        attrs.push(("w:lineRule", line_rule));
    }
    xml.empty("w:spacing", &attrs)
}

fn indent(xml: &mut Xml, left: u32) -> Result<(), MarkdownConversionError> {
    let left = left.to_string();
    xml.empty("w:ind", &[("w:left", &left)])
}

fn courier(xml: &mut Xml) -> Result<(), MarkdownConversionError> {
    xml.empty(
        "w:rFonts",
        &[
            ("w:ascii", "Courier New"),
            ("w:cs", "Courier New"),
            ("w:eastAsia", "Courier New"),
            ("w:hAnsi", "Courier New"),
        ],
    )
}

fn jc_val(align: Option<TableAlign>) -> &'static str {
    match align {
        Some(TableAlign::Center) => "center",
        Some(TableAlign::Right) => "right",
        Some(TableAlign::Left) | Some(TableAlign::None) | None => "left",
    }
}

struct CodeChrome {
    label_color: String,
    plain_color: String,
    fill: String,
    border: String,
    show_label: bool,
}

struct HighlightPaint {
    text: String,
    color: Option<String>,
}

/// Plain code colors. Highlight spans, when the parser supplies them, replace
/// `plain_color` per token. Missing spans leave the grey code color in place.
fn code_chrome(code: &CodeBlock) -> CodeChrome {
    if let Some(colors) = &code.colors {
        return CodeChrome {
            label_color: colors.label_color.clone(),
            plain_color: colors.plain_color.clone(),
            fill: colors.fill.clone(),
            border: colors.border.clone(),
            show_label: colors.show_label,
        };
    }
    CodeChrome {
        label_color: "666666".to_string(),
        plain_color: "444444".to_string(),
        fill: "F5F5F5".to_string(),
        border: "DDDDDD".to_string(),
        show_label: true,
    }
}

fn code_highlights(code: &CodeBlock) -> Vec<HighlightPaint> {
    if code.highlights.is_empty() || code.highlights.iter().all(|span| span.color.is_none()) {
        return Vec::new();
    }
    code.highlights
        .iter()
        .map(|span| HighlightPaint {
            text: span.text.clone(),
            color: span.color.clone(),
        })
        .collect()
}

#[derive(Clone, Copy)]
enum CellMerge {
    None,
    Restart,
    Continue,
}

struct PlacedCell<'a> {
    source: Option<&'a TableCell>,
    column: usize,
    column_span: u32,
    merge: CellMerge,
}

struct RowCover {
    end: usize,
    span: u32,
}

/// Logical cells, including `vMerge` continuations for a row span.
fn place_table(table: &Table) -> Vec<(bool, Vec<PlacedCell<'_>>)> {
    let mut source: Vec<(bool, &[TableCell])> = Vec::new();
    if !table.header.is_empty() {
        source.push((true, table.header.as_slice()));
    }
    for row in &table.rows {
        source.push((false, row.as_slice()));
    }
    let mut occupied: BTreeMap<usize, RowCover> = BTreeMap::new();
    let mut placed = Vec::new();
    for (row_index, (header, cells)) in source.into_iter().enumerate() {
        let mut column = 0usize;
        let mut out = Vec::new();
        let mut pending = cells.iter();
        let mut next = pending.next();
        loop {
            if let Some(cover) = occupied.get(&column).filter(|cover| cover.end > row_index) {
                let span = cover.span.max(1);
                out.push(PlacedCell {
                    source: None,
                    column,
                    column_span: span,
                    merge: CellMerge::Continue,
                });
                column += span as usize;
                continue;
            }
            let Some(cell) = next else {
                break;
            };
            next = pending.next();
            let (column_span, row_span) = cell_spans(cell);
            let column_span = column_span.max(1);
            let row_span = row_span.max(1);
            let merge = if row_span > 1 {
                CellMerge::Restart
            } else {
                CellMerge::None
            };
            out.push(PlacedCell {
                source: Some(cell),
                column,
                column_span,
                merge,
            });
            if row_span > 1 {
                occupied.insert(
                    column,
                    RowCover {
                        end: row_index + row_span as usize,
                        span: column_span,
                    },
                );
            }
            column += column_span as usize;
        }
        while let Some(cover) = occupied.get(&column).filter(|cover| cover.end > row_index) {
            let span = cover.span.max(1);
            out.push(PlacedCell {
                source: None,
                column,
                column_span: span,
                merge: CellMerge::Continue,
            });
            column += span as usize;
        }
        placed.push((header, out));
    }
    placed
}

fn cell_spans(cell: &TableCell) -> (u32, u32) {
    (cell.column_span.max(1), cell.row_span.max(1))
}

fn column_widths(table: &Table) -> Option<&[u32]> {
    table.column_widths.as_deref()
}

/// `true` keeps the historical `w:cantSplit w:val="false"`.
fn table_row_split(_table: &Table) -> bool {
    true
}

fn leading_nbsp(line: &str) -> String {
    let lead = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count();
    let mut out = "\u{00A0}".repeat(lead);
    out.push_str(&line[lead..]);
    out
}

enum LinkKind<'a> {
    External(&'a str),
    Anchor(String),
    Plain,
}

fn link_kind(link: Option<&str>) -> LinkKind<'_> {
    let Some(link) = link else {
        return LinkKind::Plain;
    };
    if let Some(fragment) = link.strip_prefix('#') {
        return LinkKind::Anchor(percent_decode(fragment));
    }
    match scheme(link) {
        Some(scheme) if is_safe_scheme(scheme) => LinkKind::External(link),
        Some(_) => LinkKind::Plain,
        None => LinkKind::External(link),
    }
}

fn scheme(url: &str) -> Option<&str> {
    let bytes = url.as_bytes();
    if !bytes.first().is_some_and(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut end = 1;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric()
            || bytes[end] == b'+'
            || bytes[end] == b'-'
            || bytes[end] == b'.')
    {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b':' {
        Some(&url[..end])
    } else {
        None
    }
}

fn is_safe_scheme(scheme: &str) -> bool {
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto" | "tel"
    )
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[index + 1]), hex_val(bytes[index + 2])) {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn remember_link(links: &mut Vec<String>, target: &str, first_id: usize) -> String {
    if let Some(index) = links.iter().position(|link| link == target) {
        return format!("rId{}", index + first_id);
    }
    links.push(target.to_string());
    format!("rId{}", links.len() + first_id - 1)
}

fn plain_inline(text: String) -> Inline {
    Inline {
        text,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        code: false,
        link: None,
        footnote: None,
        math_omml: None,
        cross_ref: None,
    }
}

fn inline_text(runs: &[Inline]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn gather_footnotes(blocks: &[Block]) -> Vec<(String, &[Block])> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    gather_footnote_blocks(blocks, &mut seen, &mut found);
    found
}

fn gather_footnote_blocks<'a>(
    blocks: &'a [Block],
    seen: &mut BTreeSet<String>,
    found: &mut Vec<(String, &'a [Block])>,
) {
    for block in blocks {
        match block {
            Block::FootnoteDefinition { id, children } => {
                let key = id.to_string();
                if seen.insert(key.clone()) {
                    found.push((key, children.as_slice()));
                }
                gather_footnote_blocks(children, seen, found);
            }
            Block::List(list) => {
                for item in &list.children {
                    gather_footnote_blocks(&item.children, seen, found);
                }
            }
            Block::Blockquote(quote) => gather_footnote_blocks(&quote.children, seen, found),
            Block::Paragraph(_)
            | Block::Heading(_)
            | Block::CodeBlock(_)
            | Block::Table(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Caption(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. } => {}
        }
    }
}

fn write_separator_footnote(
    xml: &mut Xml,
    id: &str,
    kind: &str,
    mark: &str,
) -> Result<(), MarkdownConversionError> {
    xml.start("w:footnote", &[("w:type", kind), ("w:id", id)])?;
    xml.start("w:p", &[])?;
    xml.start("w:pPr", &[])?;
    xml.empty(
        "w:spacing",
        &[("w:after", "0"), ("w:line", "240"), ("w:lineRule", "auto")],
    )?;
    xml.end("w:pPr")?;
    xml.start("w:r", &[])?;
    xml.empty(mark, &[])?;
    xml.end("w:r")?;
    xml.end("w:p")?;
    xml.end("w:footnote")
}

fn write_content_types(
    has_footnotes: bool,
    media_types: &[(&str, &str)],
    stories: &[StoryPart],
    has_settings: bool,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Types", &[("xmlns", CT_NS)])?;
    xml.empty(
        "Default",
        &[
            ("Extension", "rels"),
            (
                "ContentType",
                "application/vnd.openxmlformats-package.relationships+xml",
            ),
        ],
    )?;
    xml.empty(
        "Default",
        &[("Extension", "xml"), ("ContentType", "application/xml")],
    )?;
    for (extension, content_type) in media_types {
        xml.empty(
            "Default",
            &[("Extension", extension), ("ContentType", content_type)],
        )?;
    }
    xml.empty(
        "Override",
        &[
            ("PartName", "/word/document.xml"),
            (
                "ContentType",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            ),
        ],
    )?;
    xml.empty(
        "Override",
        &[
            ("PartName", "/word/styles.xml"),
            (
                "ContentType",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
            ),
        ],
    )?;
    xml.empty(
        "Override",
        &[
            ("PartName", "/word/numbering.xml"),
            (
                "ContentType",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
            ),
        ],
    )?;
    if has_footnotes {
        xml.empty(
            "Override",
            &[
                ("PartName", "/word/footnotes.xml"),
                (
                    "ContentType",
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml",
                ),
            ],
        )?;
    }
    if has_settings {
        xml.empty(
            "Override",
            &[
                ("PartName", "/word/settings.xml"),
                (
                    "ContentType",
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
                ),
            ],
        )?;
    }
    for story in stories {
        let Some(content_type) = story.content_type else {
            continue;
        };
        let part_name = format!("/{}", story.path);
        xml.empty(
            "Override",
            &[("PartName", &part_name), ("ContentType", content_type)],
        )?;
    }
    xml.empty(
        "Override",
        &[
            ("PartName", "/docProps/core.xml"),
            ("ContentType", CORE_CONTENT_TYPE),
        ],
    )?;
    xml.end("Types")?;
    Ok(xml.finish())
}

fn write_package_rels() -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Relationships", &[("xmlns", REL_NS)])?;
    xml.empty(
        "Relationship",
        &[
            ("Id", "rId1"),
            (
                "Type",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
            ),
            ("Target", "word/document.xml"),
        ],
    )?;
    xml.empty(
        "Relationship",
        &[
            ("Id", "rId2"),
            ("Type", CORE_REL),
            ("Target", "docProps/core.xml"),
        ],
    )?;
    xml.end("Relationships")?;
    Ok(xml.finish())
}

const CORE_NS: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const DCTERMS_NS: &str = "http://purl.org/dc/terms/";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const APP_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const CUSTOM_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";
const VT_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";
const CUSTOM_FMTID: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";
const CORE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
const APP_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.extended-properties+xml";
const CUSTOM_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.custom-properties+xml";
const CORE_REL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const APP_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const CUSTOM_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";

/// Core, app, and custom document properties supplied by conversion options.
///
/// `custom: None` omits `docProps/custom.xml`. An empty list still writes the
/// part. Omitted timestamps are not filled in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageMetadata {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub keywords: Option<String>,
    pub category: Option<String>,
    pub company: Option<String>,
    pub language: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub custom: Option<Vec<(String, String)>>,
}

/// `None` leaves the legacy core part alone. A set value, including an empty
/// object, rewrites core properties. App and custom parts are included only
/// when those fields are set.
fn package_metadata(
    options: &ConvertOptions,
) -> Result<Option<PackageMetadata>, MarkdownConversionError> {
    let Some(metadata) = &options.metadata else {
        return Ok(None);
    };
    Ok(Some(metadata_parts(metadata)?))
}

pub(crate) fn metadata_parts(
    metadata: &DocumentMetadata,
) -> Result<PackageMetadata, MarkdownConversionError> {
    let normalized = normalize_document_metadata(metadata)?;
    Ok(PackageMetadata {
        title: normalized.title,
        subject: normalized.subject,
        description: normalized.description,
        creator: normalized.creator,
        keywords: normalized.keywords,
        category: normalized.category,
        company: normalized.company,
        language: normalized.language,
        created: normalized.created,
        modified: normalized.modified,
        custom: normalized
            .custom
            .map(|properties| properties.into_iter().collect()),
    })
}

fn push_legacy_core(extras: &mut Vec<(String, Vec<u8>)>) -> Result<(), MarkdownConversionError> {
    extras.push((
        "docProps/core.xml".to_string(),
        write_legacy_core_properties()?,
    ));
    Ok(())
}

/// The `docx` library stamps `dc:creator` as `Un-named` and writes both
/// timestamps. The stamp is fixed so a repeated conversion stays the same.
fn write_legacy_core_properties() -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start(
        "cp:coreProperties",
        &[
            ("xmlns:cp", CORE_NS),
            ("xmlns:dc", DC_NS),
            ("xmlns:dcterms", DCTERMS_NS),
            ("xmlns:xsi", XSI_NS),
        ],
    )?;
    write_text_element(&mut xml, "dc:creator", Some("Un-named"))?;
    write_text_element(&mut xml, "cp:lastModifiedBy", Some("Un-named"))?;
    write_text_element(&mut xml, "cp:revision", Some("1"))?;
    write_timestamp(
        &mut xml,
        "dcterms:created",
        Some("1980-01-01T00:00:00.000Z"),
    )?;
    write_timestamp(
        &mut xml,
        "dcterms:modified",
        Some("1980-01-01T00:00:00.000Z"),
    )?;
    xml.end("cp:coreProperties")?;
    Ok(xml.finish())
}

/// Write core, app, and custom property parts when metadata was supplied.
pub(crate) fn install_package_metadata(parts: &mut Parts) -> Result<(), MarkdownConversionError> {
    let Some(metadata) = parts.metadata.clone() else {
        return Ok(());
    };
    let core = write_core_properties(&metadata)?;
    upsert_extra(parts, "docProps/core.xml", core);
    ensure_override(
        &mut parts.content_types,
        "/docProps/core.xml",
        CORE_CONTENT_TYPE,
    )?;
    ensure_package_relationship(&mut parts.rels, "docProps/core.xml", CORE_REL)?;
    if let Some(company) = &metadata.company {
        let app = write_app_properties(company)?;
        upsert_extra(parts, "docProps/app.xml", app);
        ensure_override(
            &mut parts.content_types,
            "/docProps/app.xml",
            APP_CONTENT_TYPE,
        )?;
        ensure_package_relationship(&mut parts.rels, "docProps/app.xml", APP_REL)?;
    }
    if let Some(custom) = &metadata.custom {
        let custom_xml = write_custom_properties(custom)?;
        upsert_extra(parts, "docProps/custom.xml", custom_xml);
        ensure_override(
            &mut parts.content_types,
            "/docProps/custom.xml",
            CUSTOM_CONTENT_TYPE,
        )?;
        ensure_package_relationship(&mut parts.rels, "docProps/custom.xml", CUSTOM_REL)?;
    }
    parts.metadata = None;
    Ok(())
}

fn write_core_properties(metadata: &PackageMetadata) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start(
        "cp:coreProperties",
        &[
            ("xmlns:cp", CORE_NS),
            ("xmlns:dc", DC_NS),
            ("xmlns:dcterms", DCTERMS_NS),
            ("xmlns:xsi", XSI_NS),
        ],
    )?;
    write_text_element(&mut xml, "dc:title", metadata.title.as_deref())?;
    write_text_element(&mut xml, "dc:subject", metadata.subject.as_deref())?;
    write_text_element(&mut xml, "dc:description", metadata.description.as_deref())?;
    write_text_element(&mut xml, "dc:creator", metadata.creator.as_deref())?;
    write_text_element(&mut xml, "cp:keywords", metadata.keywords.as_deref())?;
    write_text_element(&mut xml, "cp:category", metadata.category.as_deref())?;
    write_text_element(&mut xml, "dc:language", metadata.language.as_deref())?;
    write_timestamp(&mut xml, "dcterms:created", metadata.created.as_deref())?;
    write_timestamp(&mut xml, "dcterms:modified", metadata.modified.as_deref())?;
    xml.end("cp:coreProperties")?;
    Ok(xml.finish())
}

fn write_app_properties(company: &str) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Properties", &[("xmlns", APP_NS), ("xmlns:vt", VT_NS)])?;
    write_text_element(&mut xml, "Company", Some(company))?;
    xml.end("Properties")?;
    Ok(xml.finish())
}

fn write_custom_properties(
    custom: &[(String, String)],
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Properties", &[("xmlns", CUSTOM_NS), ("xmlns:vt", VT_NS)])?;
    let mut pid = 1_u32;
    for (name, value) in custom {
        pid = pid.saturating_add(1);
        let pid = pid.to_string();
        xml.start(
            "property",
            &[
                ("fmtid", CUSTOM_FMTID),
                ("pid", pid.as_str()),
                ("name", name),
            ],
        )?;
        xml.start("vt:lpwstr", &[])?;
        xml.plain_text(value)?;
        xml.end("vt:lpwstr")?;
        xml.end("property")?;
    }
    xml.end("Properties")?;
    Ok(xml.finish())
}

fn write_text_element(
    xml: &mut Xml,
    name: &str,
    value: Option<&str>,
) -> Result<(), MarkdownConversionError> {
    let Some(value) = value else {
        return Ok(());
    };
    xml.start(name, &[])?;
    xml.plain_text(value)?;
    xml.end(name)
}

fn write_timestamp(
    xml: &mut Xml,
    name: &str,
    value: Option<&str>,
) -> Result<(), MarkdownConversionError> {
    let Some(value) = value else {
        return Ok(());
    };
    let value = timestamp_w3cdtf(value);
    xml.start(name, &[("xsi:type", "dcterms:W3CDTF")])?;
    xml.plain_text(&value)?;
    xml.end(name)
}

fn upsert_extra(parts: &mut Parts, path: &str, bytes: Vec<u8>) {
    if let Some(slot) = parts.extras.iter_mut().find(|(name, _)| name == path) {
        slot.1 = bytes;
    } else {
        parts.extras.push((path.to_string(), bytes));
    }
}

fn ensure_override(
    content_types: &mut Vec<u8>,
    part_name: &str,
    content_type: &str,
) -> Result<(), MarkdownConversionError> {
    let current = std::str::from_utf8(content_types)
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    if current.contains(&format!("PartName=\"{part_name}\"")) {
        return Ok(());
    }
    let snippet = format!("<Override PartName=\"{part_name}\" ContentType=\"{content_type}\"/>");
    insert_before_close(content_types, "</Types>", &snippet)
}

fn ensure_package_relationship(
    rels: &mut Vec<u8>,
    target: &str,
    rel_type: &str,
) -> Result<(), MarkdownConversionError> {
    let current =
        std::str::from_utf8(rels).map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    if current.contains(rel_type) {
        return Ok(());
    }
    let id = next_package_relationship_id(current);
    let snippet = format!("<Relationship Id=\"rId{id}\" Type=\"{rel_type}\" Target=\"{target}\"/>");
    insert_before_close(rels, "</Relationships>", &snippet)
}

fn next_package_relationship_id(rels: &str) -> u32 {
    let mut max = 0_u32;
    let mut rest = rels;
    while let Some(index) = rest.find("Id=\"rId") {
        rest = &rest[index + 7..];
        let end = rest.find('"').unwrap_or(rest.len());
        if let Ok(number) = rest[..end].parse::<u32>() {
            max = max.max(number);
        }
        if end < rest.len() {
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    max.saturating_add(1)
}

fn insert_before_close(
    xml: &mut Vec<u8>,
    close: &str,
    snippet: &str,
) -> Result<(), MarkdownConversionError> {
    let mut text = String::from_utf8(std::mem::take(xml))
        .map_err(|err| MarkdownConversionError::new(err.to_string()))?;
    let Some(index) = text.rfind(close) else {
        *xml = text.into_bytes();
        return Err(MarkdownConversionError::new(format!(
            "Package part is missing {close}"
        )));
    };
    text.insert_str(index, snippet);
    *xml = text.into_bytes();
    Ok(())
}

/// `Date.toISOString()` form: millisecond precision and a `Z` suffix.
fn timestamp_w3cdtf(value: &str) -> String {
    let Some((year, month, day, hour, minute, second, millis, offset)) = parse_timestamp(value)
    else {
        return value.to_string();
    };
    let Some((year, month, day, hour, minute, second)) =
        shift_minutes(year, month, day, hour, minute, second, -offset)
    else {
        return value.to_string();
    };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn parse_timestamp(value: &str) -> Option<(i32, i32, i32, i32, i32, i32, i32, i32)> {
    let mut index = 0;
    let year = take_digits(value, &mut index, 4)?;
    take_char(value, &mut index, '-')?;
    let month = take_digits(value, &mut index, 2)?;
    take_char(value, &mut index, '-')?;
    let day = take_digits(value, &mut index, 2)?;
    take_char(value, &mut index, 'T')?;
    let hour = take_digits(value, &mut index, 2)?;
    take_char(value, &mut index, ':')?;
    let minute = take_digits(value, &mut index, 2)?;
    take_char(value, &mut index, ':')?;
    let second = take_digits(value, &mut index, 2)?;
    let millis = if take_char(value, &mut index, '.').is_some() {
        let start = index;
        while index < value.len() && value.as_bytes()[index].is_ascii_digit() {
            index += 1;
        }
        if index == start {
            return None;
        }
        let mut digits = value[start..index].to_string();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits[..3].parse::<i32>().ok()?
    } else {
        0
    };
    let offset = if take_char(value, &mut index, 'Z').is_some()
        || take_char(value, &mut index, 'z').is_some()
    {
        0
    } else {
        let sign = if take_char(value, &mut index, '+').is_some() {
            1
        } else if take_char(value, &mut index, '-').is_some() {
            -1
        } else {
            return None;
        };
        let offset_hour = take_digits(value, &mut index, 2)?;
        take_char(value, &mut index, ':')?;
        let offset_minute = take_digits(value, &mut index, 2)?;
        sign * (offset_hour * 60 + offset_minute)
    };
    if index != value.len()
        || !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
        || day < 1
        || day > days_in_month(year, month)
    {
        return None;
    }
    Some((year, month, day, hour, minute, second, millis, offset))
}

fn take_digits(value: &str, index: &mut usize, width: usize) -> Option<i32> {
    let end = index.checked_add(width)?;
    if end > value.len() {
        return None;
    }
    let text = &value[*index..end];
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    *index = end;
    text.parse().ok()
}

fn take_char(value: &str, index: &mut usize, expected: char) -> Option<()> {
    let next = value[*index..].chars().next()?;
    if next != expected {
        return None;
    }
    *index += expected.len_utf8();
    Some(())
}

fn shift_minutes(
    mut year: i32,
    mut month: i32,
    mut day: i32,
    mut hour: i32,
    mut minute: i32,
    second: i32,
    delta: i32,
) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let mut total = hour * 60 + minute + delta;
    while total < 0 {
        total += 24 * 60;
        shift_days(&mut year, &mut month, &mut day, -1)?;
    }
    while total >= 24 * 60 {
        total -= 24 * 60;
        shift_days(&mut year, &mut month, &mut day, 1)?;
    }
    hour = total / 60;
    minute = total % 60;
    Some((year, month, day, hour, minute, second))
}

fn shift_days(year: &mut i32, month: &mut i32, day: &mut i32, delta: i32) -> Option<()> {
    if delta > 0 {
        *day += 1;
        if *day > days_in_month(*year, *month) {
            *day = 1;
            *month += 1;
            if *month > 12 {
                *month = 1;
                *year += 1;
            }
        }
    } else {
        *day -= 1;
        if *day < 1 {
            *month -= 1;
            if *month < 1 {
                *month = 12;
                *year -= 1;
            }
            *day = days_in_month(*year, *month);
        }
    }
    (*year >= 0).then_some(())
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn write_document_rels(
    links: &[String],
    has_footnotes: bool,
    stories: &[StoryRel],
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Relationships", &[("xmlns", REL_NS)])?;
    xml.empty(
        "Relationship",
        &[
            ("Id", "rId1"),
            (
                "Type",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
            ),
            ("Target", "styles.xml"),
        ],
    )?;
    xml.empty(
        "Relationship",
        &[
            ("Id", "rId2"),
            (
                "Type",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            ),
            ("Target", "numbering.xml"),
        ],
    )?;
    for (index, target) in links.iter().enumerate() {
        let id = format!("rId{}", index + 3);
        xml.empty(
            "Relationship",
            &[
                ("Id", &id),
                (
                    "Type",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                ),
                ("Target", target),
                ("TargetMode", "External"),
            ],
        )?;
    }
    if has_footnotes {
        let id = format!("rId{}", links.len() + 3);
        xml.empty(
            "Relationship",
            &[
                ("Id", &id),
                (
                    "Type",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
                ),
                ("Target", "footnotes.xml"),
            ],
        )?;
    }
    for story in stories {
        xml.empty(
            "Relationship",
            &[
                ("Id", &story.id),
                ("Type", story.rel_type),
                ("Target", &story.target),
            ],
        )?;
    }
    let settings_id = format!(
        "rId{}",
        3 + links.len() + usize::from(has_footnotes) + stories.len()
    );
    xml.empty(
        "Relationship",
        &[
            ("Id", &settings_id),
            (
                "Type",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
            ),
            ("Target", "settings.xml"),
        ],
    )?;
    xml.end("Relationships")?;
    Ok(xml.finish())
}

fn write_footnote_rels(links: &[String]) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Relationships", &[("xmlns", REL_NS)])?;
    xml.empty(
        "Relationship",
        &[
            ("Id", "rId1"),
            (
                "Type",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            ),
            ("Target", "numbering.xml"),
        ],
    )?;
    for (index, target) in links.iter().enumerate() {
        let id = format!("rId{}", index + 2);
        xml.empty(
            "Relationship",
            &[
                ("Id", &id),
                (
                    "Type",
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
                ),
                ("Target", target),
                ("TargetMode", "External"),
            ],
        )?;
    }
    xml.end("Relationships")?;
    Ok(xml.finish())
}

fn write_styles(style: Option<&ResolvedStyle>) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("w:styles", &[("xmlns:w", W_NS)])?;
    xml.start("w:docDefaults", &[])?;
    xml.start("w:rPrDefault", &[])?;
    xml.start("w:rPr", &[])?;
    xml.empty("w:sz", &[("w:val", "24")])?;
    xml.empty("w:szCs", &[("w:val", "24")])?;
    write_proofing_language(&mut xml, style)?;
    xml.end("w:rPr")?;
    xml.end("w:rPrDefault")?;
    xml.start("w:pPrDefault", &[])?;
    xml.start("w:pPr", &[])?;
    xml.empty(
        "w:spacing",
        &[("w:after", "0"), ("w:line", "276"), ("w:lineRule", "auto")],
    )?;
    xml.end("w:pPr")?;
    xml.end("w:pPrDefault")?;
    xml.end("w:docDefaults")?;
    write_style(&mut xml, "Normal", "Normal", true, None)?;
    write_style(
        &mut xml,
        "ListParagraph",
        "List Paragraph",
        false,
        Some("Normal"),
    )?;
    let spacing = [
        (360, 240),
        (320, 160),
        (280, 120),
        (240, 120),
        (220, 100),
        (200, 100),
    ];
    for level in 1..=6 {
        let size = heading_size(level);
        let (before, after) = spacing[usize::from(level - 1)];
        let named = format!("Heading{level}");
        let display = format!("Heading {level}");
        let numeric = level.to_string();
        write_heading_style(
            &mut xml,
            &named,
            &display,
            size,
            before,
            after,
            level - 1,
            style,
        )?;
        write_heading_style(
            &mut xml,
            &numeric,
            &display,
            size,
            before,
            after,
            level - 1,
            style,
        )?;
    }
    write_footnote_reference_style(&mut xml)?;
    write_toc_heading_style(&mut xml)?;
    write_style(&mut xml, "Caption", "Caption", false, Some("Normal"))?;
    xml.end("w:styles")?;
    Ok(xml.finish())
}

fn write_style(
    xml: &mut Xml,
    id: &str,
    name: &str,
    default: bool,
    based_on: Option<&str>,
) -> Result<(), MarkdownConversionError> {
    let mut attrs = vec![("w:type", "paragraph"), ("w:styleId", id)];
    if default {
        attrs.push(("w:default", "1"));
    }
    xml.start("w:style", &attrs)?;
    xml.empty("w:name", &[("w:val", name)])?;
    if let Some(based_on) = based_on {
        xml.empty("w:basedOn", &[("w:val", based_on)])?;
    }
    xml.empty("w:qFormat", &[])?;
    xml.end("w:style")
}

fn write_toc_heading_style(xml: &mut Xml) -> Result<(), MarkdownConversionError> {
    xml.start(
        "w:style",
        &[("w:type", "paragraph"), ("w:styleId", "TOCHeading")],
    )?;
    xml.empty("w:name", &[("w:val", "TOC Heading")])?;
    xml.empty("w:basedOn", &[("w:val", "Normal")])?;
    xml.empty("w:qFormat", &[])?;
    xml.start("w:pPr", &[])?;
    xml.empty("w:spacing", &[("w:after", "240")])?;
    xml.empty("w:jc", &[("w:val", "center")])?;
    xml.end("w:pPr")?;
    xml.end("w:style")
}

struct TocEntryStyle {
    size: u32,
    bold: bool,
    italic: bool,
}

fn toc_entry_style(level: u8, paragraph_size: u32) -> TocEntryStyle {
    let steps = u32::from(level.saturating_sub(1));
    let size = paragraph_size
        .saturating_sub(steps.saturating_mul(2))
        .max(1);
    TocEntryStyle {
        size,
        bold: level == 1,
        italic: false,
    }
}

fn caption_label(kind: CaptionKind) -> &'static str {
    match kind {
        CaptionKind::Figure => "Figure",
        CaptionKind::Table => "Table",
    }
}

fn caption_sequence(kind: CaptionKind) -> &'static str {
    match kind {
        CaptionKind::Figure => "MdToDocxFigure",
        CaptionKind::Table => "MdToDocxTable",
    }
}

fn toc_instruction(min_depth: u8, max_depth: u8, page_numbers: bool) -> String {
    let pages = if page_numbers { "" } else { " \\n" };
    format!(" TOC \\o \"{min_depth}-{max_depth}\" \\h \\z{pages} ")
}

fn is_toc_paragraph(paragraph: &Paragraph) -> bool {
    if paragraph.children.iter().any(|run| run.footnote.is_some()) {
        return false;
    }
    inline_text(&paragraph.children).trim() == "[TOC]"
}

fn collect_headings(blocks: &[Block], headings: &mut Vec<TocHeading>) {
    for block in blocks {
        match block {
            Block::Heading(heading) => headings.push(TocHeading {
                text: inline_text(&heading.children),
                level: heading.level,
                bookmark: heading.bookmark.clone(),
            }),
            Block::List(list) => {
                for item in &list.children {
                    collect_headings(&item.children, headings);
                }
            }
            Block::Blockquote(quote) => collect_headings(&quote.children, headings),
            Block::Caption(_) | Block::FootnoteDefinition { .. } => {}
            _ => {}
        }
    }
}

fn write_settings(update_fields: bool) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("w:settings", &[("xmlns:w", W_NS)])?;
    xml.empty("w:evenAndOddHeaders", &[("w:val", "false")])?;
    if update_fields {
        xml.empty("w:updateFields", &[])?;
    }
    xml.end("w:settings")?;
    Ok(xml.finish())
}

fn empty_relationships() -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start("Relationships", &[("xmlns", REL_NS)])?;
    xml.end("Relationships")?;
    Ok(xml.finish())
}

fn write_page_number_story(
    root: &str,
    align: &str,
    display: &str,
    size: u32,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut xml = Xml::new()?;
    xml.start(
        root,
        &[("xmlns:w", W_NS), ("xmlns:r", R_NS), ("xmlns:m", MATH_NS)],
    )?;
    xml.start("w:p", &[])?;
    xml.start("w:pPr", &[])?;
    xml.empty("w:bidi", &[("w:val", "false")])?;
    xml.empty("w:jc", &[("w:val", align)])?;
    xml.end("w:pPr")?;
    xml.start("w:r", &[])?;
    let size = size.to_string();
    xml.start("w:rPr", &[])?;
    xml.empty("w:sz", &[("w:val", &size)])?;
    xml.empty("w:szCs", &[("w:val", &size)])?;
    xml.empty("w:rtl", &[("w:val", "false")])?;
    xml.end("w:rPr")?;
    write_page_number_fields(&mut xml, display)?;
    xml.end("w:r")?;
    xml.end("w:p")?;
    xml.end(root)?;
    Ok(xml.finish())
}

fn write_page_number_fields(xml: &mut Xml, display: &str) -> Result<(), MarkdownConversionError> {
    let fields: &[&str] = match display {
        "current" => &["PAGE"],
        "currentAndTotal" => &["PAGE", "NUMPAGES"],
        "currentAndSectionTotal" => &["PAGE", "SECTIONPAGES"],
        _ => &[],
    };
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            xml.text_elem(" / ")?;
        }
        xml.empty("w:fldChar", &[("w:fldCharType", "begin")])?;
        xml.start("w:instrText", &[("xml:space", "preserve")])?;
        xml.text(field)?;
        xml.end("w:instrText")?;
        xml.empty("w:fldChar", &[("w:fldCharType", "separate")])?;
        xml.empty("w:fldChar", &[("w:fldCharType", "end")])?;
    }
    Ok(())
}

fn write_footnote_reference_style(xml: &mut Xml) -> Result<(), MarkdownConversionError> {
    xml.start(
        "w:style",
        &[("w:type", "character"), ("w:styleId", "FootnoteReference")],
    )?;
    xml.empty("w:name", &[("w:val", "footnote reference")])?;
    xml.start("w:rPr", &[])?;
    xml.empty("w:vertAlign", &[("w:val", "superscript")])?;
    xml.end("w:rPr")?;
    xml.end("w:style")
}

fn write_heading_style(
    xml: &mut Xml,
    id: &str,
    name: &str,
    size: u32,
    before: u32,
    after: u32,
    outline: u8,
    style: Option<&ResolvedStyle>,
) -> Result<(), MarkdownConversionError> {
    xml.start("w:style", &[("w:type", "paragraph"), ("w:styleId", id)])?;
    xml.empty("w:name", &[("w:val", name)])?;
    xml.empty("w:basedOn", &[("w:val", "Normal")])?;
    xml.empty("w:next", &[("w:val", "Normal")])?;
    xml.empty("w:qFormat", &[])?;
    xml.start("w:pPr", &[])?;
    spacing(xml, before, after, None, None)?;
    let outline = outline.to_string();
    xml.empty("w:outlineLvl", &[("w:val", &outline)])?;
    xml.end("w:pPr")?;
    xml.start("w:rPr", &[])?;
    xml.empty("w:b", &[])?;
    xml.empty("w:bCs", &[])?;
    xml.empty("w:color", &[("w:val", "000000")])?;
    let size = size.to_string();
    xml.empty("w:sz", &[("w:val", &size)])?;
    xml.empty("w:szCs", &[("w:val", &size)])?;
    write_proofing_language(xml, style)?;
    xml.end("w:rPr")?;
    xml.end("w:style")
}

/// `w:lang` from the style the parser already resolved.
///
/// East Asian tags add `w:eastAsia`. Right-to-left adds `w:bidi`. A style with
/// no language leaves the element out.
fn write_proofing_language(
    xml: &mut Xml,
    style: Option<&ResolvedStyle>,
) -> Result<(), MarkdownConversionError> {
    let Some(style) = style else {
        return Ok(());
    };
    let Some(language) = style.language.as_deref().filter(|tag| !tag.is_empty()) else {
        return Ok(());
    };
    let run = run_language(language, style.direction);
    let mut attrs = vec![("w:val", run.value.as_str())];
    if let Some(east_asia) = run.east_asia.as_deref() {
        attrs.push(("w:eastAsia", east_asia));
    }
    if let Some(bidirectional) = run.bidirectional.as_deref() {
        attrs.push(("w:bidi", bidirectional));
    }
    xml.empty("w:lang", &attrs)
}

fn write_numbering(starts: &[(u32, u64)]) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut ordered = starts.to_vec();
    ordered.sort_by_key(|(sequence, _)| *sequence);
    let mut xml = Xml::new()?;
    xml.start("w:numbering", &[("xmlns:w", W_NS)])?;
    write_abstract(&mut xml, 1, true, 1)?;
    for (sequence, start) in &ordered {
        write_abstract(&mut xml, sequence.saturating_add(1), false, *start)?;
    }
    write_num(&mut xml, 1, 1, 1)?;
    for (sequence, start) in &ordered {
        let id = sequence.saturating_add(1);
        write_num(&mut xml, id, id, *start)?;
    }
    xml.end("w:numbering")?;
    Ok(xml.finish())
}

fn write_abstract(
    xml: &mut Xml,
    id: u32,
    bullet: bool,
    start: u64,
) -> Result<(), MarkdownConversionError> {
    let id = id.to_string();
    xml.start("w:abstractNum", &[("w:abstractNumId", &id)])?;
    xml.empty("w:multiLevelType", &[("w:val", "hybridMultilevel")])?;
    for level in 0..9 {
        let ilvl = level.to_string();
        let left = (720 * (level + 1)).to_string();
        let hanging = if bullet { "360" } else { "260" };
        let start_value = start.to_string();
        let text = if bullet {
            BULLETS[level].to_string()
        } else {
            format!("%{}.", level + 1)
        };
        xml.start("w:lvl", &[("w:ilvl", &ilvl)])?;
        xml.empty("w:start", &[("w:val", &start_value)])?;
        xml.empty(
            "w:numFmt",
            &[("w:val", if bullet { "bullet" } else { "decimal" })],
        )?;
        xml.empty("w:lvlText", &[("w:val", &text)])?;
        xml.empty("w:lvlJc", &[("w:val", "left")])?;
        xml.start("w:pPr", &[])?;
        xml.empty("w:ind", &[("w:left", &left), ("w:hanging", hanging)])?;
        xml.end("w:pPr")?;
        xml.end("w:lvl")?;
    }
    xml.end("w:abstractNum")
}

fn write_num(
    xml: &mut Xml,
    num_id: u32,
    abstract_id: u32,
    start: u64,
) -> Result<(), MarkdownConversionError> {
    let num_id = num_id.to_string();
    let abstract_id = abstract_id.to_string();
    let start = start.to_string();
    xml.start("w:num", &[("w:numId", &num_id)])?;
    xml.empty("w:abstractNumId", &[("w:val", &abstract_id)])?;
    xml.start("w:lvlOverride", &[("w:ilvl", "0")])?;
    xml.empty("w:startOverride", &[("w:val", &start)])?;
    xml.end("w:lvlOverride")?;
    xml.end("w:num")
}

fn insert_footnote_images(children: &mut Vec<Block>, urls: &[String]) {
    if urls.is_empty() || blocks_have_image(children) {
        return;
    }
    let runs = urls
        .iter()
        .map(|url| Inline {
            text: format!("{IMAGE_MARK}{url}"),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            code: false,
            link: None,
            footnote: None,
            math_omml: None,
            cross_ref: None,
        })
        .collect::<Vec<_>>();
    if let Some(Block::Paragraph(paragraph)) = children.last_mut() {
        let blank = paragraph
            .children
            .iter()
            .all(|run| run.text.is_empty() && run.footnote.is_none() && run.link.is_none());
        if blank {
            paragraph.children = runs;
            return;
        }
    }
    children.push(Block::Paragraph(Paragraph { children: runs }));
}

fn blocks_have_image(blocks: &[Block]) -> bool {
    blocks.iter().any(block_has_image)
}

fn block_has_image(block: &Block) -> bool {
    match block {
        Block::Image(_) => true,
        Block::Paragraph(paragraph) => paragraph.children.iter().any(|run| {
            run.text.starts_with(IMAGE_MARK)
                || run.text.starts_with("[Image could not be displayed")
        }),
        Block::Heading(heading) => heading.children.iter().any(|run| {
            run.text.starts_with(IMAGE_MARK)
                || run.text.starts_with("[Image could not be displayed")
        }),
        Block::Caption(caption) => caption.children.iter().any(|run| {
            run.text.starts_with(IMAGE_MARK)
                || run.text.starts_with("[Image could not be displayed")
        }),
        Block::List(list) => list
            .children
            .iter()
            .any(|item| blocks_have_image(&item.children)),
        Block::Blockquote(quote) => blocks_have_image(&quote.children),
        Block::FootnoteDefinition { children, .. } => blocks_have_image(children),
        Block::Table(table) => table
            .header
            .iter()
            .chain(table.rows.iter().flatten())
            .any(|cell| {
                cell.children.iter().any(|run| {
                    run.text.starts_with(IMAGE_MARK)
                        || run.text.starts_with("[Image could not be displayed")
                })
            }),
        Block::CodeBlock(_)
        | Block::ThematicBreak
        | Block::PageBreak
        | Block::Comment(_)
        | Block::DisplayMath { .. }
        | Block::ChartImage { .. } => false,
    }
}

/// Image URLs for each referenced footnote definition, in first-reference order.
fn footnote_image_groups(markdown: &str) -> Vec<Vec<String>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let mut order = Vec::new();
    let mut defined = BTreeSet::new();
    let mut images: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut ignore = 0_u32;
    for event in Parser::new_ext(markdown, options) {
        if ignore > 0 {
            match event {
                MdEvent::Start(Tag::FootnoteDefinition(_)) => ignore = ignore.saturating_add(1),
                MdEvent::End(TagEnd::FootnoteDefinition) => ignore = ignore.saturating_sub(1),
                MdEvent::Start(_) => ignore = ignore.saturating_add(1),
                MdEvent::End(_) => ignore = ignore.saturating_sub(1),
                _ => {}
            }
            continue;
        }
        match event {
            MdEvent::Start(Tag::FootnoteDefinition(label)) => {
                let id = normalize_footnote_label(&label);
                if defined.contains(&id) {
                    ignore = 1;
                    current = None;
                } else {
                    defined.insert(id.clone());
                    images.entry(id.clone()).or_default();
                    current = Some(id);
                }
            }
            MdEvent::End(TagEnd::FootnoteDefinition) => current = None,
            MdEvent::Start(Tag::Image { dest_url, .. }) => {
                if let Some(id) = &current {
                    images
                        .entry(id.clone())
                        .or_default()
                        .push(dest_url.to_string());
                }
            }
            MdEvent::FootnoteReference(label) => {
                let id = normalize_footnote_label(&label);
                if !order.contains(&id) {
                    order.push(id);
                }
            }
            _ => {}
        }
    }
    order
        .into_iter()
        .filter(|id| defined.contains(id))
        .map(|id| images.remove(&id).unwrap_or_default())
        .collect()
}

fn normalize_footnote_label(label: &str) -> String {
    label.trim().to_lowercase()
}

#[derive(Clone, Copy)]
struct RasterKind {
    extension: &'static str,
    content_type: &'static str,
    width: Option<u32>,
    height: Option<u32>,
}

const EMU_PER_PIXEL: u64 = 9525;
const MAX_IMAGE_DIMENSION: u32 = 10_000;

struct ImageLabel {
    alt: String,
    title: Option<String>,
}

fn emit_omml(xml: &mut Xml, omml: &str) -> Result<(), MarkdownConversionError> {
    let omml = omml.trim();
    if omml.is_empty() {
        return Ok(());
    }
    if omml.contains("<m:oMath") || omml.contains("<m:oMathPara") {
        xml.push_bytes(omml.as_bytes());
        return Ok(());
    }
    let wrapped = format!(r#"<m:oMath xmlns:m="{MATH_NS}">{omml}</m:oMath>"#);
    xml.push_bytes(wrapped.as_bytes());
    Ok(())
}

fn image_placeholder(alt: &str) -> String {
    if alt.is_empty() {
        "[Image could not be displayed]".to_string()
    } else {
        format!("[Image could not be displayed: {alt}]")
    }
}

fn extension_for_content_type(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpeg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

fn split_image_mark(rest: &str) -> (&str, &str, Option<&str>) {
    let mut parts = rest.splitn(3, IMAGE_MARK);
    let url = parts.next().unwrap_or("");
    let alt = parts.next().unwrap_or("");
    let title = parts.next().filter(|text| !text.is_empty());
    (url, alt, title)
}

/// Inline image runs keep the raster bytes and drop the Markdown alt and title.
/// Pair them with images in the source so `descr` and the drawing name survive.
fn stamp_image_labels(blocks: &mut [Block], markdown: &str) {
    let labels = image_labels(markdown);
    let mut labels = labels.into_iter();
    stamp_blocks(blocks, &mut labels);
}

fn image_labels(markdown: &str) -> Vec<ImageLabel> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let mut labels = Vec::new();
    let mut alt = String::new();
    let mut title: Option<String> = None;
    let mut depth = 0_u32;
    for event in Parser::new_ext(markdown, options) {
        if depth > 0 {
            match event {
                MdEvent::Start(_) => depth = depth.saturating_add(1),
                MdEvent::End(_) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        labels.push(ImageLabel {
                            alt: std::mem::take(&mut alt),
                            title: title.take().filter(|text| !text.is_empty()),
                        });
                    }
                }
                MdEvent::Text(text) | MdEvent::Code(text) => alt.push_str(&text),
                MdEvent::SoftBreak => alt.push(' '),
                MdEvent::HardBreak => alt.push('\n'),
                _ => {}
            }
            continue;
        }
        if let MdEvent::Start(Tag::Image {
            title: image_title, ..
        }) = event
        {
            title = if image_title.is_empty() {
                None
            } else {
                Some(image_title.to_string())
            };
            alt.clear();
            depth = 1;
        }
    }
    labels
}

fn stamp_blocks(blocks: &mut [Block], labels: &mut impl Iterator<Item = ImageLabel>) {
    for block in blocks {
        match block {
            Block::Image(_) => {
                let _ = labels.next();
            }
            Block::Paragraph(paragraph) => stamp_runs(&mut paragraph.children, labels),
            Block::Heading(heading) => stamp_runs(&mut heading.children, labels),
            Block::Caption(caption) => stamp_runs(&mut caption.children, labels),
            Block::List(list) => {
                for item in &mut list.children {
                    stamp_blocks(&mut item.children, labels);
                }
            }
            Block::Blockquote(quote) => stamp_blocks(&mut quote.children, labels),
            Block::FootnoteDefinition { children, .. } => stamp_blocks(children, labels),
            Block::Table(table) => {
                for cell in table
                    .header
                    .iter_mut()
                    .chain(table.rows.iter_mut().flatten())
                {
                    stamp_runs(&mut cell.children, labels);
                }
            }
            Block::CodeBlock(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. } => {}
        }
    }
}

fn stamp_runs(runs: &mut [Inline], labels: &mut impl Iterator<Item = ImageLabel>) {
    for run in runs {
        if let Some(rest) = run.text.strip_prefix(IMAGE_MARK) {
            let Some(label) = labels.next() else {
                continue;
            };
            if rest.contains(IMAGE_MARK) {
                continue;
            }
            let title = label.title.unwrap_or_default();
            run.text = format!(
                "{IMAGE_MARK}{rest}{IMAGE_MARK}{}{IMAGE_MARK}{title}",
                label.alt
            );
            continue;
        }
        if run.text.starts_with("[Image could not be displayed") {
            let _ = labels.next();
        }
    }
}

fn image_title(image: &Image) -> Option<&str> {
    image.title.as_deref().filter(|text| !text.is_empty())
}

fn drawing_name(alt: &str, title: Option<&str>) -> String {
    if let Some(title) = title.map(str::trim).filter(|text| !text.is_empty()) {
        return title.to_string();
    }
    let alt = alt.trim();
    if alt.is_empty() {
        "Image".to_string()
    } else {
        alt.to_string()
    }
}

fn static_media_type(extension: &str) -> Option<(&'static str, &'static str)> {
    match extension {
        "png" => Some(("png", "image/png")),
        "jpeg" => Some(("jpeg", "image/jpeg")),
        "gif" => Some(("gif", "image/gif")),
        "webp" => Some(("webp", "image/webp")),
        "bmp" => Some(("bmp", "image/bmp")),
        "bin" => Some(("bin", "application/octet-stream")),
        _ => None,
    }
}

fn image_hints(url: &str) -> (Option<u32>, Option<u32>) {
    let Some((_, fragment)) = url.rsplit_once('#') else {
        return (None, None);
    };
    if let Some((width, height)) = fragment.split_once('x') {
        if let (Some(width), Some(height)) = (positive_u32(width), positive_u32(height)) {
            if !height_has_query(height_text(fragment)) {
                return (Some(width), Some(height));
            }
        }
    }
    let query = fragment.replace("&amp;", "&");
    let mut width = None;
    let mut height = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let parsed = positive_u32(value);
        match key {
            "w" | "width" => width = parsed,
            "h" | "height" => height = parsed,
            _ => {}
        }
    }
    (width, height)
}

fn height_text(fragment: &str) -> &str {
    fragment
        .split_once('x')
        .map(|(_, height)| height)
        .unwrap_or("")
}

fn height_has_query(height: &str) -> bool {
    height.contains('=') || height.contains('&')
}

fn positive_u32(text: &str) -> Option<u32> {
    let value = text.parse::<u32>().ok()?;
    (value > 0).then_some(value)
}

fn sniff_raster(bytes: &[u8]) -> Option<RasterKind> {
    if is_png(bytes) {
        let (width, height) = png_size(bytes);
        return Some(RasterKind {
            extension: "png",
            content_type: "image/png",
            width,
            height,
        });
    }
    if is_jpeg(bytes) {
        let (width, height) = jpeg_size(bytes);
        return Some(RasterKind {
            extension: "jpeg",
            content_type: "image/jpeg",
            width,
            height,
        });
    }
    if is_gif(bytes) {
        let (width, height) = gif_size(bytes);
        return Some(RasterKind {
            extension: "gif",
            content_type: "image/gif",
            width,
            height,
        });
    }
    None
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.len() >= 8
        && bytes[0] == 0x89
        && bytes[1] == 0x50
        && bytes[2] == 0x4e
        && bytes[3] == 0x47
        && bytes[4] == 0x0d
        && bytes[5] == 0x0a
        && bytes[6] == 0x1a
        && bytes[7] == 0x0a
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.len() >= 6
        && bytes[0] == b'G'
        && bytes[1] == b'I'
        && bytes[2] == b'F'
        && bytes[3] == b'8'
        && (bytes[4] == b'7' || bytes[4] == b'9')
        && bytes[5] == b'a'
}

fn png_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    if bytes.len() < 24 {
        return (None, None);
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (nonzero(width), nonzero(height))
}

fn gif_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    if bytes.len() < 10 {
        return (None, None);
    }
    let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    (nonzero(width), nonzero(height))
}

fn jpeg_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            break;
        }
        let marker = bytes[offset + 1];
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]);
        if marker == 0xc0 || marker == 0xc2 {
            let height = u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]));
            return (nonzero(width), nonzero(height));
        }
        let step = usize::from(length).saturating_add(2);
        if step == 0 {
            break;
        }
        offset = offset.saturating_add(step);
    }
    (None, None)
}

fn nonzero(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

fn display_pixels(
    intrinsic_width: Option<u32>,
    intrinsic_height: Option<u32>,
    hint_width: Option<u32>,
    hint_height: Option<u32>,
) -> (u32, u32) {
    let aspect = match (intrinsic_width, intrinsic_height) {
        (Some(width), Some(height)) => Some(f64::from(width) / f64::from(height)),
        _ => None,
    };
    let (width, height) = if let (Some(width), Some(height)) = (hint_width, hint_height) {
        (width, Some(height))
    } else if let (Some(width), Some(aspect)) = (hint_width, aspect) {
        (width, Some(round_positive(f64::from(width) / aspect)))
    } else if let (Some(height), Some(aspect)) = (hint_height, aspect) {
        (round_positive(f64::from(height) * aspect), Some(height))
    } else if let Some(width) = intrinsic_width {
        let width = width.min(400);
        let height = aspect.map(|aspect| round_positive(f64::from(width) / aspect));
        (width, height)
    } else {
        (200, None)
    };
    let width = clamp_dimension(width);
    let height = clamp_dimension(height.unwrap_or_else(|| round_positive(f64::from(width) * 0.75)));
    (width, height)
}

fn round_positive(value: f64) -> u32 {
    let rounded = value.round();
    if rounded < 1.0 {
        1
    } else if rounded > f64::from(u32::MAX) {
        u32::MAX
    } else {
        rounded as u32
    }
}

fn clamp_dimension(value: u32) -> u32 {
    value.clamp(1, MAX_IMAGE_DIMENSION)
}

fn decode_image(
    url: &str,
) -> Result<(Vec<u8>, &'static str, &'static str), MarkdownConversionError> {
    let rest = url
        .strip_prefix("data:")
        .ok_or_else(|| MarkdownConversionError::new("Footnote image is not a data URL"))?;
    let (meta, payload) = rest.split_once(',').ok_or_else(|| {
        MarkdownConversionError::new("Footnote image data URL is missing a payload")
    })?;
    if !meta.split(';').any(|part| part == "base64") {
        return Err(MarkdownConversionError::new(
            "Footnote image data URL is not base64",
        ));
    }
    let payload = payload
        .split_once('#')
        .map(|(body, _)| body)
        .unwrap_or(payload);
    let mime = meta.split(';').next().unwrap_or("");
    let (extension, content_type) = match mime {
        "image/png" => ("png", "image/png"),
        "image/jpeg" | "image/jpg" => ("jpeg", "image/jpeg"),
        "image/gif" => ("gif", "image/gif"),
        "image/webp" => ("webp", "image/webp"),
        "image/bmp" => ("bmp", "image/bmp"),
        _ => {
            return Err(MarkdownConversionError::new(
                "Footnote image type is not supported",
            ))
        }
    };
    let bytes = decode_base64(payload)?;
    if bytes.is_empty() {
        return Err(MarkdownConversionError::new("Footnote image data is empty"));
    }
    Ok((bytes, extension, content_type))
}

fn decode_base64(input: &str) -> Result<Vec<u8>, MarkdownConversionError> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let packed = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if packed.len() % 4 != 0 {
        return Err(MarkdownConversionError::new(
            "Footnote image base64 is truncated",
        ));
    }
    let mut out = Vec::with_capacity(packed.len() / 4 * 3);
    for chunk in packed.chunks(4) {
        let pad = chunk.iter().filter(|byte| **byte == b'=').count();
        if pad > 2 || chunk[..4 - pad].contains(&b'=') {
            return Err(MarkdownConversionError::new(
                "Footnote image base64 is invalid",
            ));
        }
        let mut sextets = [0_u8; 4];
        for (index, byte) in chunk.iter().take(4 - pad).enumerate() {
            sextets[index] = value(*byte)
                .ok_or_else(|| MarkdownConversionError::new("Footnote image base64 is invalid"))?;
        }
        out.push((sextets[0] << 2) | (sextets[1] >> 4));
        if pad < 2 {
            out.push((sextets[1] << 4) | (sextets[2] >> 2));
        }
        if pad < 1 {
            out.push((sextets[2] << 6) | sextets[3]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use zip::ZipArchive;

    use super::render;
    use crate::model::{
        Block, Blockquote, CalloutKind, Caption, CaptionKind, CodeBlock, CrossRef, Document,
        Heading, Inline, List, ListItem, Paragraph, Table, TableAlign, TableCell,
    };
    use crate::package::pack;

    fn pack_parts(doc: &Document) -> Vec<u8> {
        let parts = render(doc).expect("render");
        pack(&parts).expect("pack")
    }

    fn part(bytes: &[u8], name: &str) -> String {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut file = archive.by_name(name).expect(name);
        let mut text = String::new();
        file.read_to_string(&mut text).expect("read");
        text
    }

    fn run(text: &str) -> Inline {
        Inline {
            text: text.to_string(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            code: false,
            link: None,
            footnote: None,
            math_omml: None,
            cross_ref: None,
        }
    }

    fn marked(
        text: &str,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        code: bool,
    ) -> Inline {
        Inline {
            text: text.to_string(),
            bold,
            italic,
            underline,
            strike,
            code,
            link: None,
            footnote: None,
            math_omml: None,
            cross_ref: None,
        }
    }

    #[test]
    fn empty_document_has_one_paragraph_and_page_geometry() {
        let xml = part(
            &pack_parts(&Document {
                children: Vec::new(),
            }),
            "word/document.xml",
        );
        assert_eq!(xml.matches("</w:p>").count(), 1);
        assert!(xml.contains("w:w=\"11906\""));
        assert!(xml.contains("w:h=\"16838\""));
        assert!(xml.contains("w:top=\"1440\""));
        assert!(xml.contains("w:left=\"1080\""));
        let styles = part(
            &pack_parts(&Document {
                children: Vec::new(),
            }),
            "word/styles.xml",
        );
        for id in [
            "Normal", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
        ] {
            assert!(styles.contains(&format!("w:styleId=\"{id}\"")), "{id}");
        }
    }

    #[test]
    fn slice_one_body_marks_lists_tables_and_code() {
        let doc = Document {
            children: vec![
                Block::Heading(Heading {
                    level: 1,
                    bookmark: String::new(),
                    children: vec![run("Title")],
                }),
                Block::Heading(Heading {
                    level: 6,
                    bookmark: String::new(),
                    children: vec![run("Small")],
                }),
                Block::Paragraph(Paragraph {
                    children: vec![
                        marked("bold", true, false, false, false, false),
                        marked("italic", false, true, false, false, false),
                        marked("under", false, false, true, false, false),
                        marked("gone", false, false, false, true, false),
                        marked("code", false, false, false, false, true),
                        Inline {
                            text: "link".to_string(),
                            bold: false,
                            italic: false,
                            underline: false,
                            strike: false,
                            code: false,
                            link: Some("https://example.com".to_string()),
                            footnote: None,
                            math_omml: None,
                            cross_ref: None,
                        },
                    ],
                }),
                Block::List(List {
                    ordered: false,
                    start: None,
                    sequence_id: None,
                    children: vec![ListItem {
                        checked: Some(false),
                        children: vec![Block::Paragraph(Paragraph {
                            children: vec![
                                run("\u{2610} "),
                                marked("work", true, false, false, false, false),
                            ],
                        })],
                    }],
                }),
                Block::List(List {
                    ordered: true,
                    start: Some(0),
                    sequence_id: Some(1),
                    children: vec![ListItem {
                        checked: None,
                        children: vec![Block::Paragraph(Paragraph {
                            children: vec![run("zero")],
                        })],
                    }],
                }),
                Block::List(List {
                    ordered: true,
                    start: Some(5),
                    sequence_id: Some(2),
                    children: vec![ListItem {
                        checked: Some(true),
                        children: vec![
                            Block::Paragraph(Paragraph {
                                children: vec![run("\u{2612} outer")],
                            }),
                            Block::List(List {
                                ordered: true,
                                start: Some(3),
                                sequence_id: Some(3),
                                children: vec![ListItem {
                                    checked: None,
                                    children: vec![Block::Paragraph(Paragraph {
                                        children: vec![run("inner")],
                                    })],
                                }],
                            }),
                        ],
                    }],
                }),
                Block::ThematicBreak,
                Block::CodeBlock(CodeBlock::plain(
                    Some("typescript".to_string()),
                    "fn main() {\n  return;\n}".to_string(),
                )),
                Block::Blockquote(Blockquote {
                    callout: None,
                    children: vec![Block::Paragraph(Paragraph {
                        children: vec![run("quoted")],
                    })],
                }),
                Block::Blockquote(Blockquote {
                    callout: Some(CalloutKind::Note),
                    children: vec![Block::Paragraph(Paragraph {
                        children: vec![run("body")],
                    })],
                }),
                Block::Table(Table::gfm(
                    vec![TableAlign::Left, TableAlign::Center, TableAlign::Right],
                    vec![
                        TableCell::text(vec![run("Left")]),
                        TableCell::text(vec![run("Center")]),
                        TableCell::text(vec![run("Right")]),
                    ],
                    vec![vec![
                        TableCell::text(vec![run("a")]),
                        TableCell::text(vec![run("b")]),
                        TableCell::text(vec![marked("c", false, false, false, false, true)]),
                    ]],
                )),
            ],
        };
        let bytes = pack_parts(&doc);
        let xml = part(&bytes, "word/document.xml");
        assert!(xml.contains("Heading1"));
        assert!(xml.contains("Heading6"));
        assert!(xml.contains("<w:b/>"));
        assert!(xml.contains("<w:i/>"));
        assert!(xml.contains("w:val=\"single\""));
        assert!(xml.contains("<w:strike/>"));
        assert!(xml.contains("Courier New"));
        assert!(xml.contains("444444"));
        assert!(xml.contains("F5F5F5"));
        assert!(xml.contains("666666"));
        assert!(xml.contains("\u{2610} "));
        assert!(xml.contains("\u{2612} "));
        assert!(xml.contains("w:w=\"9746\""));
        assert!(xml.contains("w:type=\"dxa\""));
        assert!(xml.contains("A6A6A6"));
        assert!(xml.contains("w:sz=\"6\""));
        assert!(xml.contains("w:space=\"1\""));
        assert!(xml.contains("0969DA"));
        assert!(xml.contains("EFF6FF"));
        assert!(xml.contains(">Note<"));
        assert!(xml.contains("w:val=\"center\""));
        assert!(xml.contains("w:val=\"right\""));
        let numbering = part(&bytes, "word/numbering.xml");
        assert!(numbering.contains("w:val=\"0\""));
        assert!(numbering.contains("w:val=\"5\""));
        assert!(numbering.contains("w:val=\"3\""));
        assert!(numbering.contains("w:val=\"%1.\""));
        assert!(numbering.contains("w:val=\"%9.\""));
        assert!(numbering.contains("w:val=\"bullet\""));
        let rels = part(&bytes, "word/_rels/document.xml.rels");
        assert!(rels.contains("https://example.com"));
        assert!(rels.contains("TargetMode=\"External\""));
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
            "word/numbering.xml",
        ] {
            assert!(archive.by_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn caption_seq_and_cross_ref_fields() {
        let doc = Document {
            children: vec![
                Block::Paragraph(Paragraph {
                    children: vec![
                        Inline {
                            text: String::new(),
                            bold: false,
                            italic: false,
                            underline: false,
                            strike: false,
                            code: false,
                            link: None,
                            footnote: None,
                            math_omml: None,
                            cross_ref: Some(CrossRef {
                                kind: CaptionKind::Figure,
                                id: "overview".to_string(),
                                bookmark: "mdxref_figoverview_abc1234".to_string(),
                            }),
                        },
                        Inline {
                            text: "[@fig:literal]".to_string(),
                            bold: false,
                            italic: false,
                            underline: false,
                            strike: false,
                            code: true,
                            link: None,
                            footnote: None,
                            math_omml: None,
                            cross_ref: Some(CrossRef {
                                kind: CaptionKind::Figure,
                                id: "literal".to_string(),
                                bookmark: "mdxref_figliteral_abc1234".to_string(),
                            }),
                        },
                    ],
                }),
                Block::Caption(Caption {
                    kind: CaptionKind::Figure,
                    id: "overview".to_string(),
                    bookmark: "mdxref_figoverview_abc1234".to_string(),
                    children: vec![Inline {
                        text: "overview".to_string(),
                        bold: false,
                        italic: true,
                        underline: false,
                        strike: false,
                        code: false,
                        link: None,
                        footnote: None,
                        math_omml: None,
                        cross_ref: None,
                    }],
                }),
                Block::Caption(Caption {
                    kind: CaptionKind::Table,
                    id: "results".to_string(),
                    bookmark: "mdxref_tblresults_def5678".to_string(),
                    children: vec![run("Results")],
                }),
            ],
        };
        let bytes = pack_parts(&doc);
        let xml = part(&bytes, "word/document.xml");
        assert!(xml.contains("w:name=\"mdxref_figoverview_abc1234\""));
        assert!(xml.contains("w:name=\"mdxref_tblresults_def5678\""));
        assert!(xml.contains("w:instr=\" SEQ MdToDocxFigure \\* ARABIC \""));
        assert!(xml.contains("w:instr=\" SEQ MdToDocxTable \\* ARABIC \""));
        assert!(xml.contains("w:instr=\" REF mdxref_figoverview_abc1234 \\h \""));
        assert!(xml.contains(">Figure 1<"));
        assert!(xml.contains(">Table <"));
        assert!(xml.contains(">1<"));
        assert!(xml.contains("[@fig:literal]"));
        assert!(!xml.contains("mdxref_figliteral"));
        let figure = xml.find("w:name=\"mdxref_figoverview_abc1234\"").unwrap();
        let seq = xml.find("SEQ MdToDocxFigure").unwrap();
        let end = xml.find("w:bookmarkEnd").unwrap();
        assert!(figure < seq && seq < end);
        let settings = part(&bytes, "word/settings.xml");
        assert!(settings.contains("<w:updateFields"));
    }

    #[test]
    fn omitted_metadata_keeps_legacy_core_properties() {
        let parts = render(&Document { children: vec![] }).unwrap();
        let bytes = pack(&parts).unwrap();
        let core = part(&bytes, "docProps/core.xml");
        assert!(core.contains("Un-named"));
        assert!(core.contains("dcterms:created"));
        assert!(core.contains("dcterms:modified"));
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(archive.by_name("docProps/app.xml").is_err());
        assert!(archive.by_name("docProps/custom.xml").is_err());
    }

    #[test]
    fn empty_metadata_rewrites_core_without_legacy_fields() {
        use crate::convert::convert_with_options;
        use crate::options::{ConvertOptions, DocumentMetadata};

        let bytes = convert_with_options(
            "Content",
            &ConvertOptions {
                metadata: Some(DocumentMetadata::default()),
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        let core = part(&bytes, "docProps/core.xml");
        assert!(!core.contains("dcterms:created"));
        assert!(!core.contains("dcterms:modified"));
        assert!(!core.contains("Un-named"));
    }

    #[test]
    fn set_metadata_rewrites_core_app_and_custom() {
        use std::collections::BTreeMap;

        use crate::convert::convert_with_options;
        use crate::options::{ConvertOptions, DocumentMetadata};

        let bytes = convert_with_options(
            "# Report",
            &ConvertOptions {
                metadata: Some(DocumentMetadata {
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
                    custom: Some(BTreeMap::from([
                        ("Build".to_string(), "v3".to_string()),
                        ("ReviewState".to_string(), "Draft".to_string()),
                    ])),
                }),
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        let core = part(&bytes, "docProps/core.xml");
        let app = part(&bytes, "docProps/app.xml");
        let custom = part(&bytes, "docProps/custom.xml");
        assert!(core.contains(">Roadmap v3</dc:title>"));
        assert!(core.contains(">Primary creator</dc:creator>"));
        assert!(!core.contains("Ignored alias"));
        assert!(core.contains(">docx; a11y; Unicode 二</cp:keywords>"));
        assert!(core.contains(">en-GB</dc:language>"));
        assert!(core.contains("2026-01-02T03:04:05.000Z"));
        assert!(core.contains("2026-02-03T04:05:06.000Z"));
        assert!(!core.contains("Un-named"));
        assert!(app.contains("<Company>Example Co</Company>"));
        assert!(custom.contains("name=\"Build\""));
        assert!(custom.contains("name=\"ReviewState\""));
    }

    #[test]
    fn omitted_company_keeps_existing_app_properties() {
        use crate::convert::apply_document_metadata;
        use crate::options::DocumentMetadata;

        let mut parts = render(&Document { children: vec![] }).unwrap();
        parts.metadata = Some(super::PackageMetadata {
            company: Some("Existing Co".to_string()),
            ..super::PackageMetadata::default()
        });
        let source = pack(&parts).unwrap();
        let updated = apply_document_metadata(
            &source,
            &DocumentMetadata {
                title: Some("Updated title".to_string()),
                ..DocumentMetadata::default()
            },
        )
        .unwrap();
        let core = part(&updated, "docProps/core.xml");
        let app = part(&updated, "docProps/app.xml");
        assert!(core.contains("Updated title"));
        assert!(!core.contains("Un-named"));
        assert!(!core.contains("dcterms:created"));
        assert!(app.contains("<Company>Existing Co</Company>"));
    }

    #[test]
    fn metadata_writes_core_app_and_custom_properties() {
        let mut parts = render(&Document { children: vec![] }).unwrap();
        parts.metadata = Some(super::PackageMetadata {
            title: Some("R&D <測試> \"alpha\"".to_string()),
            creator: Some("Zoë & فريق".to_string()),
            company: Some("A < B & C".to_string()),
            keywords: Some("docx; a11y; Unicode 二".to_string()),
            created: Some("2026-01-02T03:04:05Z".to_string()),
            modified: Some("2026-02-03T04:05:06Z".to_string()),
            custom: Some(vec![
                ("Build".to_string(), "v3".to_string()),
                ("Owner & team".to_string(), "Miyuki <美雪> & co".to_string()),
            ]),
            ..super::PackageMetadata::default()
        });
        super::install_package_metadata(&mut parts).unwrap();
        let core = named(&parts, "docProps/core.xml");
        let app = named(&parts, "docProps/app.xml");
        let custom = named(&parts, "docProps/custom.xml");
        assert!(core.contains("R&amp;D &lt;測試&gt; \"alpha\""));
        assert!(core.contains("Zoë &amp; فريق"));
        assert!(core.contains(">docx; a11y; Unicode 二</cp:keywords>"));
        assert!(core.contains("2026-01-02T03:04:05.000Z"));
        assert!(core.contains("2026-02-03T04:05:06.000Z"));
        assert_eq!(core.matches("<?xml").count(), 1);
        assert!(app.contains("A &lt; B &amp; C"));
        assert!(custom.contains("name=\"Build\""));
        assert!(custom.contains(">v3</vt:lpwstr>"));
        assert!(custom.contains("name=\"Owner &amp; team\""));
        assert!(custom.contains("Miyuki &lt;美雪&gt; &amp; co"));
        let types = named(&parts, "[Content_Types].xml");
        assert!(types.contains("PartName=\"/docProps/core.xml\""));
        assert!(types.contains("PartName=\"/docProps/app.xml\""));
        assert!(types.contains("PartName=\"/docProps/custom.xml\""));
        let rels = named(&parts, "_rels/.rels");
        assert!(rels.contains("Target=\"docProps/core.xml\""));
        assert!(rels.contains("Target=\"docProps/app.xml\""));
        assert!(rels.contains("Target=\"docProps/custom.xml\""));
        assert!(rels.contains("Id=\"rId2\""));
    }

    fn named(parts: &super::Parts, path: &str) -> String {
        let (_, bytes) = parts
            .files()
            .into_iter()
            .find(|(name, _)| *name == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}
