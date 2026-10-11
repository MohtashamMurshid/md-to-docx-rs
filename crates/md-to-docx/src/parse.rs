//! Walk pulldown-cmark events into the owned document model.

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
    TextMergeStream,
};

use std::collections::BTreeMap;

use crate::bookmarks::{heading_bookmark, HeadingBookmarks};
use crate::captions::cross_reference_bookmark_id;
use crate::error::MarkdownConversionError;
use crate::images::{load_markdown_image, ImageOptions};
use crate::model::{
    Block, Blockquote, CalloutKind, Caption, CaptionKind, CodeBlock, CodeColors, CrossRef,
    Document, FootnoteMark, HeaderFooterBody, Heading, Image, Inline, List, ListItem, Paragraph,
    PreparedDocument, PreparedSection, ResolvedHeaderFooter, SectionBlock, Table, TableAlign,
    TableCell, TocBlock, TocHeading,
};
use crate::options::{
    resolve_sections, resolve_toc, CaptionFailureMode, CaptionPlacement, ChartInvalidBehavior,
    ChartRenderer, ChartRenderingOptions, CodeHighlightTheme, ConvertOptions, MathUnsupported,
    MermaidFailureMode, MermaidRenderInput, MermaidRenderer, MermaidRenderingOptions,
    MissingImageAltText, OptionBool, PluginBlock, PluginFailureMode, PluginFenceInput,
    PluginInline, PluginRuntime, ResolvedHeaderFooterInput, ResolvedSectionInput,
    ResolvedSlotInput, TextFind, TextReplace, TocMode, TocSettings, UpdateFieldsOnOpen,
};

/// Parse `markdown` into the document tree.
///
/// Parser flags are tables, strikethrough, GFM callouts, task lists,
/// footnotes, and math. `mathRendering.enabled: false` leaves `$...$` as
/// text. A footnote reference whose id has no definition stays the literal
/// `[^id]` text.
pub fn parse_markdown(markdown: &str) -> Result<Document, MarkdownConversionError> {
    let mut cursor = ParseCursor::new();
    let mut document = parse_with(markdown, &mut cursor)?;
    bind_documents(
        std::slice::from_mut(&mut document),
        &CaptionPass::preserve(),
    )?;
    Ok(document)
}

/// Resolve sections, headers, footers, and `[TOC]` placeholders.
///
/// A footnote in a header or footer fails with
/// `Word headers and footers cannot contain footnotes`.
pub fn prepare(
    markdown: &str,
    options: &ConvertOptions,
) -> Result<PreparedDocument, MarkdownConversionError> {
    let toc = resolve_toc(options.toc.as_ref());
    let resolved = resolve_sections(markdown, options);
    reject_header_footer_length(&resolved, options.max_input_length)?;
    let mut cursor = ParseCursor::new();
    cursor.missing_alt = missing_alt_policy(options);
    cursor.images.missing_image_alt_text = image_alt_policy(cursor.missing_alt);
    cursor.math_enabled = math_enabled(options);
    cursor.math_throw = math_throws(options);
    cursor.chart = chart_pass(options);
    cursor.mermaid = mermaid_pass(options);
    cursor.highlight = highlight_pass(options);
    cursor.replacements = prepare_replacements(options)?;
    let runtime = PluginRuntime::create(options)?;
    let max_elements = options.max_elements.map(u64::from);
    let mut element_count = 0u64;
    let mut bodies = Vec::with_capacity(resolved.len());
    for section in &resolved {
        bodies.push(parse_with(&section.markdown, &mut cursor)?);
    }
    if let Some(runtime) = &runtime {
        for document in &mut bodies {
            runtime.transform();
            document.children =
                rewrite_plugin_blocks(std::mem::take(&mut document.children), runtime)?;
        }
    }
    let pass = CaptionPass::from_options(options);
    let defined = bind_documents(&mut bodies, &pass)?;
    let section_count = resolved.len();
    let mut sections = Vec::with_capacity(section_count);
    for (index, (section, mut document)) in resolved.into_iter().zip(bodies).enumerate() {
        // A section that ends on a caption gets a trailing paragraph for the
        // section break.
        if index + 1 < section_count && ends_with_caption(&document.children) {
            document.children.push(Block::Paragraph(Paragraph {
                children: Vec::new(),
            }));
        }
        if let Some(max) = max_elements {
            element_count = count_elements(&document.children, element_count, max)?;
        }
        let headers = parse_header_footer(&section.headers, &mut cursor)?;
        let footers = parse_header_footer(&section.footers, &mut cursor)?;
        if let Some(max) = max_elements {
            element_count = count_header_footer(&headers, element_count, max)?;
            element_count = count_header_footer(&footers, element_count, max)?;
        }
        sections.push(PreparedSection {
            markdown: section.markdown,
            style: section.style,
            page: section.page,
            page_numbering: section.page_numbering,
            title_page: section.title_page,
            break_type: section.break_type,
            headers,
            footers,
            children: lift_toc(document.children),
        });
    }
    let _ = element_count;
    let headings = collect_headings(&sections);
    let filtered = filter_headings(&headings, &toc);
    let insert = !filtered.is_empty();
    let mut marked = false;
    for section in &mut sections {
        for block in &mut section.children {
            let SectionBlock::TableOfContents(placeholder) = block else {
                continue;
            };
            placeholder.settings = toc.clone();
            if insert && !marked {
                placeholder.primary = true;
                placeholder.headings = filtered.clone();
                marked = true;
            }
        }
    }
    let captions_open = captions_update_on_open(options);
    let update_fields = (defined && captions_open)
        || (marked && toc.mode != TocMode::Links && toc.update_fields_on_open);
    Ok(PreparedDocument {
        sections,
        update_fields,
        toc,
    })
}

/// Replace plugin fences after text replacements and before the element limit.
fn rewrite_plugin_blocks(
    blocks: Vec<Block>,
    runtime: &PluginRuntime,
) -> Result<Vec<Block>, MarkdownConversionError> {
    let mut rewritten = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            Block::CodeBlock(code) => {
                rewritten.extend(render_plugin_fence(code, runtime)?);
            }
            Block::List(mut list) => {
                for item in &mut list.children {
                    item.children =
                        rewrite_plugin_blocks(std::mem::take(&mut item.children), runtime)?;
                }
                rewritten.push(Block::List(list));
            }
            Block::Blockquote(mut quote) => {
                quote.children =
                    rewrite_plugin_blocks(std::mem::take(&mut quote.children), runtime)?;
                rewritten.push(Block::Blockquote(quote));
            }
            Block::FootnoteDefinition { id, children } => {
                rewritten.push(Block::FootnoteDefinition {
                    id,
                    children: rewrite_plugin_blocks(children, runtime)?,
                });
            }
            Block::Table(mut table) => {
                for cell in &mut table.header {
                    cell.blocks = rewrite_plugin_blocks(std::mem::take(&mut cell.blocks), runtime)?;
                }
                for row in &mut table.rows {
                    for cell in row {
                        cell.blocks =
                            rewrite_plugin_blocks(std::mem::take(&mut cell.blocks), runtime)?;
                    }
                }
                rewritten.push(Block::Table(table));
            }
            Block::Paragraph(paragraph) => rewritten.push(Block::Paragraph(paragraph)),
            Block::Heading(heading) => rewritten.push(Block::Heading(heading)),
            Block::ThematicBreak => rewritten.push(Block::ThematicBreak),
            Block::PageBreak => rewritten.push(Block::PageBreak),
            Block::Comment(comment) => rewritten.push(Block::Comment(comment)),
            Block::Caption(caption) => rewritten.push(Block::Caption(caption)),
            Block::Image(image) => rewritten.push(Block::Image(image)),
            Block::DisplayMath { omml } => rewritten.push(Block::DisplayMath { omml }),
            Block::ChartImage {
                bytes,
                width_px,
                height_px,
                alt,
            } => rewritten.push(Block::ChartImage {
                bytes,
                width_px,
                height_px,
                alt,
            }),
        }
    }
    Ok(rewritten)
}

fn render_plugin_fence(
    code: CodeBlock,
    runtime: &PluginRuntime,
) -> Result<Vec<Block>, MarkdownConversionError> {
    let Some(language) = code.language.as_deref() else {
        return Ok(vec![Block::CodeBlock(code)]);
    };
    let normalized = language.trim().to_ascii_lowercase();
    let Some(binding) = runtime.fence(&normalized) else {
        return Ok(vec![Block::CodeBlock(code)]);
    };
    let input = PluginFenceInput {
        language: normalized,
        value: code.value.clone(),
        meta: None,
    };
    match binding.render(&input) {
        Ok(blocks) => Ok(lower_plugin_blocks(blocks)),
        Err(error) => match binding.failure_mode {
            PluginFailureMode::Fallback => Ok(vec![Block::CodeBlock(code)]),
            PluginFailureMode::Skip => Ok(Vec::new()),
            PluginFailureMode::Throw => Err(MarkdownConversionError::new(format!(
                "Plugin \"{}\" fence renderer failed: {}",
                binding.plugin_name, error
            ))),
        },
    }
}

fn lower_plugin_blocks(blocks: Vec<PluginBlock>) -> Vec<Block> {
    let mut lowered = Vec::new();
    for block in blocks {
        match block {
            PluginBlock::Paragraph { children } => {
                lowered.push(Block::Paragraph(Paragraph {
                    children: children.into_iter().map(plugin_run).collect(),
                }));
            }
            PluginBlock::Skip => {}
        }
    }
    lowered
}

fn plugin_run(inline: PluginInline) -> Inline {
    Inline {
        text: inline.text,
        bold: inline.bold,
        italic: inline.italic,
        underline: inline.underline,
        strike: inline.strikethrough,
        code: inline.code,
        link: inline.link,
        footnote: None,
        cross_ref: None,
        math_omml: None,
    }
}

fn parse_with(
    markdown: &str,
    cursor: &mut ParseCursor,
) -> Result<Document, MarkdownConversionError> {
    let owned = std::mem::replace(cursor, ParseCursor::new());
    let mut walker = Walker::new(owned);
    let math_enabled = walker.math_enabled;
    let events = TextMergeStream::new(Parser::new_ext(markdown, parser_options(math_enabled)));
    for event in events {
        walker.on_event(event)?;
    }
    let (document, next) = walker.finish()?;
    *cursor = next;
    Ok(document)
}

fn parser_options(math_enabled: bool) -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);
    // `Event::TaskListMarker` is emitted only with this flag. In 0.13.4,
    // `ENABLE_GFM` covers callout kinds and does not set the task-list bit.
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    // `Options::ENABLE_MATH` is the 0.13.4 flag for `InlineMath` and `DisplayMath`.
    // This crate has no `Options::from_name`.
    if math_enabled {
        options.insert(Options::ENABLE_MATH);
    }
    options
}

struct ParseCursor {
    next_sequence: u32,
    footnote_base: u32,
    bookmarks: HeadingBookmarks,
    images: ImageOptions,
    missing_alt: MissingImageAltText,
    math_enabled: bool,
    math_throw: bool,
    chart: Option<ChartPass>,
    mermaid: Option<MermaidPass>,
    highlight: Option<HighlightPass>,
    replacements: Vec<PreparedReplacement>,
}

impl ParseCursor {
    fn new() -> Self {
        Self {
            next_sequence: 1,
            footnote_base: 0,
            bookmarks: HeadingBookmarks::new(),
            images: ImageOptions::default(),
            missing_alt: MissingImageAltText::Permissive,
            math_enabled: true,
            math_throw: false,
            chart: None,
            mermaid: None,
            highlight: None,
            replacements: Vec::new(),
        }
    }
}

fn missing_alt_policy(options: &ConvertOptions) -> MissingImageAltText {
    options
        .accessibility
        .as_ref()
        .and_then(|accessibility| accessibility.missing_image_alt_text)
        .unwrap_or(MissingImageAltText::Permissive)
}

fn math_enabled(options: &ConvertOptions) -> bool {
    match options
        .math_rendering
        .as_ref()
        .and_then(|math| math.enabled)
    {
        Some(OptionBool::Bool(enabled)) => enabled,
        Some(OptionBool::Invalid) | None => true,
    }
}

fn math_throws(options: &ConvertOptions) -> bool {
    matches!(
        options
            .math_rendering
            .as_ref()
            .and_then(|math| math.unsupported),
        Some(MathUnsupported::Throw)
    )
}

fn chart_pass(options: &ConvertOptions) -> Option<ChartPass> {
    let chart = options.chart_rendering.as_ref()?;
    if !matches!(chart.enabled, Some(OptionBool::Bool(true))) {
        return None;
    }
    Some(ChartPass::from_options(chart))
}

fn mermaid_pass(options: &ConvertOptions) -> Option<MermaidPass> {
    let mermaid = options.mermaid_rendering.as_ref()?;
    if !matches!(mermaid.enabled, Some(OptionBool::Bool(true))) {
        return None;
    }
    Some(MermaidPass::from_options(mermaid))
}

fn highlight_pass(options: &ConvertOptions) -> Option<HighlightPass> {
    if !options.highlighting_enabled() {
        return None;
    }
    let highlighting = options.code_highlighting.as_ref()?;
    let theme = highlighting.theme.as_ref();
    Some(HighlightPass {
        show_label: !matches!(
            highlighting.show_language_label,
            Some(OptionBool::Bool(false))
        ),
        languages: highlighting.languages.clone(),
        label_color: theme_hex(theme, |theme| theme.language_label.as_deref(), "6A737D"),
        plain_color: theme_hex(theme, |theme| theme.default.as_deref(), "24292E"),
        fill: theme_hex(theme, |theme| theme.background.as_deref(), "FFFFFF"),
        border: theme_hex(theme, |theme| theme.border.as_deref(), "D0D7DE"),
    })
}

fn theme_hex(
    theme: Option<&CodeHighlightTheme>,
    pick: impl Fn(&CodeHighlightTheme) -> Option<&str>,
    fallback: &str,
) -> String {
    theme
        .and_then(pick)
        .filter(|color| !color.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn prepare_replacements(
    options: &ConvertOptions,
) -> Result<Vec<PreparedReplacement>, MarkdownConversionError> {
    let Some(items) = options.text_replacements.as_ref() else {
        return Ok(Vec::new());
    };
    let mut prepared = Vec::with_capacity(items.len());
    for item in items {
        let find = match &item.find {
            TextFind::Literal(text) => PreparedFind::Literal(text.clone()),
            TextFind::Regex(source) => {
                PreparedFind::Pattern(regex::Regex::new(source).map_err(|_| {
                    MarkdownConversionError::new("Unsafe textReplacements RegExp rejected")
                })?)
            }
        };
        prepared.push(PreparedReplacement {
            find,
            replace: item.replace.clone(),
        });
    }
    Ok(prepared)
}

fn image_alt_policy(policy: MissingImageAltText) -> crate::images::MissingImageAltText {
    match policy {
        MissingImageAltText::Throw => crate::images::MissingImageAltText::Throw,
        MissingImageAltText::Permissive | MissingImageAltText::Invalid => {
            crate::images::MissingImageAltText::Permissive
        }
    }
}

struct Walker {
    stack: Vec<Frame>,
    marks: MarkCounts,
    next_sequence: u32,
    footnote_base: u32,
    /// Footnote bodies keyed by the normalized label. The first definition wins.
    footnote_definitions: BTreeMap<String, Vec<Block>>,
    /// Normalized labels in the order of their first reference.
    referenced_footnotes: Vec<String>,
    /// Word ids (`"1"`, `"2"`, ...) for those labels.
    footnote_numbers: BTreeMap<String, String>,
    heading_bookmarks: HeadingBookmarks,
    images: ImageOptions,
    missing_alt: MissingImageAltText,
    math_enabled: bool,
    math_throw: bool,
    chart: Option<ChartPass>,
    mermaid: Option<MermaidPass>,
    highlight: Option<HighlightPass>,
    replacements: Vec<PreparedReplacement>,
    /// Rich-table fence depth. The document is 0. A cell is parsed at depth + 1.
    table_depth: u32,
}

impl Walker {
    fn new(cursor: ParseCursor) -> Self {
        Self {
            stack: vec![Frame::Root {
                children: Vec::new(),
            }],
            marks: MarkCounts::default(),
            next_sequence: cursor.next_sequence,
            footnote_base: cursor.footnote_base,
            footnote_definitions: BTreeMap::new(),
            referenced_footnotes: Vec::new(),
            footnote_numbers: BTreeMap::new(),
            heading_bookmarks: cursor.bookmarks,
            images: cursor.images,
            missing_alt: cursor.missing_alt,
            math_enabled: cursor.math_enabled,
            math_throw: cursor.math_throw,
            chart: cursor.chart,
            mermaid: cursor.mermaid,
            highlight: cursor.highlight,
            replacements: cursor.replacements,
            table_depth: 0,
        }
    }

    fn finish(mut self) -> Result<(Document, ParseCursor), MarkdownConversionError> {
        match self.stack.pop() {
            Some(Frame::Root { mut children }) if self.stack.is_empty() => {
                let assigned = u32::try_from(self.footnote_numbers.len()).unwrap_or(u32::MAX);
                for label in self.referenced_footnotes {
                    let Some(body) = self.footnote_definitions.remove(&label) else {
                        continue;
                    };
                    let id = self.footnote_numbers.get(&label).cloned().unwrap_or(label);
                    children.push(Block::FootnoteDefinition { id, children: body });
                }
                let cursor = ParseCursor {
                    next_sequence: self.next_sequence,
                    footnote_base: self.footnote_base.saturating_add(assigned),
                    bookmarks: self.heading_bookmarks,
                    images: self.images,
                    missing_alt: self.missing_alt,
                    math_enabled: self.math_enabled,
                    math_throw: self.math_throw,
                    chart: self.chart,
                    mermaid: self.mermaid,
                    highlight: self.highlight,
                    replacements: self.replacements,
                };
                Ok((Document { children }, cursor))
            }
            _ => Err(parse_error("unclosed Markdown block")),
        }
    }

    fn on_event(&mut self, event: Event<'_>) -> Result<(), MarkdownConversionError> {
        if self.in_skip() {
            return self.on_skipped_event(event);
        }
        if self.in_image_alt() {
            return self.on_image_alt(event);
        }

        match event {
            Event::Start(tag) => self.on_start(tag),
            Event::End(end) => self.on_end(end),
            Event::Text(text) => self.on_text(&text),
            Event::Code(text) => self.on_code(&text),
            Event::SoftBreak => self.on_soft_break(),
            Event::HardBreak => self.on_hard_break(),
            Event::Rule => self.add_block(Block::ThematicBreak),
            Event::TaskListMarker(checked) => self.on_task_marker(checked),
            Event::Html(html) => self.on_html(&html),
            Event::InlineHtml(_) => Ok(()),
            Event::FootnoteReference(label) => self.on_footnote_reference(&label),
            Event::InlineMath(text) => self.on_inline_math(&text),
            Event::DisplayMath(text) => self.on_display_math(&text),
        }
    }

    fn on_start(&mut self, tag: Tag<'_>) -> Result<(), MarkdownConversionError> {
        match tag {
            Tag::Paragraph => {
                self.flush_tight_item();
                let inline = self.fresh_inline();
                self.stack.push(Frame::Paragraph {
                    inline,
                    image: None,
                    display: None,
                });
                Ok(())
            }
            Tag::Heading { level, .. } => {
                self.flush_tight_item();
                let inline = self.fresh_inline();
                self.stack.push(Frame::Heading {
                    level: heading_level(level),
                    inline,
                });
                Ok(())
            }
            Tag::BlockQuote(kind) => {
                self.flush_tight_item();
                self.stack.push(Frame::BlockQuote {
                    callout: kind.map(callout_kind),
                    children: Vec::new(),
                });
                Ok(())
            }
            Tag::CodeBlock(kind) => {
                self.flush_tight_item();
                let (language, meta) = fence_parts(&kind);
                self.stack.push(Frame::Code {
                    language,
                    meta,
                    value: String::new(),
                });
                Ok(())
            }
            Tag::List(start) => {
                self.flush_tight_item();
                let ordered = start.is_some();
                let sequence_id = if ordered {
                    let id = self.next_sequence;
                    self.next_sequence = self.next_sequence.saturating_add(1);
                    Some(id)
                } else {
                    None
                };
                self.stack.push(Frame::List {
                    ordered,
                    start,
                    sequence_id,
                    items: Vec::new(),
                });
                Ok(())
            }
            Tag::Item => {
                let inline = self.fresh_inline();
                self.stack.push(Frame::Item {
                    checked: None,
                    marker: None,
                    children: Vec::new(),
                    inline,
                });
                Ok(())
            }
            Tag::Table(align) => {
                self.flush_tight_item();
                self.stack.push(Frame::Table {
                    align: align.into_iter().map(table_align).collect(),
                    header: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                });
                Ok(())
            }
            Tag::TableHead | Tag::TableRow => match self.stack.last_mut() {
                Some(Frame::Table { row, .. }) => {
                    row.clear();
                    Ok(())
                }
                _ => Err(parse_error("table event outside a table")),
            },
            Tag::TableCell => {
                let inline = self.fresh_inline();
                self.stack.push(Frame::Cell { inline });
                Ok(())
            }
            Tag::Emphasis => self.push_mark(MarkKind::Italic),
            Tag::Strong => self.push_mark(MarkKind::Bold),
            Tag::Strikethrough => self.push_mark(MarkKind::Strike),
            Tag::Link { dest_url, .. } => {
                let url: &str = &dest_url;
                self.marks.links.push(url.to_string());
                self.sync_marks();
                Ok(())
            }
            Tag::HtmlBlock => {
                self.flush_tight_item();
                self.stack.push(Frame::Html {
                    value: String::new(),
                });
                Ok(())
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_tight_item();
                self.stack.push(Frame::Footnote {
                    id: normalize_footnote_id(&label),
                    children: Vec::new(),
                });
                Ok(())
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                if self.in_footnote() || self.in_table_cell() {
                    self.stack.push(Frame::Skip { depth: 1 });
                    return Ok(());
                }
                self.demote_sole_image();
                self.demote_display_math();
                let url: &str = &dest_url;
                let title: &str = &title;
                let sole = self.stack.last().is_some_and(|frame| match frame {
                    Frame::Paragraph {
                        inline,
                        image,
                        display,
                    } => image.is_none() && display.is_none() && inline.is_blank(),
                    _ => false,
                });
                self.stack.push(Frame::ImageAlt {
                    depth: 1,
                    url: url.to_string(),
                    title: (!title.is_empty()).then(|| title.to_string()),
                    alt: String::new(),
                    sole,
                });
                Ok(())
            }
            Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {
                self.stack.push(Frame::Skip { depth: 1 });
                Ok(())
            }
            Tag::Superscript | Tag::Subscript => Ok(()),
        }
    }

    fn on_end(&mut self, end: TagEnd) -> Result<(), MarkdownConversionError> {
        match end {
            TagEnd::Emphasis => self.pop_mark(MarkKind::Italic),
            TagEnd::Strong => self.pop_mark(MarkKind::Bold),
            TagEnd::Strikethrough => self.pop_mark(MarkKind::Strike),
            TagEnd::Link => {
                self.marks.links.pop();
                self.sync_marks();
                Ok(())
            }
            TagEnd::Superscript | TagEnd::Subscript => Ok(()),
            TagEnd::TableHead => match self.stack.last_mut() {
                Some(Frame::Table { header, row, .. }) => {
                    *header = std::mem::take(row);
                    Ok(())
                }
                _ => Err(parse_error("table event outside a table")),
            },
            TagEnd::TableRow => match self.stack.last_mut() {
                Some(Frame::Table {
                    header, rows, row, ..
                }) => {
                    let width = header.len();
                    let cells = std::mem::take(row);
                    rows.push(fit_row(cells, width));
                    Ok(())
                }
                _ => Err(parse_error("table event outside a table")),
            },
            TagEnd::Paragraph => self.finish_paragraph(),
            TagEnd::Heading(_) => self.finish_heading(),
            TagEnd::BlockQuote(_) => self.finish_quote(),
            TagEnd::CodeBlock => self.finish_code(),
            TagEnd::List(_) => self.finish_list(),
            TagEnd::Item => self.finish_item(),
            TagEnd::Table => self.finish_table(),
            TagEnd::TableCell => self.finish_cell(),
            TagEnd::HtmlBlock => self.finish_html(),
            TagEnd::FootnoteDefinition => self.finish_footnote(),
            TagEnd::Image
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => Err(parse_error("unclosed skipped Markdown block")),
        }
    }

    fn on_text(&mut self, text: &str) -> Result<(), MarkdownConversionError> {
        if let Some(Frame::Code { value, .. }) = self.stack.last_mut() {
            value.push_str(text);
            return Ok(());
        }
        let replaced = self.apply_replacements(text);
        self.demote_sole_image();
        self.demote_display_math();
        self.current_inline()?.push_text(&replaced);
        Ok(())
    }

    fn on_inline_math(&mut self, source: &str) -> Result<(), MarkdownConversionError> {
        self.demote_sole_image();
        self.demote_display_math();
        match self.render_math(source, false)? {
            MathPiece::Omml(omml) => self.current_inline()?.push_math(omml),
            MathPiece::Text(text) => self.current_inline()?.push_text(&text),
        }
        Ok(())
    }

    fn on_display_math(&mut self, source: &str) -> Result<(), MarkdownConversionError> {
        self.demote_sole_image();
        self.demote_display_math();
        let source = source.trim();
        match self.render_math(source, true)? {
            MathPiece::Text(text) => {
                self.current_inline()?.push_text(&text);
                Ok(())
            }
            MathPiece::Omml(omml) => {
                let sole = self.stack.last().is_some_and(|frame| match frame {
                    Frame::Paragraph {
                        inline,
                        image,
                        display,
                    } => image.is_none() && display.is_none() && inline.is_blank(),
                    _ => false,
                });
                if sole {
                    if let Some(Frame::Paragraph { display, .. }) = self.stack.last_mut() {
                        *display = Some(omml);
                    }
                    return Ok(());
                }
                self.current_inline()?.push_math(omml);
                Ok(())
            }
        }
    }

    fn render_math(
        &self,
        source: &str,
        display: bool,
    ) -> Result<MathPiece, MarkdownConversionError> {
        match crate::math::tex_to_omml(source) {
            Ok(omml) => Ok(MathPiece::Omml(omml)),
            Err(err) if self.math_throw => Err(unsupported_math(source, err)),
            Err(_) => Ok(MathPiece::Text(literal_math(source, display))),
        }
    }

    fn on_code(&mut self, text: &str) -> Result<(), MarkdownConversionError> {
        self.demote_sole_image();
        self.demote_display_math();
        self.current_inline()?.push_code(text);
        Ok(())
    }

    fn on_soft_break(&mut self) -> Result<(), MarkdownConversionError> {
        if let Some(Frame::Code { value, .. }) = self.stack.last_mut() {
            value.push('\n');
            return Ok(());
        }
        // `[!INFO]` and the next quoted line stay one run, joined by a newline.
        let newline = self.unsupported_callout_soft_break();
        self.demote_sole_image();
        self.demote_display_math();
        self.current_inline()?
            .push_text(if newline { "\n" } else { " " });
        Ok(())
    }

    fn on_hard_break(&mut self) -> Result<(), MarkdownConversionError> {
        if let Some(Frame::Code { value, .. }) = self.stack.last_mut() {
            value.push('\n');
            return Ok(());
        }
        self.demote_sole_image();
        self.demote_display_math();
        self.current_inline()?.push_hard_break();
        Ok(())
    }

    fn on_task_marker(&mut self, checked: bool) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Item {
            checked: slot,
            marker,
            ..
        }) = self.stack.last_mut()
        else {
            return Err(parse_error("task list marker outside a list item"));
        };
        if slot.is_none() {
            *slot = Some(checked);
            *marker = Some(if checked { "\u{2612} " } else { "\u{2610} " });
        }
        Ok(())
    }

    fn on_html(&mut self, html: &str) -> Result<(), MarkdownConversionError> {
        if let Some(Frame::Html { value }) = self.stack.last_mut() {
            value.push_str(html);
        }
        Ok(())
    }

    fn on_footnote_reference(&mut self, label: &str) -> Result<(), MarkdownConversionError> {
        // A reference inside a footnote body stays the source text, even when
        // that id has its own definition.
        if self.in_footnote() {
            return self.on_text(&format!("[^{label}]"));
        }
        let label = normalize_footnote_id(label);
        let (id, first) = if let Some(id) = self.footnote_numbers.get(&label) {
            (id.clone(), false)
        } else {
            let id = (self.footnote_base + self.footnote_numbers.len() as u32 + 1).to_string();
            self.footnote_numbers.insert(label.clone(), id.clone());
            self.referenced_footnotes.push(label);
            (id, true)
        };
        self.demote_sole_image();
        self.demote_display_math();
        self.current_inline()?
            .push_footnote(FootnoteMark { id, first });
        Ok(())
    }

    fn in_footnote(&self) -> bool {
        self.stack
            .iter()
            .any(|frame| matches!(frame, Frame::Footnote { .. }))
    }

    fn in_table_cell(&self) -> bool {
        self.stack
            .iter()
            .any(|frame| matches!(frame, Frame::Cell { .. }))
    }

    fn in_image_alt(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::ImageAlt { .. }))
    }

    fn on_image_alt(&mut self, event: Event<'_>) -> Result<(), MarkdownConversionError> {
        match event {
            Event::Start(_) => {
                if let Some(Frame::ImageAlt { depth, .. }) = self.stack.last_mut() {
                    *depth = depth.saturating_add(1);
                }
            }
            Event::End(_) => {
                let done = match self.stack.last_mut() {
                    Some(Frame::ImageAlt { depth, .. }) => {
                        *depth = depth.saturating_sub(1);
                        *depth == 0
                    }
                    _ => false,
                };
                if done {
                    let Some(Frame::ImageAlt {
                        url,
                        title,
                        alt,
                        sole,
                        ..
                    }) = self.stack.pop()
                    else {
                        return Err(parse_error("image ended without an image"));
                    };
                    return self.finish_image(url, title, alt, sole);
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(Frame::ImageAlt { alt, .. }) = self.stack.last_mut() {
                    alt.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if let Some(Frame::ImageAlt { alt, .. }) = self.stack.last_mut() {
                    alt.push(' ');
                }
            }
            Event::HardBreak => {
                if let Some(Frame::ImageAlt { alt, .. }) = self.stack.last_mut() {
                    alt.push('\n');
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish_image(
        &mut self,
        url: String,
        title: Option<String>,
        alt: String,
        sole: bool,
    ) -> Result<(), MarkdownConversionError> {
        validate_image_text(&alt, title.as_deref())?;
        if self.missing_alt == MissingImageAltText::Throw && alt.trim().is_empty() {
            return Err(MarkdownConversionError::new(
                "Missing image alt text in Markdown image; strict accessibility mode requires a non-empty description",
            ));
        }
        let loaded = load_markdown_image(&url, &alt, title.as_deref(), &self.images);
        let image = match loaded {
            Ok(loaded) => Image {
                bytes: Some(loaded.bytes).filter(|bytes| !bytes.is_empty()),
                content_type: loaded.content_type,
                width_px: loaded.width_px,
                height_px: loaded.height_px,
                alt,
                title,
                url,
            },
            Err(err) if image_error_stops_conversion(&err) => return Err(err),
            Err(_) => Image {
                bytes: None,
                content_type: String::new(),
                width_px: 0,
                height_px: 0,
                alt,
                title,
                url,
            },
        };
        if sole {
            if let Some(Frame::Paragraph {
                inline,
                image: slot,
                display,
            }) = self.stack.last_mut()
            {
                if slot.is_none() && display.is_none() && inline.is_blank() {
                    *slot = Some(image);
                    return Ok(());
                }
            }
        }
        self.push_inline_image(image);
        Ok(())
    }

    fn demote_sole_image(&mut self) {
        let Some(Frame::Paragraph { inline, image, .. }) = self.stack.last_mut() else {
            return;
        };
        let Some(image) = image.take() else {
            return;
        };
        inline.push_run(inline_image_run(&image));
    }

    fn demote_display_math(&mut self) {
        let Some(Frame::Paragraph {
            inline, display, ..
        }) = self.stack.last_mut()
        else {
            return;
        };
        let Some(omml) = display.take() else {
            return;
        };
        inline.push_math(omml);
    }

    fn push_inline_image(&mut self, image: Image) {
        let Some(inline) = self.current_inline_mut() else {
            return;
        };
        inline.push_run(inline_image_run(&image));
    }

    fn finish_html(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Html { value }) = self.stack.pop() else {
            return Err(parse_error("html block ended without an html block"));
        };
        match classify_html(&value) {
            Some(block) => self.add_block(block),
            None => Ok(()),
        }
    }

    fn finish_footnote(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Footnote { id, children }) = self.stack.pop() else {
            return Err(parse_error("footnote ended without a footnote"));
        };
        if self.footnote_definitions.contains_key(&id) {
            return Ok(());
        }
        self.footnote_definitions
            .insert(id, footnote_children(children));
        Ok(())
    }

    fn finish_paragraph(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Paragraph {
            inline,
            image,
            display,
        }) = self.stack.pop()
        else {
            return Err(parse_error("paragraph ended without a paragraph"));
        };
        let mut runs = inline.finish();
        let in_cell = matches!(self.stack.last(), Some(Frame::Cell { .. }));
        if let Some(omml) = display {
            if runs_are_blank(&runs) && image.is_none() && !in_cell {
                return self.add_block(Block::DisplayMath { omml });
            }
            runs.insert(0, math_inline(omml));
        }
        if let Some(Frame::Cell { inline: cell }) = self.stack.last_mut() {
            cell.append_runs(runs);
            return Ok(());
        }
        if let Some(image) = image {
            if runs_are_blank(&runs) && !self.in_footnote() {
                return self.add_block(Block::Image(image));
            }
            runs.insert(0, inline_image_run(&image));
        }
        if is_root_pagebreak(&self.stack, &runs) {
            return self.add_block(Block::PageBreak);
        }
        self.add_block(Block::Paragraph(Paragraph { children: runs }))
    }

    fn finish_heading(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Heading { level, inline }) = self.stack.pop() else {
            return Err(parse_error("heading ended without a heading"));
        };
        let children = inline.finish();
        let text = plain_text(&children);
        let bookmark = heading_bookmark(&mut self.heading_bookmarks, &text);
        self.add_block(Block::Heading(Heading {
            level,
            bookmark,
            children,
        }))
    }

    fn finish_quote(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::BlockQuote { callout, children }) = self.stack.pop() else {
            return Err(parse_error("blockquote ended without a blockquote"));
        };
        self.add_block(Block::Blockquote(Blockquote { callout, children }))
    }

    fn finish_code(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Code {
            language,
            meta,
            mut value,
        }) = self.stack.pop()
        else {
            return Err(parse_error("code block ended without a code block"));
        };
        trim_trailing_newline(&mut value);
        let key = language
            .as_deref()
            .map(|token| token.trim().to_ascii_lowercase());
        if matches!(key.as_deref(), Some("chart" | "chartjs")) {
            if let Some(chart) = self.chart.clone() {
                return self.finish_chart(&value, &chart);
            }
        }
        if key.as_deref() == Some("mermaid") {
            if let Some(mermaid) = self.mermaid.clone() {
                return self.finish_mermaid(&value, language, meta, &mermaid);
            }
        }
        if key.as_deref() == Some("table") {
            return self.finish_rich_table(&value);
        }
        self.add_block(Block::CodeBlock(self.paint_code(language, value)))
    }

    /// Theme colors when highlighting is on. `highlight_code` runs only then,
    /// and only for a language the whitelist allows. A span with no color
    /// stays off `highlights`, so the writer keeps one plain run.
    fn paint_code(&self, language: Option<String>, value: String) -> CodeBlock {
        let Some(pass) = &self.highlight else {
            return CodeBlock::plain(language, value);
        };
        let mut block = CodeBlock::plain(language, value);
        block.colors = Some(pass.colors());
        if let Some(language) = block.language.as_deref() {
            if pass.allows(language) {
                let spans = crate::highlight::highlight_code(&block.value, language);
                if spans.iter().any(|span| span.color.is_some()) {
                    block.highlights = spans;
                }
            }
        }
        block
    }

    fn apply_replacements(&self, text: &str) -> String {
        if self.replacements.is_empty() {
            return text.to_string();
        }
        let mut current = text.to_string();
        for replacement in &self.replacements {
            current = replacement.apply(&current);
        }
        current
    }

    fn finish_rich_table(&mut self, value: &str) -> Result<(), MarkdownConversionError> {
        if self.table_depth >= 8 {
            return Err(MarkdownConversionError::new(
                "Rich table nesting limit exceeded",
            ));
        }
        let definition = parse_rich_table(value)?;
        let mut logical: Vec<&[RichCell]> = Vec::new();
        let has_header = definition
            .headers
            .as_ref()
            .is_some_and(|row| !row.is_empty());
        if has_header {
            if let Some(header) = definition.headers.as_deref() {
                logical.push(header);
            }
        }
        for row in &definition.rows {
            logical.push(row);
        }
        validate_table_tiling(&logical, definition.widths.as_deref())?;
        let header = if has_header {
            self.rich_row(definition.headers.unwrap_or_default())?
        } else {
            Vec::new()
        };
        let mut rows = Vec::with_capacity(definition.rows.len());
        for row in definition.rows {
            rows.push(self.rich_row(row)?);
        }
        let mut table = Table::gfm(Vec::new(), header, rows);
        table.column_widths = definition.widths;
        self.add_block(Block::Table(table))
    }

    fn rich_row(&mut self, row: Vec<RichCell>) -> Result<Vec<TableCell>, MarkdownConversionError> {
        let mut cells = Vec::with_capacity(row.len());
        for cell in row {
            let blocks = self.parse_fragment(&cell.markdown)?;
            cells.push(TableCell {
                children: Vec::new(),
                blocks,
                column_span: cell.column_span,
                row_span: cell.row_span,
            });
        }
        Ok(cells)
    }

    fn parse_fragment(&mut self, markdown: &str) -> Result<Vec<Block>, MarkdownConversionError> {
        let mut nested = self.detach_child();
        let driven = nested.drive(markdown);
        let result = match driven {
            Ok(()) => nested.take_root(),
            Err(err) => Err(err),
        };
        self.attach_child(nested);
        result
    }

    fn drive(&mut self, markdown: &str) -> Result<(), MarkdownConversionError> {
        let math_enabled = self.math_enabled;
        let events = TextMergeStream::new(Parser::new_ext(markdown, parser_options(math_enabled)));
        for event in events {
            self.on_event(event)?;
        }
        Ok(())
    }

    fn detach_child(&mut self) -> Walker {
        Walker {
            stack: vec![Frame::Root {
                children: Vec::new(),
            }],
            marks: MarkCounts::default(),
            next_sequence: self.next_sequence,
            footnote_base: self.footnote_base,
            footnote_definitions: std::mem::take(&mut self.footnote_definitions),
            referenced_footnotes: std::mem::take(&mut self.referenced_footnotes),
            footnote_numbers: std::mem::take(&mut self.footnote_numbers),
            heading_bookmarks: std::mem::replace(
                &mut self.heading_bookmarks,
                HeadingBookmarks::new(),
            ),
            images: std::mem::take(&mut self.images),
            missing_alt: self.missing_alt,
            math_enabled: self.math_enabled,
            math_throw: self.math_throw,
            chart: self.chart.clone(),
            mermaid: self.mermaid.clone(),
            highlight: self.highlight.clone(),
            replacements: self.replacements.clone(),
            table_depth: self.table_depth.saturating_add(1),
        }
    }

    fn attach_child(&mut self, child: Walker) {
        self.next_sequence = child.next_sequence;
        self.footnote_base = child.footnote_base;
        self.footnote_definitions = child.footnote_definitions;
        self.referenced_footnotes = child.referenced_footnotes;
        self.footnote_numbers = child.footnote_numbers;
        self.heading_bookmarks = child.heading_bookmarks;
        self.images = child.images;
    }

    fn take_root(&mut self) -> Result<Vec<Block>, MarkdownConversionError> {
        if self.stack.len() != 1 {
            return Err(parse_error("unclosed Markdown block"));
        }
        match self.stack.pop() {
            Some(Frame::Root { children }) => Ok(children),
            _ => Err(parse_error("unclosed Markdown block")),
        }
    }

    fn finish_chart(
        &mut self,
        value: &str,
        chart: &ChartPass,
    ) -> Result<(), MarkdownConversionError> {
        // These are the sizes `render_chart` uses when the JSON omits that
        // axis. A JSON width or height replaces the argument. A value above
        // 2000 is already an error from the rasterizer.
        let rendered = match &chart.renderer {
            Some(renderer) => renderer.render(value, chart.width, chart.height),
            None => crate::charts::render_chart(value, chart.width, chart.height),
        };
        match rendered {
            Ok(bytes) => {
                let (width_px, height_px) =
                    png_pixel_size(&bytes).unwrap_or((chart.width, chart.height));
                self.add_block(Block::ChartImage {
                    bytes,
                    width_px,
                    height_px,
                    alt: chart_drawing_alt(value),
                })
            }
            Err(err) if chart.throw_invalid || chart_error_stops_conversion(&err) => {
                Err(invalid_chart(err))
            }
            Err(err) => self.add_block(note_paragraph(format!(
                "[Chart could not be rendered: {}]",
                err.message()
            ))),
        }
    }

    fn finish_mermaid(
        &mut self,
        value: &str,
        language: Option<String>,
        meta: Option<String>,
        mermaid: &MermaidPass,
    ) -> Result<(), MarkdownConversionError> {
        let Some(renderer) = &mermaid.renderer else {
            return self.finish_mermaid_failure(value, language, None, mermaid);
        };
        let rendered = renderer.render(&MermaidRenderInput {
            code: value.to_string(),
            meta,
        });
        match rendered {
            Ok(Some(image)) => self.finish_mermaid_image(image),
            Ok(None) => self.finish_mermaid_failure(
                value,
                language,
                Some("Mermaid renderer returned no image"),
                mermaid,
            ),
            Err(err) => self.finish_mermaid_failure(value, language, Some(err.message()), mermaid),
        }
    }

    fn finish_mermaid_image(
        &mut self,
        image: crate::options::MermaidRenderOutput,
    ) -> Result<(), MarkdownConversionError> {
        let alt = image.alt;
        if self.missing_alt == MissingImageAltText::Throw && alt.trim().is_empty() {
            return Err(MarkdownConversionError::new(
                "Missing image alt text in Mermaid renderer result; strict accessibility mode requires a non-empty description",
            ));
        }
        let content_type = if image.content_type.is_empty() {
            "image/png".to_string()
        } else {
            image.content_type
        };
        self.add_block(Block::Image(Image {
            bytes: Some(image.bytes).filter(|bytes| !bytes.is_empty()),
            content_type,
            width_px: image.width_px.unwrap_or(0),
            height_px: image.height_px.unwrap_or(0),
            alt,
            title: image.title.filter(|title| !title.is_empty()),
            url: String::new(),
        }))
    }

    fn finish_mermaid_failure(
        &mut self,
        value: &str,
        language: Option<String>,
        reason: Option<&str>,
        mermaid: &MermaidPass,
    ) -> Result<(), MarkdownConversionError> {
        match mermaid.failure {
            MermaidFailureMode::Throw => Err(MarkdownConversionError::new(match reason {
                Some(reason) if !reason.is_empty() => {
                    format!("Failed to render Mermaid diagram: {reason}")
                }
                _ => "Failed to render Mermaid diagram".to_string(),
            })),
            MermaidFailureMode::Placeholder => self.add_block(note_paragraph(
                "[Mermaid diagram could not be rendered]".to_string(),
            )),
            MermaidFailureMode::CodeBlock | MermaidFailureMode::Invalid => self.add_block(
                Block::CodeBlock(self.paint_code(language, value.to_string())),
            ),
        }
    }

    fn finish_list(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::List {
            ordered,
            start,
            sequence_id,
            items,
        }) = self.stack.pop()
        else {
            return Err(parse_error("list ended without a list"));
        };
        self.add_block(Block::List(List {
            ordered,
            start,
            sequence_id,
            children: items,
        }))
    }

    fn finish_item(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Item {
            checked,
            marker,
            mut children,
            inline,
        }) = self.stack.pop()
        else {
            return Err(parse_error("list item ended without a list item"));
        };
        let runs = inline.finish();
        if !runs.is_empty() {
            children.push(Block::Paragraph(Paragraph { children: runs }));
        }
        if children.is_empty() {
            children.push(Block::Paragraph(Paragraph {
                children: Vec::new(),
            }));
        }
        if let Some(prefix) = marker {
            prefix_task(&mut children, prefix);
        }
        match self.stack.last_mut() {
            Some(Frame::List { items, .. }) => {
                items.push(ListItem { checked, children });
                Ok(())
            }
            _ => Err(parse_error("list item outside a list")),
        }
    }

    fn finish_table(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Table {
            align,
            header,
            rows,
            ..
        }) = self.stack.pop()
        else {
            return Err(parse_error("table ended without a table"));
        };
        self.add_block(Block::Table(Table::gfm(align, header, rows)))
    }

    fn finish_cell(&mut self) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Cell { inline }) = self.stack.pop() else {
            return Err(parse_error("table cell ended without a table cell"));
        };
        let cell = TableCell::text(inline.finish());
        match self.stack.last_mut() {
            Some(Frame::Table { row, .. }) => {
                row.push(cell);
                Ok(())
            }
            _ => Err(parse_error("table cell outside a table")),
        }
    }

    fn add_block(&mut self, block: Block) -> Result<(), MarkdownConversionError> {
        self.flush_tight_item();
        match self.stack.last_mut() {
            Some(
                Frame::Root { children }
                | Frame::BlockQuote { children, .. }
                | Frame::Item { children, .. }
                | Frame::Footnote { children, .. },
            ) => {
                children.push(block);
                Ok(())
            }
            _ => Err(parse_error("block ended outside a container")),
        }
    }

    fn flush_tight_item(&mut self) {
        let Some(Frame::Item { inline, .. }) = self.stack.last_mut() else {
            return;
        };
        let runs = inline.take_runs();
        if runs.is_empty() {
            return;
        }
        if let Some(Frame::Item { children, .. }) = self.stack.last_mut() {
            children.push(Block::Paragraph(Paragraph { children: runs }));
        }
    }

    fn push_mark(&mut self, kind: MarkKind) -> Result<(), MarkdownConversionError> {
        self.marks.bump(kind, 1);
        self.sync_marks();
        Ok(())
    }

    fn pop_mark(&mut self, kind: MarkKind) -> Result<(), MarkdownConversionError> {
        self.marks.bump(kind, -1);
        self.sync_marks();
        Ok(())
    }

    fn sync_marks(&mut self) {
        let marks = self.marks.current();
        if let Some(inline) = self.current_inline_mut() {
            inline.set_marks(marks);
        }
    }

    fn fresh_inline(&self) -> InlineBuf {
        InlineBuf::new(self.marks.current())
    }

    fn current_inline(&mut self) -> Result<&mut InlineBuf, MarkdownConversionError> {
        self.current_inline_mut()
            .ok_or_else(|| parse_error("inline content outside an inline container"))
    }

    fn current_inline_mut(&mut self) -> Option<&mut InlineBuf> {
        match self.stack.last_mut() {
            Some(
                Frame::Paragraph { inline, .. }
                | Frame::Heading { inline, .. }
                | Frame::Cell { inline }
                | Frame::Item { inline, .. },
            ) => Some(inline),
            _ => None,
        }
    }

    fn inline_ref(&self) -> Option<&InlineBuf> {
        match self.stack.last() {
            Some(
                Frame::Paragraph { inline, .. }
                | Frame::Heading { inline, .. }
                | Frame::Cell { inline }
                | Frame::Item { inline, .. },
            ) => Some(inline),
            _ => None,
        }
    }

    /// True when this soft break sits in a plain quote whose text so far is
    /// only an unsupported callout marker, such as `[!INFO]`.
    fn unsupported_callout_soft_break(&self) -> bool {
        let in_plain_quote = self
            .stack
            .iter()
            .rev()
            .find_map(|frame| match frame {
                Frame::BlockQuote { callout, .. } => Some(callout.is_none()),
                _ => None,
            })
            .unwrap_or(false);
        if !in_plain_quote {
            return false;
        }
        let Some(inline) = self.inline_ref() else {
            return false;
        };
        inline.runs.is_empty() && is_unsupported_callout_marker(&inline.pending)
    }

    fn in_skip(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Skip { .. }))
    }

    fn on_skipped_event(&mut self, event: Event<'_>) -> Result<(), MarkdownConversionError> {
        let Some(Frame::Skip { depth }) = self.stack.last_mut() else {
            return Ok(());
        };
        match event {
            Event::Start(_) => *depth = depth.saturating_add(1),
            Event::End(_) => {
                *depth = depth.saturating_sub(1);
                if *depth == 0 {
                    self.stack.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

enum Frame {
    Root {
        children: Vec<Block>,
    },
    BlockQuote {
        callout: Option<CalloutKind>,
        children: Vec<Block>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        sequence_id: Option<u32>,
        items: Vec<ListItem>,
    },
    Item {
        checked: Option<bool>,
        marker: Option<&'static str>,
        children: Vec<Block>,
        inline: InlineBuf,
    },
    Paragraph {
        inline: InlineBuf,
        /// Set when this paragraph is exactly one image.
        image: Option<Image>,
        /// Set when this paragraph is exactly one display-math expression.
        display: Option<String>,
    },
    ImageAlt {
        depth: u32,
        url: String,
        title: Option<String>,
        alt: String,
        sole: bool,
    },
    Heading {
        level: u8,
        inline: InlineBuf,
    },
    Code {
        language: Option<String>,
        meta: Option<String>,
        value: String,
    },
    Table {
        align: Vec<TableAlign>,
        header: Vec<TableCell>,
        rows: Vec<Vec<TableCell>>,
        row: Vec<TableCell>,
    },
    Cell {
        inline: InlineBuf,
    },
    Html {
        value: String,
    },
    Footnote {
        id: String,
        children: Vec<Block>,
    },
    Skip {
        depth: u32,
    },
}

#[derive(Clone, Copy)]
enum MarkKind {
    Bold,
    Italic,
    Strike,
}

#[derive(Clone, PartialEq, Eq)]
struct Marks {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<String>,
}

impl Marks {
    fn run(&self, text: impl Into<String>) -> Inline {
        Inline {
            text: text.into(),
            bold: self.bold,
            italic: self.italic,
            underline: false,
            strike: self.strike,
            code: false,
            link: self.link.clone(),
            footnote: None,
            cross_ref: None,
            math_omml: None,
        }
    }
}

#[derive(Default)]
struct MarkCounts {
    bold: i32,
    italic: i32,
    strike: i32,
    links: Vec<String>,
}

impl MarkCounts {
    fn bump(&mut self, kind: MarkKind, delta: i32) {
        let slot = match kind {
            MarkKind::Bold => &mut self.bold,
            MarkKind::Italic => &mut self.italic,
            MarkKind::Strike => &mut self.strike,
        };
        *slot = slot.saturating_add(delta).max(0);
    }

    fn current(&self) -> Marks {
        Marks {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            link: self.links.last().cloned(),
        }
    }
}

struct InlineBuf {
    runs: Vec<Inline>,
    pending: String,
    marks: Marks,
}

impl InlineBuf {
    fn new(marks: Marks) -> Self {
        Self {
            runs: Vec::new(),
            pending: String::new(),
            marks,
        }
    }

    fn set_marks(&mut self, marks: Marks) {
        if self.marks != marks {
            self.flush();
            self.marks = marks;
        }
    }

    fn push_text(&mut self, text: &str) {
        self.pending.push_str(text);
    }

    fn push_hard_break(&mut self) {
        self.flush();
        self.runs.push(self.marks.run("\n"));
    }

    fn push_footnote(&mut self, mark: FootnoteMark) {
        self.flush();
        self.runs.push(Inline {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            code: false,
            link: None,
            footnote: Some(mark),
            cross_ref: None,
            math_omml: None,
        });
    }

    fn push_code(&mut self, text: &str) {
        self.flush();
        let mut run = self.marks.run(text);
        run.code = true;
        self.runs.push(run);
    }

    fn push_math(&mut self, omml: String) {
        self.flush();
        let mut run = self.marks.run("");
        run.math_omml = Some(omml);
        self.runs.push(run);
    }

    fn take_runs(&mut self) -> Vec<Inline> {
        self.flush();
        std::mem::take(&mut self.runs)
    }

    fn append_runs(&mut self, mut runs: Vec<Inline>) {
        self.flush();
        self.runs.append(&mut runs);
    }

    fn push_run(&mut self, run: Inline) {
        self.flush();
        self.runs.push(run);
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush();
        self.runs
    }

    fn is_blank(&self) -> bool {
        self.pending.is_empty() && runs_are_blank(&self.runs)
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        push_text_with_underline(&text, &self.marks, &mut self.runs);
    }
}

/// Split on `(\+\+[^+\n][\s\S]*?\+\+)`, the same pattern as
/// `pushTextWithUnderline` in `src/mdastToDocxModel.ts`.
fn push_text_with_underline(value: &str, marks: &Marks, out: &mut Vec<Inline>) {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    let mut index = 0;
    while index + 4 < bytes.len() {
        if underline_opens_at(bytes, index) {
            if let Some(end) = find_closing_plus(bytes, index + 3) {
                let matched = &value[index..end];
                if matched.chars().count() > 4 {
                    if cursor < index {
                        out.push(marks.run(&value[cursor..index]));
                    }
                    let mut run = marks.run(&value[index + 2..end - 2]);
                    run.underline = true;
                    out.push(run);
                    cursor = end;
                    index = end;
                    continue;
                }
            }
        }
        index += 1;
    }
    if cursor < value.len() {
        out.push(marks.run(&value[cursor..]));
    }
}

fn is_unsupported_callout_marker(text: &str) -> bool {
    let text = text.trim();
    let Some(name) = text
        .strip_prefix("[!")
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return false;
    };
    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return false;
    }
    !matches!(
        name.to_ascii_uppercase().as_str(),
        "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION"
    )
}

fn underline_opens_at(bytes: &[u8], index: usize) -> bool {
    index + 2 < bytes.len()
        && bytes[index] == b'+'
        && bytes[index + 1] == b'+'
        && bytes[index + 2] != b'+'
        && bytes[index + 2] != b'\n'
}

fn find_closing_plus(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index + 1 < bytes.len() {
        if bytes[index] == b'+' && bytes[index + 1] == b'+' {
            return Some(index + 2);
        }
        index += 1;
    }
    None
}

fn prefix_task(children: &mut Vec<Block>, prefix: &str) {
    let run = Inline {
        text: prefix.to_string(),
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        code: false,
        link: None,
        footnote: None,
        cross_ref: None,
        math_omml: None,
    };
    if let Some(Block::Paragraph(paragraph)) = children
        .iter_mut()
        .find(|block| matches!(block, Block::Paragraph(_)))
    {
        paragraph.children.insert(0, run);
        return;
    }
    children.insert(
        0,
        Block::Paragraph(Paragraph {
            children: vec![run],
        }),
    );
}

fn fence_parts(kind: &CodeBlockKind<'_>) -> (Option<String>, Option<String>) {
    let CodeBlockKind::Fenced(info) = kind else {
        return (None, None);
    };
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let (language, meta) = match trimmed.split_once(char::is_whitespace) {
        Some((language, rest)) => {
            let meta = rest.trim();
            (
                language.to_string(),
                (!meta.is_empty()).then(|| meta.to_string()),
            )
        }
        None => (trimmed.to_string(), None),
    };
    (Some(language), meta)
}

fn trim_trailing_newline(value: &mut String) {
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
}

fn fit_row(mut row: Vec<TableCell>, width: usize) -> Vec<TableCell> {
    if width == 0 {
        return row;
    }
    row.truncate(width);
    while row.len() < width {
        row.push(TableCell::text(Vec::new()));
    }
    row
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn callout_kind(kind: BlockQuoteKind) -> CalloutKind {
    match kind {
        BlockQuoteKind::Note => CalloutKind::Note,
        BlockQuoteKind::Tip => CalloutKind::Tip,
        BlockQuoteKind::Important => CalloutKind::Important,
        BlockQuoteKind::Warning => CalloutKind::Warning,
        BlockQuoteKind::Caution => CalloutKind::Caution,
    }
}

fn table_align(align: Alignment) -> TableAlign {
    match align {
        Alignment::None => TableAlign::None,
        Alignment::Left => TableAlign::Left,
        Alignment::Center => TableAlign::Center,
        Alignment::Right => TableAlign::Right,
    }
}

fn parse_error(message: &str) -> MarkdownConversionError {
    MarkdownConversionError::new(message)
}

const DEFAULT_CHART_WIDTH: u32 = 640;
const DEFAULT_CHART_HEIGHT: u32 = 360;

enum MathPiece {
    Omml(String),
    Text(String),
}

#[derive(Clone)]
struct ChartPass {
    width: u32,
    height: u32,
    throw_invalid: bool,
    renderer: Option<ChartRenderer>,
}

impl ChartPass {
    fn from_options(chart: &ChartRenderingOptions) -> Self {
        Self {
            width: positive_or(chart.width, DEFAULT_CHART_WIDTH),
            height: positive_or(chart.height, DEFAULT_CHART_HEIGHT),
            throw_invalid: matches!(
                chart.invalid_definition_behavior,
                Some(ChartInvalidBehavior::Throw)
            ),
            renderer: chart.renderer.clone(),
        }
    }
}

#[derive(Clone)]
struct MermaidPass {
    renderer: Option<MermaidRenderer>,
    failure: MermaidFailureMode,
}

impl MermaidPass {
    fn from_options(mermaid: &MermaidRenderingOptions) -> Self {
        Self {
            renderer: mermaid.render.clone(),
            failure: match mermaid.failure_mode {
                Some(
                    mode @ (MermaidFailureMode::CodeBlock
                    | MermaidFailureMode::Placeholder
                    | MermaidFailureMode::Throw),
                ) => mode,
                Some(MermaidFailureMode::Invalid) | None => MermaidFailureMode::CodeBlock,
            },
        }
    }
}

fn positive_or(value: Option<u32>, default: u32) -> u32 {
    match value {
        Some(value) if value > 0 => value,
        Some(_) | None => default,
    }
}

fn unsupported_math(_source: &str, _err: MarkdownConversionError) -> MarkdownConversionError {
    MarkdownConversionError::new("Unsupported math expression")
}

fn literal_math(source: &str, display: bool) -> String {
    if display {
        format!("$$\n{source}\n$$")
    } else {
        format!("${source}$")
    }
}

fn math_inline(omml: String) -> Inline {
    Inline {
        text: String::new(),
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        code: false,
        link: None,
        footnote: None,
        cross_ref: None,
        math_omml: Some(omml),
    }
}

fn note_paragraph(text: String) -> Block {
    Block::Paragraph(Paragraph {
        children: vec![Inline {
            text,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            code: false,
            link: None,
            footnote: None,
            cross_ref: None,
            math_omml: None,
        }],
    })
}

fn invalid_chart(err: MarkdownConversionError) -> MarkdownConversionError {
    let message = err.message();
    if let Some(rest) = message.strip_prefix("Invalid chart block:") {
        if rest.starts_with(' ') || rest.is_empty() {
            return err;
        }
    }
    MarkdownConversionError::new(format!("Invalid chart block: {message}"))
}

fn chart_error_stops_conversion(err: &MarkdownConversionError) -> bool {
    let message = err.message();
    message.contains("strict accessibility")
        || message.contains("not valid in OOXML")
        || message.contains("maximum length")
}

fn png_pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// Drawing description for a chart fence. A root `alt` wins. Otherwise the
/// chart `type` is the description, matching `processChartBlock`.
fn chart_drawing_alt(definition: &str) -> String {
    let alt = root_json_string(definition, "alt").unwrap_or_default();
    if !alt.is_empty() {
        return alt;
    }
    root_json_string(definition, "type").unwrap_or_default()
}

fn root_json_string(definition: &str, key: &str) -> Option<String> {
    let bytes = definition.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if bytes.get(index) != Some(&b'{') {
        return None;
    }
    index += 1;
    let mut depth = 1_i32;
    while index < bytes.len() && depth > 0 {
        match bytes[index] {
            b'"' => {
                let (text, next) = json_string(definition, index)?;
                if depth == 1 && text == key {
                    let mut cursor = next;
                    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                        cursor += 1;
                    }
                    if bytes.get(cursor) == Some(&b':') {
                        cursor += 1;
                        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                            cursor += 1;
                        }
                        if bytes.get(cursor) == Some(&b'"') {
                            return json_string(definition, cursor).map(|(value, _)| value);
                        }
                    }
                }
                index = next;
            }
            b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn json_string(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut index = start + 1;
    let mut out = String::new();
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                let escaped = *bytes.get(index + 1)?;
                match escaped {
                    b'"' | b'\\' | b'/' => out.push(escaped as char),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = source.get(index + 2..index + 6)?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        out.push(char::from_u32(code)?);
                        index += 6;
                        continue;
                    }
                    _ => return None,
                }
                index += 2;
            }
            b'"' => return Some((out, index + 1)),
            _ => {
                let ch = source[index..].chars().next()?;
                out.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    None
}

const HEADER_FOOTER_FOOTNOTE: &str = "Word headers and footers cannot contain footnotes";

fn parse_header_footer(
    group: &ResolvedHeaderFooterInput,
    cursor: &mut ParseCursor,
) -> Result<ResolvedHeaderFooter, MarkdownConversionError> {
    Ok(ResolvedHeaderFooter {
        default: parse_slot(group.default.as_ref(), cursor)?,
        first: parse_slot(group.first.as_ref(), cursor)?,
        even: parse_slot(group.even.as_ref(), cursor)?,
    })
}

fn parse_slot(
    slot: Option<&ResolvedSlotInput>,
    cursor: &mut ParseCursor,
) -> Result<Option<HeaderFooterBody>, MarkdownConversionError> {
    let Some(slot) = slot else {
        return Ok(None);
    };
    let blocks = match slot.content.markdown.as_deref() {
        Some(markdown) if !markdown.is_empty() => {
            let document = parse_with(markdown, cursor)?;
            if contains_footnote(&document.children) {
                return Err(MarkdownConversionError::new(HEADER_FOOTER_FOOTNOTE));
            }
            document.children
        }
        _ => Vec::new(),
    };
    Ok(Some(HeaderFooterBody {
        blocks,
        text: slot.content.text.clone(),
        alignment: slot.content.alignment,
        page_number_display: slot.content.page_number_display,
    }))
}

fn lift_toc(children: Vec<Block>) -> Vec<SectionBlock> {
    let mut blocks: Vec<SectionBlock> = children
        .into_iter()
        .map(|block| {
            if is_toc_paragraph(&block) {
                SectionBlock::TableOfContents(TocBlock {
                    primary: false,
                    headings: Vec::new(),
                    settings: TocSettings::default(),
                })
            } else {
                SectionBlock::Body(block)
            }
        })
        .collect();
    let has_body = blocks.iter().any(|block| match block {
        SectionBlock::TableOfContents(_) => true,
        SectionBlock::Body(Block::FootnoteDefinition { .. }) => false,
        SectionBlock::Body(_) => true,
    });
    if !has_body {
        let insert_at = blocks
            .iter()
            .position(|block| matches!(block, SectionBlock::Body(Block::FootnoteDefinition { .. })))
            .unwrap_or(blocks.len());
        blocks.insert(
            insert_at,
            SectionBlock::Body(Block::Paragraph(Paragraph {
                children: Vec::new(),
            })),
        );
    }
    blocks
}

fn is_toc_paragraph(block: &Block) -> bool {
    let Block::Paragraph(paragraph) = block else {
        return false;
    };
    plain_text(&paragraph.children).trim() == "[TOC]"
}

fn collect_headings(sections: &[PreparedSection]) -> Vec<TocHeading> {
    let mut headings = Vec::new();
    for section in sections {
        for block in &section.children {
            if let SectionBlock::Body(block) = block {
                collect_block_headings(block, &mut headings);
            }
        }
    }
    headings
}

fn collect_block_headings(block: &Block, headings: &mut Vec<TocHeading>) {
    match block {
        Block::Heading(heading) => headings.push(TocHeading {
            text: plain_text(&heading.children),
            level: heading.level,
            bookmark: heading.bookmark.clone(),
        }),
        Block::List(list) => {
            for item in &list.children {
                for child in &item.children {
                    collect_block_headings(child, headings);
                }
            }
        }
        Block::Blockquote(quote) => {
            for child in &quote.children {
                collect_block_headings(child, headings);
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

fn filter_headings(headings: &[TocHeading], toc: &TocSettings) -> Vec<TocHeading> {
    headings
        .iter()
        .filter(|heading| heading.level >= toc.min_depth && heading.level <= toc.max_depth)
        .cloned()
        .collect()
}

fn contains_footnote(blocks: &[Block]) -> bool {
    blocks.iter().any(block_has_footnote)
}

fn block_has_footnote(block: &Block) -> bool {
    match block {
        Block::FootnoteDefinition { .. } => true,
        Block::Paragraph(paragraph) => inlines_have_footnote(&paragraph.children),
        Block::Heading(heading) => inlines_have_footnote(&heading.children),
        Block::CodeBlock(_)
        | Block::ThematicBreak
        | Block::PageBreak
        | Block::Comment(_)
        | Block::Image(_)
        | Block::DisplayMath { .. }
        | Block::ChartImage { .. } => false,
        Block::Caption(caption) => inlines_have_footnote(&caption.children),
        Block::List(list) => list
            .children
            .iter()
            .any(|item| item.children.iter().any(block_has_footnote)),
        Block::Blockquote(quote) => quote.children.iter().any(block_has_footnote),
        Block::Table(table) => table
            .header
            .iter()
            .chain(table.rows.iter().flatten())
            .any(|cell| inlines_have_footnote(&cell.children)),
    }
}

fn inlines_have_footnote(runs: &[Inline]) -> bool {
    runs.iter().any(|run| run.footnote.is_some())
}

fn plain_text(runs: &[Inline]) -> String {
    let mut text = String::new();
    for run in runs {
        if run.footnote.is_none() {
            text.push_str(&run.text);
        }
    }
    text
}

fn is_root_pagebreak(stack: &[Frame], runs: &[Inline]) -> bool {
    if !matches!(stack.last(), Some(Frame::Root { .. })) {
        return false;
    }
    plain_text(runs).trim() == "\\pagebreak"
}

fn normalize_footnote_id(label: &str) -> String {
    label.trim().to_lowercase()
}

/// `classifyHtmlNode` from `src/mdastToDocxModel.ts`.
///
/// `<!-- COMMENT: text -->` is a comment. Any other HTML block whose text
/// contains `pagebreak` is a page break. Anything else is dropped.
fn classify_html(value: &str) -> Option<Block> {
    if let Some(comment) = html_comment(value) {
        return Some(Block::Comment(comment));
    }
    if value.contains("pagebreak") {
        return Some(Block::PageBreak);
    }
    None
}

fn html_comment(value: &str) -> Option<String> {
    let rest = value.trim_start().strip_prefix("<!--")?;
    let rest = rest.trim_start().strip_prefix("COMMENT:")?;
    let rest = rest.trim_start();
    let end = rest.find("-->").unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn footnote_children(children: Vec<Block>) -> Vec<Block> {
    let mut blocks = Vec::new();
    for child in children {
        match child {
            Block::Table(table) => blocks.extend(flatten_footnote_table(table)),
            other => blocks.push(other),
        }
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph {
            children: Vec::new(),
        }));
    }
    blocks
}

fn flatten_footnote_table(table: Table) -> Vec<Block> {
    let mut rows = Vec::with_capacity(table.rows.len().saturating_add(1));
    rows.push(table.header);
    rows.extend(table.rows);
    rows.into_iter()
        .map(|row| {
            let text = row
                .iter()
                .map(|cell| plain_text(&cell.children))
                .collect::<Vec<_>>()
                .join(" | ");
            Block::Paragraph(Paragraph {
                children: vec![Inline {
                    text,
                    bold: false,
                    italic: false,
                    underline: false,
                    strike: false,
                    code: false,
                    link: None,
                    footnote: None,
                    cross_ref: None,
                    math_omml: None,
                }],
            })
        })
        .collect()
}

const IMAGE_MARK: char = '\u{1}';
const MAX_IMAGE_ALT_TEXT_LENGTH: usize = 2048;
const MAX_IMAGE_TITLE_LENGTH: usize = 512;

fn inline_image_run(image: &Image) -> Inline {
    let text = match image.bytes.as_deref() {
        Some(bytes) if !bytes.is_empty() => {
            let mime = if image.content_type.is_empty() {
                "image/png"
            } else {
                image.content_type.as_str()
            };
            format!("{IMAGE_MARK}data:{mime};base64,{}", base64_encode(bytes))
        }
        _ => placeholder_text(&image.alt),
    };
    Inline {
        text,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        code: false,
        link: None,
        footnote: None,
        cross_ref: None,
        math_omml: None,
    }
}

fn placeholder_text(alt: &str) -> String {
    if alt.is_empty() {
        "[Image could not be displayed]".to_string()
    } else {
        format!("[Image could not be displayed: {alt}]")
    }
}

fn image_error_stops_conversion(err: &MarkdownConversionError) -> bool {
    let message = err.message();
    message.contains("not valid in OOXML")
        || message.contains("maximum length of")
        || message.contains("strict accessibility mode")
}

fn validate_image_text(alt: &str, title: Option<&str>) -> Result<(), MarkdownConversionError> {
    validate_ooxml_text(alt, "Markdown image alt text", MAX_IMAGE_ALT_TEXT_LENGTH)?;
    if let Some(title) = title {
        validate_ooxml_text(title, "Markdown image title", MAX_IMAGE_TITLE_LENGTH)?;
    }
    Ok(())
}

fn validate_ooxml_text(
    value: &str,
    context: &str,
    max_length: usize,
) -> Result<(), MarkdownConversionError> {
    if utf16_len(value) > max_length {
        return Err(MarkdownConversionError::new(format!(
            "{context} exceeds the maximum length of {max_length} characters"
        )));
    }
    if value.chars().any(invalid_ooxml_char) {
        return Err(MarkdownConversionError::new(format!(
            "{context} contains characters that are not valid in OOXML"
        )));
    }
    Ok(())
}

fn invalid_ooxml_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' | '\u{FFFE}'
            | '\u{FFFF}'
    )
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = u32::from(bytes[index]) << 16
            | u32::from(bytes[index + 1]) << 8
            | u32::from(bytes[index + 2]);
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(chunk & 0x3f) as usize] as char);
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        let chunk = u32::from(bytes[index]) << 16;
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let chunk = u32::from(bytes[index]) << 16 | u32::from(bytes[index + 1]) << 8;
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

fn runs_are_blank(runs: &[Inline]) -> bool {
    runs.iter()
        .all(|run| run.text.is_empty() && run.footnote.is_none() && run.math_omml.is_none())
}

fn captions_update_on_open(options: &ConvertOptions) -> bool {
    match options
        .captions
        .as_ref()
        .and_then(|captions| captions.update_fields_on_open)
    {
        Some(UpdateFieldsOnOpen::Bool(value)) => value,
        Some(UpdateFieldsOnOpen::Invalid) | None => true,
    }
}

struct CaptionPass {
    throw: bool,
    figure_above: bool,
    table_above: bool,
}

impl CaptionPass {
    fn preserve() -> Self {
        Self {
            throw: false,
            figure_above: false,
            table_above: false,
        }
    }

    fn from_options(options: &ConvertOptions) -> Self {
        let captions = options.captions.as_ref();
        Self {
            throw: captions.and_then(|captions| captions.failure_mode)
                == Some(CaptionFailureMode::Throw),
            figure_above: captions.and_then(|captions| captions.figure_placement)
                == Some(CaptionPlacement::Above),
            table_above: captions.and_then(|captions| captions.table_placement)
                == Some(CaptionPlacement::Above),
        }
    }
}

fn ends_with_caption(children: &[Block]) -> bool {
    matches!(
        children
            .iter()
            .rev()
            .find(|block| !matches!(block, Block::FootnoteDefinition { .. })),
        Some(Block::Caption(_))
    )
}

/// Bind captions in every document, then resolve `[@fig:id]` and `[@tbl:id]`.
///
/// Definitions are shared across sections, so a reference can point at a
/// caption in an earlier section. The first id wins.
fn bind_documents(
    docs: &mut [Document],
    pass: &CaptionPass,
) -> Result<bool, MarkdownConversionError> {
    let mut book = BTreeMap::new();
    for doc in docs.iter_mut() {
        bind_captions(&mut doc.children, &mut book, pass)?;
    }
    for doc in docs.iter_mut() {
        resolve_blocks(&mut doc.children, &book, pass.throw)?;
    }
    Ok(!book.is_empty())
}

fn bind_captions(
    blocks: &mut Vec<Block>,
    book: &mut BTreeMap<String, CrossRef>,
    pass: &CaptionPass,
) -> Result<(), MarkdownConversionError> {
    let mut index = 0;
    while index + 1 < blocks.len() {
        let Some(kind) = caption_anchor(&blocks[index]) else {
            index += 1;
            continue;
        };
        let read = match &blocks[index + 1] {
            Block::Paragraph(paragraph) => read_caption(paragraph, kind),
            _ => {
                index += 1;
                continue;
            }
        };
        match read {
            CaptionRead::No => index += 1,
            CaptionRead::Error(message) => {
                if pass.throw {
                    return Err(MarkdownConversionError::new(message));
                }
                index += 1;
            }
            CaptionRead::Ready { id, children } => {
                if book.contains_key(&id) {
                    if pass.throw {
                        return Err(MarkdownConversionError::new(format!(
                            "Duplicate caption identifier: {id}"
                        )));
                    }
                    index += 1;
                    continue;
                }
                let bookmark = cross_reference_bookmark_id(&id);
                book.insert(
                    id.clone(),
                    CrossRef {
                        kind,
                        id: id.clone(),
                        bookmark: bookmark.clone(),
                    },
                );
                let above = match kind {
                    CaptionKind::Figure => pass.figure_above,
                    CaptionKind::Table => pass.table_above,
                };
                blocks[index + 1] = Block::Caption(Caption {
                    kind,
                    id,
                    bookmark,
                    children,
                });
                if above {
                    blocks.swap(index, index + 1);
                }
                index += 2;
            }
        }
    }
    Ok(())
}

fn caption_anchor(block: &Block) -> Option<CaptionKind> {
    if figure_anchor(block) {
        Some(CaptionKind::Figure)
    } else if matches!(block, Block::Table(_)) {
        Some(CaptionKind::Table)
    } else {
        None
    }
}

fn figure_anchor(block: &Block) -> bool {
    if matches!(block, Block::Image(_) | Block::ChartImage { .. }) {
        return true;
    }
    let Block::Paragraph(paragraph) = block else {
        return false;
    };
    matches!(
        paragraph.children.as_slice(),
        [run]
            if run.text.starts_with(IMAGE_MARK)
                && !run.code
                && run.footnote.is_none()
                && run.cross_ref.is_none()
    )
}

enum CaptionRead {
    No,
    Error(String),
    Ready { id: String, children: Vec<Inline> },
}

fn read_caption(paragraph: &Paragraph, kind: CaptionKind) -> CaptionRead {
    let runs = &paragraph.children;
    let (Some(first), Some(last)) = (runs.first(), runs.last()) else {
        return CaptionRead::No;
    };
    if !is_caption_text(first)
        || !is_caption_text(last)
        || caption_prefix_bytes(&first.text).is_none()
    {
        return CaptionRead::No;
    }
    let Some(suffix) = caption_suffix(&last.text) else {
        let plain: String = runs.iter().map(|run| run.text.as_str()).collect();
        if plain.contains("{#") {
            return CaptionRead::Error("Malformed caption identifier".to_string());
        }
        return CaptionRead::No;
    };
    let id = suffix.id;
    let expected = match kind {
        CaptionKind::Figure => "fig:",
        CaptionKind::Table => "tbl:",
    };
    if !id.starts_with(expected) || utf16_len(&id) == utf16_len(expected) {
        let label = match kind {
            CaptionKind::Figure => "Figure",
            CaptionKind::Table => "Table",
        };
        return CaptionRead::Error(format!("{label} captions require a {expected} identifier"));
    }
    if utf16_len(&id) > 128 {
        return CaptionRead::Error(
            "Caption identifiers must be at most 128 characters".to_string(),
        );
    }
    let mut children = runs.clone();
    let last_index = children.len() - 1;
    if last_index == 0 {
        let without_prefix = strip_caption_prefix(&children[0].text);
        children[0].text = strip_caption_suffix(&without_prefix);
    } else {
        children[0].text = strip_caption_prefix(&children[0].text);
        children[last_index].text = strip_caption_suffix(&children[last_index].text);
    }
    let joined: String = children.iter().map(|run| run.text.as_str()).collect();
    if joined.trim().is_empty() {
        return CaptionRead::Error("Captions must contain text".to_string());
    }
    CaptionRead::Ready { id, children }
}

fn is_caption_text(run: &Inline) -> bool {
    !run.code
        && run.footnote.is_none()
        && run.cross_ref.is_none()
        && run.link.is_none()
        && !run.text.starts_with(IMAGE_MARK)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn caption_prefix_bytes(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if first != ':' {
        return None;
    }
    let mut end = 1;
    let mut saw_space = false;
    for (index, ch) in chars {
        if js_space(ch) {
            saw_space = true;
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    saw_space.then_some(end)
}

fn strip_caption_prefix(text: &str) -> String {
    match caption_prefix_bytes(text) {
        Some(len) => text[len..].to_string(),
        None => text.to_string(),
    }
}

struct CaptionSuffix {
    start: usize,
    id: String,
}

fn caption_suffix(text: &str) -> Option<CaptionSuffix> {
    let end = trim_end_js_bytes(text);
    if end == 0 || !text[..end].ends_with('}') {
        return None;
    }
    let body = &text[..end - '}'.len_utf8()];
    let hash = body.rfind("{#")?;
    let id = &body[hash + 2..];
    if id.is_empty() || id.chars().any(|ch| js_space(ch) || ch == '{' || ch == '}') {
        return None;
    }
    let before = &body[..hash];
    let mut start = None;
    for (index, ch) in before.char_indices().rev() {
        if js_space(ch) {
            start = Some(index);
        } else {
            break;
        }
    }
    Some(CaptionSuffix {
        start: start?,
        id: id.to_string(),
    })
}

fn strip_caption_suffix(text: &str) -> String {
    match caption_suffix(text) {
        Some(suffix) => text[..suffix.start].to_string(),
        None => text.to_string(),
    }
}

fn trim_end_js_bytes(text: &str) -> usize {
    let mut end = text.len();
    while end > 0 {
        let Some(ch) = text[..end].chars().next_back() else {
            break;
        };
        if !js_space(ch) {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

fn js_space(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | ' '
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    ) || ('\u{2000}'..='\u{200A}').contains(&ch)
}

fn resolve_blocks(
    blocks: &mut [Block],
    book: &BTreeMap<String, CrossRef>,
    throw: bool,
) -> Result<(), MarkdownConversionError> {
    for block in blocks {
        resolve_block(block, book, throw)?;
    }
    Ok(())
}

fn resolve_block(
    block: &mut Block,
    book: &BTreeMap<String, CrossRef>,
    throw: bool,
) -> Result<(), MarkdownConversionError> {
    match block {
        Block::Paragraph(paragraph) => resolve_inlines(&mut paragraph.children, book, throw),
        Block::Heading(heading) => resolve_inlines(&mut heading.children, book, throw),
        Block::Caption(caption) => resolve_inlines(&mut caption.children, book, throw),
        Block::List(list) => {
            for item in &mut list.children {
                resolve_blocks(&mut item.children, book, throw)?;
            }
            Ok(())
        }
        Block::Blockquote(quote) => resolve_blocks(&mut quote.children, book, throw),
        Block::Table(table) => {
            for cell in table
                .header
                .iter_mut()
                .chain(table.rows.iter_mut().flatten())
            {
                resolve_inlines(&mut cell.children, book, throw)?;
            }
            Ok(())
        }
        Block::FootnoteDefinition { children, .. } => resolve_blocks(children, book, throw),
        Block::CodeBlock(_)
        | Block::ThematicBreak
        | Block::PageBreak
        | Block::Comment(_)
        | Block::Image(_)
        | Block::DisplayMath { .. }
        | Block::ChartImage { .. } => Ok(()),
    }
}

fn resolve_inlines(
    runs: &mut Vec<Inline>,
    book: &BTreeMap<String, CrossRef>,
    throw: bool,
) -> Result<(), MarkdownConversionError> {
    let original = std::mem::take(runs);
    for run in original {
        if run.code
            || run.footnote.is_some()
            || run.cross_ref.is_some()
            || run.text.starts_with(IMAGE_MARK)
        {
            runs.push(run);
            continue;
        }
        expand_run(run, book, throw, runs)?;
    }
    Ok(())
}

fn expand_run(
    run: Inline,
    book: &BTreeMap<String, CrossRef>,
    throw: bool,
    out: &mut Vec<Inline>,
) -> Result<(), MarkdownConversionError> {
    let mut rest = run.text.as_str();
    let mut produced = false;
    loop {
        let found = find_ref(rest).map(|(start, end, id)| (start, end, id.to_string()));
        let Some((start, end, id)) = found else {
            break;
        };
        if start > 0 {
            let mut piece = run.clone();
            piece.text = rest[..start].to_string();
            piece.cross_ref = None;
            out.push(piece);
        }
        match book.get(&id) {
            Some(cross) => out.push(cross_run(cross)),
            None if throw => {
                return Err(MarkdownConversionError::new(format!(
                    "Unresolved cross-reference: {id}"
                )));
            }
            None => {
                let mut piece = run.clone();
                piece.text = rest[start..end].to_string();
                piece.cross_ref = None;
                out.push(piece);
            }
        }
        rest = &rest[end..];
        produced = true;
    }
    if !produced {
        out.push(run);
    } else if !rest.is_empty() {
        let mut piece = run.clone();
        piece.text = rest.to_string();
        piece.cross_ref = None;
        out.push(piece);
    }
    Ok(())
}

fn cross_run(cross: &CrossRef) -> Inline {
    Inline {
        text: String::new(),
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        code: false,
        link: None,
        footnote: None,
        cross_ref: Some(cross.clone()),
        math_omml: None,
    }
}

/// `\[@((?:fig|tbl):[^\]\s]+)\]`
fn find_ref(text: &str) -> Option<(usize, usize, &str)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 6 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1] == b'@' {
            let after = index + 2;
            if let Some(relative) = text[after..].find(']') {
                let id = &text[after..after + relative];
                if is_caption_ref_id(id) {
                    return Some((index, after + relative + 1, id));
                }
            }
        }
        let width = text[index..].chars().next().map_or(1, char::len_utf8);
        index += width;
    }
    None
}

fn is_caption_ref_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("fig:").or_else(|| id.strip_prefix("tbl:")) else {
        return false;
    };
    !rest.is_empty()
        && !rest
            .chars()
            .any(|ch| js_space(ch) || ch == ']' || ch == '[')
}

#[derive(Clone)]
struct HighlightPass {
    show_label: bool,
    languages: Option<Vec<String>>,
    label_color: String,
    plain_color: String,
    fill: String,
    border: String,
}

impl HighlightPass {
    fn colors(&self) -> CodeColors {
        CodeColors {
            label_color: self.label_color.clone(),
            plain_color: self.plain_color.clone(),
            fill: self.fill.clone(),
            border: self.border.clone(),
            show_label: self.show_label,
        }
    }

    fn allows(&self, language: &str) -> bool {
        let name = language.trim();
        if name.is_empty() {
            return false;
        }
        let Some(languages) = &self.languages else {
            return true;
        };
        languages
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(name))
    }
}

#[derive(Clone)]
struct PreparedReplacement {
    find: PreparedFind,
    replace: TextReplace,
}

#[derive(Clone)]
enum PreparedFind {
    Literal(String),
    Pattern(regex::Regex),
}

impl PreparedReplacement {
    fn apply(&self, text: &str) -> String {
        let mut out = String::new();
        let mut last = 0;
        for (start, end) in self.find.matches(text) {
            if start < last || end < start || end > text.len() {
                continue;
            }
            out.push_str(&text[last..start]);
            let matched = &text[start..end];
            match &self.replace {
                TextReplace::Text(replacement) => out.push_str(replacement),
                TextReplace::Function(function) => out.push_str(&function.call(matched)),
            }
            last = end;
            if end == start {
                let Some(ch) = text[last..].chars().next() else {
                    break;
                };
                out.push(ch);
                last += ch.len_utf8();
            }
        }
        out.push_str(&text[last..]);
        out
    }
}

impl PreparedFind {
    fn matches(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            PreparedFind::Literal(needle) => {
                if needle.is_empty() {
                    return Vec::new();
                }
                let mut found = Vec::new();
                let mut start = 0;
                while let Some(pos) = text[start..].find(needle.as_str()) {
                    let abs = start + pos;
                    let end = abs + needle.len();
                    found.push((abs, end));
                    start = end;
                }
                found
            }
            PreparedFind::Pattern(pattern) => pattern
                .find_iter(text)
                .map(|found| (found.start(), found.end()))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct RichCell {
    markdown: String,
    column_span: u32,
    row_span: u32,
}

struct RichTableJson {
    headers: Option<Vec<RichCell>>,
    rows: Vec<Vec<RichCell>>,
    widths: Option<Vec<u32>>,
}

fn parse_rich_table(value: &str) -> Result<RichTableJson, MarkdownConversionError> {
    let parsed = serde_json::from_str::<serde_json::Value>(value)
        .map_err(|_| MarkdownConversionError::new("Invalid table fence JSON"))?;
    let Some(object) = parsed.as_object() else {
        return Err(rows_error());
    };
    let Some(rows_value) = object.get("rows") else {
        return Err(rows_error());
    };
    let Some(rows_array) = rows_value.as_array() else {
        return Err(rows_error());
    };
    if rows_array.is_empty() || rows_array.len() > 10_000 {
        return Err(rows_error());
    }
    let headers = match object.get("headers") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let Some(array) = value.as_array() else {
                return Err(rows_error());
            };
            Some(parse_rich_row(array)?)
        }
    };
    let mut rows = Vec::with_capacity(rows_array.len());
    for row in rows_array {
        let Some(array) = row.as_array() else {
            return Err(MarkdownConversionError::new("Invalid table row"));
        };
        rows.push(parse_rich_row(array)?);
    }
    let widths = match object.get("columnWidths") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(parse_column_widths(value)?),
    };
    Ok(RichTableJson {
        headers,
        rows,
        widths,
    })
}

fn rows_error() -> MarkdownConversionError {
    MarkdownConversionError::new("Table fence requires rows and optional headers arrays")
}

fn parse_rich_row(array: &[serde_json::Value]) -> Result<Vec<RichCell>, MarkdownConversionError> {
    if array.len() > 1000 {
        return Err(MarkdownConversionError::new("Invalid table row"));
    }
    array.iter().map(parse_rich_cell).collect()
}

fn parse_rich_cell(value: &serde_json::Value) -> Result<RichCell, MarkdownConversionError> {
    if let Some(markdown) = value.as_str() {
        return Ok(RichCell {
            markdown: markdown.to_string(),
            column_span: 1,
            row_span: 1,
        });
    }
    let Some(object) = value.as_object() else {
        return Err(MarkdownConversionError::new(
            "Table cells require markdown text",
        ));
    };
    let Some(markdown) = object.get("markdown").and_then(serde_json::Value::as_str) else {
        return Err(MarkdownConversionError::new(
            "Table cells require markdown text",
        ));
    };
    Ok(RichCell {
        markdown: markdown.to_string(),
        column_span: parse_span(object.get("columnSpan"))?,
        row_span: parse_span(object.get("rowSpan"))?,
    })
}

fn parse_span(value: Option<&serde_json::Value>) -> Result<u32, MarkdownConversionError> {
    let Some(value) = value else {
        return Ok(1);
    };
    let Some(span) = value.as_u64() else {
        return Err(MarkdownConversionError::new("Invalid table cell span"));
    };
    if !(1..=1000).contains(&span) {
        return Err(MarkdownConversionError::new("Invalid table cell span"));
    }
    Ok(u32::try_from(span).unwrap_or(1000))
}

fn parse_column_widths(value: &serde_json::Value) -> Result<Vec<u32>, MarkdownConversionError> {
    let Some(array) = value.as_array() else {
        return Err(MarkdownConversionError::new("Invalid table column widths"));
    };
    let mut widths = Vec::with_capacity(array.len());
    for item in array {
        let Some(width) = item.as_u64() else {
            return Err(MarkdownConversionError::new("Invalid table column widths"));
        };
        if width == 0 || width > 31_680 {
            return Err(MarkdownConversionError::new("Invalid table column widths"));
        }
        widths.push(u32::try_from(width).unwrap_or(31_680));
    }
    Ok(widths)
}

fn validate_table_tiling(
    rows: &[&[RichCell]],
    widths: Option<&[u32]>,
) -> Result<(), MarkdownConversionError> {
    let column_count = rows
        .first()
        .map(|row| row.iter().map(|cell| cell.column_span).sum::<u32>())
        .unwrap_or(0);
    if let Some(widths) = widths {
        if widths.len() != usize::try_from(column_count).unwrap_or(usize::MAX) {
            return Err(MarkdownConversionError::new(
                "Table column widths must match the logical column count",
            ));
        }
    }
    let mut occupied: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for (row_index, row) in rows.iter().enumerate() {
        let row_index = u32::try_from(row_index).unwrap_or(u32::MAX);
        let mut column = 0u32;
        for cell in *row {
            while let Some(&(end, span)) = occupied.get(&column) {
                if end > row_index {
                    column = column.saturating_add(span);
                } else {
                    break;
                }
            }
            let col_span = cell.column_span;
            let row_span = cell.row_span;
            let row_count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            if column.saturating_add(col_span) > column_count
                || row_index.saturating_add(row_span) > row_count
            {
                return Err(MarkdownConversionError::new(
                    "Table cell span exceeds table bounds",
                ));
            }
            for covered in column..column.saturating_add(col_span) {
                if let Some(&(end, _)) = occupied.get(&covered) {
                    if end > row_index {
                        return Err(MarkdownConversionError::new("Overlapping table cell spans"));
                    }
                }
            }
            if row_span > 1 {
                occupied.insert(column, (row_index.saturating_add(row_span), col_span));
            }
            column = column.saturating_add(col_span);
        }
        while let Some(&(end, span)) = occupied.get(&column) {
            if end > row_index {
                column = column.saturating_add(span);
            } else {
                break;
            }
        }
        if column != column_count {
            return Err(MarkdownConversionError::new(
                "Table rows must cover the same number of columns",
            ));
        }
    }
    Ok(())
}

fn reject_header_footer_length(
    sections: &[ResolvedSectionInput],
    max: Option<u32>,
) -> Result<(), MarkdownConversionError> {
    let Some(max) = max else {
        return Ok(());
    };
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    let mut total = sections
        .iter()
        .map(|section| utf16_len(&section.markdown))
        .sum::<usize>();
    for section in sections {
        for group in [&section.headers, &section.footers] {
            for slot in [&group.default, &group.first, &group.even] {
                let Some(markdown) = slot
                    .as_ref()
                    .and_then(|slot| slot.content.markdown.as_deref())
                    .filter(|markdown| !markdown.is_empty())
                else {
                    continue;
                };
                total += utf16_len(markdown);
                if total > max {
                    return Err(MarkdownConversionError::new(
                        "Markdown input exceeds maxInputLength",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn count_header_footer(
    group: &ResolvedHeaderFooter,
    mut count: u64,
    max: u64,
) -> Result<u64, MarkdownConversionError> {
    for body in [&group.default, &group.first, &group.even] {
        if let Some(body) = body {
            count = count_elements(&body.blocks, count, max)?;
        }
    }
    Ok(count)
}

fn count_elements(
    blocks: &[Block],
    mut count: u64,
    max: u64,
) -> Result<u64, MarkdownConversionError> {
    for block in blocks {
        bump(&mut count, max)?;
        match block {
            Block::Paragraph(paragraph) => count_inlines(&paragraph.children, &mut count, max)?,
            Block::Heading(heading) => count_inlines(&heading.children, &mut count, max)?,
            Block::Caption(caption) => count_inlines(&caption.children, &mut count, max)?,
            Block::List(list) => {
                for item in &list.children {
                    bump(&mut count, max)?;
                    count = count_elements(&item.children, count, max)?;
                }
            }
            Block::Blockquote(quote) => {
                count = count_elements(&quote.children, count, max)?;
            }
            Block::FootnoteDefinition { children, .. } => {
                count = count_elements(children, count, max)?;
            }
            Block::Table(table) => {
                if !table.header.is_empty() {
                    count = count_row(&table.header, count, max)?;
                }
                for row in &table.rows {
                    count = count_row(row, count, max)?;
                }
            }
            Block::CodeBlock(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. } => {}
        }
    }
    Ok(count)
}

fn count_row(row: &[TableCell], mut count: u64, max: u64) -> Result<u64, MarkdownConversionError> {
    bump(&mut count, max)?;
    for cell in row {
        bump(&mut count, max)?;
        if cell.blocks.is_empty() {
            count_inlines(&cell.children, &mut count, max)?;
        } else {
            count = count_elements(&cell.blocks, count, max)?;
        }
    }
    Ok(count)
}

fn count_inlines(
    inlines: &[Inline],
    count: &mut u64,
    max: u64,
) -> Result<(), MarkdownConversionError> {
    for _inline in inlines {
        bump(count, max)?;
    }
    Ok(())
}

fn bump(count: &mut u64, max: u64) -> Result<(), MarkdownConversionError> {
    if *count % 1000 == 0 {
        // `ConvertOptions` has no abort signal. The check stays on this beat.
    }
    *count += 1;
    if *count > max {
        return Err(MarkdownConversionError::new(
            "Markdown element count exceeds maxElements",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::options::{
        Alignment, CaptionFailureMode, CaptionOptions, CaptionPlacement, DocumentSection,
        HeaderFooterContent, HeaderFooterGroup, HeaderFooterSlot, Orientation, PageInput,
        SectionConfig, SizeInput, StyleOverrides, TocMode, TocOverrides, UpdateFieldsOnOpen,
    };

    fn document(markdown: &str) -> Document {
        parse_markdown(markdown).expect("parse")
    }

    fn paragraph_runs(block: &Block) -> &[Inline] {
        match block {
            Block::Paragraph(paragraph) => &paragraph.children,
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    fn run_text(runs: &[Inline]) -> String {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn empty_input_has_no_blocks() {
        let doc = document("");
        assert!(doc.children.is_empty());
    }

    #[test]
    fn headings_keep_levels_and_inline_marks() {
        let doc = document("# H1\n## H2 *em*\n###### H6\n");
        match &doc.children[0] {
            Block::Heading(heading) => {
                assert_eq!(heading.level, 1);
                assert_eq!(run_text(&heading.children), "H1");
            }
            other => panic!("expected heading, got {other:?}"),
        }
        match &doc.children[1] {
            Block::Heading(heading) => {
                assert_eq!(heading.level, 2);
                assert_eq!(heading.children[1].text, "em");
                assert!(heading.children[1].italic);
            }
            other => panic!("expected heading, got {other:?}"),
        }
        match &doc.children[2] {
            Block::Heading(heading) => assert_eq!(heading.level, 6),
            other => panic!("expected heading, got {other:?}"),
        }
    }

    #[test]
    fn inline_marks_links_and_breaks() {
        let doc = document(
            "This has **bold**, *italic*, ++underline++, ~~strike~~, ***both***, and `code`.\n\nfoo\nbar  \nbaz\n",
        );
        let runs = paragraph_runs(&doc.children[0]);
        assert!(runs.iter().any(|run| run.text == "bold" && run.bold));
        assert!(runs.iter().any(|run| run.text == "italic" && run.italic));
        assert!(runs
            .iter()
            .any(|run| run.text == "underline" && run.underline && !run.bold));
        assert!(runs.iter().any(|run| run.text == "strike" && run.strike));
        assert!(runs
            .iter()
            .any(|run| run.text == "both" && run.bold && run.italic));
        assert!(runs
            .iter()
            .any(|run| run.text == "code" && run.code && !run.underline));

        let broken = paragraph_runs(&doc.children[1]);
        assert_eq!(run_text(broken), "foo bar\nbaz");
        assert!(broken.iter().any(|run| run.text == "\n"));
    }

    #[test]
    fn underline_respects_code_and_wrapped_lines() {
        let doc = document("**++both++** and `++no++`\n\n++hello\nworld++\n");
        let first = paragraph_runs(&doc.children[0]);
        assert!(first
            .iter()
            .any(|run| run.text == "both" && run.bold && run.underline));
        assert!(first
            .iter()
            .any(|run| run.text == "++no++" && run.code && !run.underline));
        let wrapped = paragraph_runs(&doc.children[1]);
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].text, "hello world");
        assert!(wrapped[0].underline);
    }

    #[test]
    fn links_keep_the_destination_and_surrounding_marks() {
        let doc = document("**[bold](https://example.com/a)** and <https://ex.com>\n");
        let runs = paragraph_runs(&doc.children[0]);
        let bold_link = runs.iter().find(|run| run.text == "bold").unwrap();
        assert!(bold_link.bold);
        assert_eq!(bold_link.link.as_deref(), Some("https://example.com/a"));
        let autolink = runs
            .iter()
            .find(|run| run.text == "https://ex.com")
            .unwrap();
        assert_eq!(autolink.link.as_deref(), Some("https://ex.com"));
    }

    #[test]
    fn lists_tasks_and_ordered_start() {
        let doc = document(
            "- [ ] Pending **bold** work\n- [x] Done *today*\n- [X] Uppercase checked\n  - [ ] Nested `code` task\n\n3. third\n4. fourth\n\n0. zero\n",
        );
        let Block::List(list) = &doc.children[0] else {
            panic!("expected list");
        };
        assert!(!list.ordered);
        assert_eq!(list.start, None);
        assert_eq!(list.children.len(), 3);
        assert_eq!(list.children[0].checked, Some(false));
        let pending = paragraph_runs(&list.children[0].children[0]);
        assert_eq!(pending[0].text, "\u{2610} ");
        assert!(pending.iter().any(|run| run.text == "bold" && run.bold));
        assert_eq!(list.children[1].checked, Some(true));
        assert!(paragraph_runs(&list.children[1].children[0])[0]
            .text
            .starts_with('\u{2612}'));
        assert_eq!(list.children[2].checked, Some(true));
        let Block::List(nested) = &list.children[2].children[1] else {
            panic!("expected nested list");
        };
        assert_eq!(nested.children[0].checked, Some(false));
        let nested_runs = paragraph_runs(&nested.children[0].children[0]);
        assert_eq!(nested_runs[0].text, "\u{2610} ");
        assert!(nested_runs.iter().any(|run| run.code && run.text == "code"));

        let Block::List(ordered) = &doc.children[1] else {
            panic!("expected ordered list");
        };
        assert!(ordered.ordered);
        assert_eq!(ordered.start, Some(3));
        assert_eq!(ordered.sequence_id, Some(1));
        assert_eq!(
            paragraph_runs(&ordered.children[0].children[0])[0].text,
            "third"
        );

        let zero = document("0. zero\n");
        let Block::List(zero) = &zero.children[0] else {
            panic!("expected zero list");
        };
        assert_eq!(zero.start, Some(0));
        assert!(zero.ordered);
        assert_eq!(zero.sequence_id, Some(1));
    }

    #[test]
    fn every_callout_kind_drops_its_marker() {
        let cases = [
            ("NOTE", CalloutKind::Note),
            ("TIP", CalloutKind::Tip),
            ("IMPORTANT", CalloutKind::Important),
            ("WARNING", CalloutKind::Warning),
            ("CAUTION", CalloutKind::Caution),
        ];
        for (marker, kind) in cases {
            let doc = document(&format!("> [!{marker}]\n> Body.\n"));
            let Block::Blockquote(quote) = &doc.children[0] else {
                panic!("expected quote for {marker}");
            };
            assert_eq!(quote.callout, Some(kind));
            assert_eq!(run_text(paragraph_runs(&quote.children[0])), "Body.");
        }
    }

    #[test]
    fn quotes_callouts_rules_and_nested_blocks() {
        let doc = document(
            "> [!NOTE]\n> Note body.\n\n> [!INFO]\n> Unsupported info callout.\n\n> outer\n>\n> > inner\n\n> - item\n\n> Quoted before\n>\n> ***\n>\n> Quoted after\n\n- List item\n\n  ___\n",
        );
        match &doc.children[0] {
            Block::Blockquote(quote) => {
                assert_eq!(quote.callout, Some(CalloutKind::Note));
                assert_eq!(run_text(paragraph_runs(&quote.children[0])), "Note body.");
                assert!(!run_text(paragraph_runs(&quote.children[0])).contains("[!NOTE]"));
            }
            other => panic!("expected callout, got {other:?}"),
        }
        match &doc.children[1] {
            Block::Blockquote(quote) => {
                assert_eq!(quote.callout, None);
                let text = run_text(paragraph_runs(&quote.children[0]));
                assert!(text.contains("[!INFO]"));
                assert!(text.contains("Unsupported info callout."));
            }
            other => panic!("expected quote, got {other:?}"),
        }
        match &doc.children[2] {
            Block::Blockquote(quote) => {
                assert!(matches!(quote.children[1], Block::Blockquote(_)));
            }
            other => panic!("expected nested quote, got {other:?}"),
        }
        match &doc.children[3] {
            Block::Blockquote(quote) => {
                assert!(matches!(quote.children[0], Block::List(_)));
            }
            other => panic!("expected list in quote, got {other:?}"),
        }
        match &doc.children[4] {
            Block::Blockquote(quote) => {
                assert!(matches!(quote.children[1], Block::ThematicBreak));
            }
            other => panic!("expected rule in quote, got {other:?}"),
        }
        match &doc.children[5] {
            Block::List(list) => {
                assert!(matches!(list.children[0].children[1], Block::ThematicBreak));
            }
            other => panic!("expected rule in item, got {other:?}"),
        }
    }

    #[test]
    fn fenced_code_keeps_the_language_token_and_body() {
        let doc = document("```typescript extra\nline1\nline2\n```\n\n    indented\n");
        match &doc.children[0] {
            Block::CodeBlock(code) => {
                assert_eq!(code.language.as_deref(), Some("typescript"));
                assert_eq!(code.value, "line1\nline2");
            }
            other => panic!("expected code, got {other:?}"),
        }
        match &doc.children[1] {
            Block::CodeBlock(code) => {
                assert_eq!(code.language, None);
                assert_eq!(code.value, "indented");
            }
            other => panic!("expected indented code, got {other:?}"),
        }
    }

    #[test]
    fn tables_keep_header_alignment_and_cell_marks() {
        let doc = document("| a | b |\n| :--- | ---: |\n| *x* | y |\n");
        let Block::Table(table) = &doc.children[0] else {
            panic!("expected table");
        };
        assert_eq!(table.align, vec![TableAlign::Left, TableAlign::Right]);
        assert_eq!(table.header.len(), 2);
        assert_eq!(table.header[0].children[0].text, "a");
        assert_eq!(table.rows.len(), 1);
        assert!(table.rows[0][0].children[0].italic);
        assert_eq!(table.rows[0][1].children[0].text, "y");
    }

    #[test]
    fn math_and_footnote_syntax_stay_text() {
        let doc = document("cost $5 and see [^1]\n");
        let text = run_text(paragraph_runs(&doc.children[0]));
        assert!(text.contains("$5"));
        assert!(text.contains("[^1]"));
    }

    #[test]
    fn toc_at_the_root_becomes_a_block() {
        let prepared =
            prepare("[TOC]\n\n# Section 1\n", &ConvertOptions::default()).expect("prepare");
        let SectionBlock::TableOfContents(toc) = &prepared.sections[0].children[0] else {
            panic!("expected a TOC block");
        };
        assert!(toc.primary);
        assert_eq!(toc.headings.len(), 1);
        assert_eq!(toc.headings[0].text, "Section 1");
        assert_eq!(toc.headings[0].level, 1);
        assert!(!toc.headings[0].bookmark.is_empty());
        assert!(prepared.update_fields);
    }

    #[test]
    fn quoted_toc_stays_a_paragraph() {
        let prepared = prepare("> [TOC]\n", &ConvertOptions::default()).expect("prepare");
        assert!(matches!(
            &prepared.sections[0].children[0],
            SectionBlock::Body(Block::Blockquote(_))
        ));
    }

    #[test]
    fn links_mode_does_not_request_field_update() {
        let prepared = prepare(
            "[TOC]\n\n# Title\n",
            &ConvertOptions {
                toc: Some(TocOverrides {
                    mode: Some(TocMode::Links),
                    ..TocOverrides::default()
                }),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        let SectionBlock::TableOfContents(toc) = &prepared.sections[0].children[0] else {
            panic!("expected a TOC block");
        };
        assert!(toc.primary);
        assert_eq!(toc.settings.mode, TocMode::Links);
        assert!(!prepared.update_fields);
    }

    #[test]
    fn empty_section_is_one_empty_paragraph() {
        let prepared = prepare("", &ConvertOptions::default()).expect("prepare");
        match &prepared.sections[0].children[0] {
            SectionBlock::Body(Block::Paragraph(paragraph)) => {
                assert!(paragraph.children.is_empty())
            }
            other => panic!("expected an empty paragraph, got {other:?}"),
        }
    }

    #[test]
    fn style_precedence_is_global_then_template_then_section() {
        let prepared = prepare(
            "",
            &ConvertOptions {
                style: Some(StyleOverrides {
                    title_size: Some(40),
                    font_familly: Some("Arial".to_string()),
                    ..StyleOverrides::default()
                }),
                template: Some(SectionConfig {
                    style: Some(StyleOverrides {
                        title_size: Some(36),
                        paragraph_spacing: Some(100),
                        ..StyleOverrides::default()
                    }),
                    ..SectionConfig::default()
                }),
                sections: Some(vec![DocumentSection {
                    markdown: "Hi".to_string(),
                    config: SectionConfig {
                        style: Some(StyleOverrides {
                            title_size: Some(28),
                            font_family: Some("Calibri".to_string()),
                            ..StyleOverrides::default()
                        }),
                        ..SectionConfig::default()
                    },
                }]),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        let style = &prepared.sections[0].style;
        assert_eq!(style.title_size, 28);
        assert_eq!(style.paragraph_spacing, 100);
        assert_eq!(style.line_spacing, 1.15);
        assert_eq!(style.font_family.as_deref(), Some("Calibri"));
        assert_eq!(style.paragraph_alignment, Alignment::Left);
        assert_eq!(style.heading_run_size(1), 28);
        assert_eq!(style.heading_run_size(2), 24);
        assert_eq!(style.heading_run_size(6), 8);
    }

    #[test]
    fn null_header_removes_the_template_slot() {
        let prepared = prepare(
            "Body",
            &ConvertOptions {
                template: Some(SectionConfig {
                    headers: Some(HeaderFooterGroup {
                        default: Some(HeaderFooterSlot::Content(HeaderFooterContent {
                            text: Some("Template".to_string()),
                            ..HeaderFooterContent::default()
                        })),
                        first: Some(HeaderFooterSlot::Content(HeaderFooterContent {
                            text: Some("First".to_string()),
                            ..HeaderFooterContent::default()
                        })),
                        ..HeaderFooterGroup::default()
                    }),
                    ..SectionConfig::default()
                }),
                sections: Some(vec![DocumentSection {
                    markdown: "Body".to_string(),
                    config: SectionConfig {
                        headers: Some(HeaderFooterGroup {
                            default: Some(HeaderFooterSlot::Remove),
                            ..HeaderFooterGroup::default()
                        }),
                        ..SectionConfig::default()
                    },
                }]),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        assert!(prepared.sections[0].headers.default.is_none());
        assert_eq!(
            prepared.sections[0]
                .headers
                .first
                .as_ref()
                .and_then(|slot| slot.text.as_deref()),
            Some("First")
        );
    }

    #[test]
    fn footnote_in_a_header_or_footer_fails() {
        let header = options_with_note(true);
        let error = prepare("Body", &header).expect_err("header footnote");
        assert_eq!(
            error.message(),
            "Word headers and footers cannot contain footnotes"
        );
        let footer = options_with_note(false);
        let error = prepare("Body", &footer).expect_err("footer footnote");
        assert_eq!(
            error.message(),
            "Word headers and footers cannot contain footnotes"
        );
    }

    #[test]
    fn unresolved_header_reference_is_not_a_footnote() {
        let prepared = prepare(
            "Body",
            &ConvertOptions {
                template: Some(SectionConfig {
                    headers: Some(HeaderFooterGroup {
                        default: Some(HeaderFooterSlot::Content(HeaderFooterContent {
                            markdown: Some("See [^missing]".to_string()),
                            ..HeaderFooterContent::default()
                        })),
                        ..HeaderFooterGroup::default()
                    }),
                    ..SectionConfig::default()
                }),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        let blocks = &prepared.sections[0]
            .headers
            .default
            .as_ref()
            .unwrap()
            .blocks;
        assert!(!blocks.is_empty());
    }

    #[test]
    fn footnote_numbers_continue_across_sections() {
        let prepared = prepare(
            "",
            &ConvertOptions {
                sections: Some(vec![
                    DocumentSection {
                        markdown: "One[^a]\n\n[^a]: Alpha.".to_string(),
                        config: SectionConfig::default(),
                    },
                    DocumentSection {
                        markdown: "Two[^b]\n\n[^b]: Beta.".to_string(),
                        config: SectionConfig::default(),
                    },
                ]),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        assert_eq!(footnote_ids(&prepared.sections[0]), vec!["1".to_string()]);
        assert_eq!(footnote_ids(&prepared.sections[1]), vec!["2".to_string()]);
    }

    #[test]
    fn landscape_content_width_uses_the_height_edge() {
        let prepared = prepare(
            "Hi",
            &ConvertOptions {
                template: Some(SectionConfig {
                    page: Some(PageInput {
                        size: Some(SizeInput {
                            orientation: Some(Orientation::Landscape),
                            ..SizeInput::default()
                        }),
                        ..PageInput::default()
                    }),
                    ..SectionConfig::default()
                }),
                ..ConvertOptions::default()
            },
        )
        .expect("prepare");
        assert_eq!(prepared.sections[0].page.content_width, 14678);
        assert_eq!(
            prepare("Hi", &ConvertOptions::default())
                .expect("prepare")
                .sections[0]
                .page
                .content_width,
            9746
        );
    }

    #[test]
    fn caption_failure_messages_and_option_checks() {
        let image = "![x](https://example.com/a.png)";
        let throw = ConvertOptions {
            captions: Some(CaptionOptions {
                failure_mode: Some(CaptionFailureMode::Throw),
                ..CaptionOptions::default()
            }),
            ..ConvertOptions::default()
        };
        let duplicate = format!("{image}\n\n: One {{#fig:same}}\n\n{image}\n\n: Two {{#fig:same}}");
        let err = crate::convert::convert_with_options(&duplicate, &throw).unwrap_err();
        assert_eq!(err.message(), "Duplicate caption identifier: fig:same");

        let bad_prefix = format!("{image}\n\n: Bad ID {{#tbl:not-a-figure}}");
        let err = crate::convert::convert_with_options(&bad_prefix, &throw).unwrap_err();
        assert_eq!(err.message(), "Figure captions require a fig: identifier");

        let err =
            crate::convert::convert_with_options("Missing [@fig:unknown].", &throw).unwrap_err();
        assert_eq!(err.message(), "Unresolved cross-reference: fig:unknown");

        let invalid = ConvertOptions {
            captions: Some(CaptionOptions {
                update_fields_on_open: Some(UpdateFieldsOnOpen::Invalid),
                ..CaptionOptions::default()
            }),
            ..ConvertOptions::default()
        };
        let err = crate::convert::convert_with_options("Hi", &invalid).unwrap_err();
        assert_eq!(
            err.message(),
            "Invalid captions.updateFieldsOnOpen: Must be a boolean"
        );

        let blank_label = ConvertOptions {
            captions: Some(CaptionOptions {
                figure_label: Some(" ".to_string()),
                ..CaptionOptions::default()
            }),
            ..ConvertOptions::default()
        };
        let err = crate::convert::convert_with_options("Hi", &blank_label).unwrap_err();
        assert_eq!(
            err.message(),
            "Invalid captions.figureLabel: Must be a non-empty string"
        );

        let size = ConvertOptions {
            captions: Some(CaptionOptions {
                size: Some(7),
                ..CaptionOptions::default()
            }),
            ..ConvertOptions::default()
        };
        let err = crate::convert::convert_with_options("Hi", &size).unwrap_err();
        assert_eq!(
            err.message(),
            "Invalid captions.size: Must be between 8 and 144 half-points"
        );

        let styled = ConvertOptions {
            captions: Some(CaptionOptions {
                figure_label: Some("Illustration".to_string()),
                figure_placement: Some(CaptionPlacement::Above),
                alignment: Some(Alignment::Left),
                italic: Some(true),
                size: Some(20),
                update_fields_on_open: Some(UpdateFieldsOnOpen::Bool(false)),
                ..CaptionOptions::default()
            }),
            ..ConvertOptions::default()
        };
        let markdown = format!("{image}\n\n: Styled caption {{#fig:styled}}");
        let prepared = prepare(&markdown, &styled).expect("prepare");
        assert!(!prepared.update_fields);
        let body: Vec<&Block> = prepared.sections[0]
            .children
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Body(block) => Some(block),
                SectionBlock::TableOfContents(_) => None,
            })
            .collect();
        assert!(matches!(body[0], Block::Caption(_)));
        let bytes = crate::convert::convert_with_options(&markdown, &styled).expect("docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut settings = archive.by_name("word/settings.xml").expect("settings");
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut settings, &mut xml).expect("read settings");
        assert!(
            !xml.contains("updateFields"),
            "settings should omit w:updateFields\n{xml}"
        );
        let code = document(&format!(
            "{image}\n\n: Literal {{#fig:literal}}\n\nSee [@fig:literal] and `[@fig:literal]`."
        ));
        let runs = paragraph_runs(code.children.last().expect("paragraph"));
        let coded = runs.iter().find(|run| run.code).expect("code run");
        assert_eq!(coded.text, "[@fig:literal]");
        assert!(coded.cross_ref.is_none());
        assert!(runs.iter().any(|run| run.cross_ref.is_some()));
    }

    fn options_with_note(header: bool) -> ConvertOptions {
        let group = HeaderFooterGroup {
            default: Some(HeaderFooterSlot::Content(HeaderFooterContent {
                markdown: Some("Note[^n]\n\n[^n]: Hidden.".to_string()),
                ..HeaderFooterContent::default()
            })),
            ..HeaderFooterGroup::default()
        };
        ConvertOptions {
            template: Some(SectionConfig {
                headers: header.then_some(group.clone()),
                footers: (!header).then_some(group),
                ..SectionConfig::default()
            }),
            ..ConvertOptions::default()
        }
    }

    fn footnote_ids(section: &PreparedSection) -> Vec<String> {
        let mut ids = Vec::new();
        for block in &section.children {
            if let SectionBlock::Body(block) = block {
                collect_footnote_ids(block, &mut ids);
            }
        }
        ids
    }

    fn collect_footnote_ids(block: &Block, ids: &mut Vec<String>) {
        match block {
            Block::Paragraph(paragraph) => push_footnote_ids(&paragraph.children, ids),
            Block::Heading(heading) => push_footnote_ids(&heading.children, ids),
            Block::List(list) => {
                for item in &list.children {
                    for child in &item.children {
                        collect_footnote_ids(child, ids);
                    }
                }
            }
            Block::Blockquote(quote) => {
                for child in &quote.children {
                    collect_footnote_ids(child, ids);
                }
            }
            Block::Table(table) => {
                for cell in table.header.iter().chain(table.rows.iter().flatten()) {
                    push_footnote_ids(&cell.children, ids);
                }
            }
            Block::Caption(caption) => push_footnote_ids(&caption.children, ids),
            Block::FootnoteDefinition { .. }
            | Block::CodeBlock(_)
            | Block::ThematicBreak
            | Block::PageBreak
            | Block::Comment(_)
            | Block::Image(_)
            | Block::DisplayMath { .. }
            | Block::ChartImage { .. } => {}
        }
    }

    fn push_footnote_ids(runs: &[Inline], ids: &mut Vec<String>) {
        for run in runs {
            if let Some(mark) = &run.footnote {
                if mark.first {
                    ids.push(mark.id.clone());
                }
            }
        }
    }
}
