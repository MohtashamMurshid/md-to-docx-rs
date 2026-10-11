//! Owned document tree for the Markdown body.
//!
//! The tree follows the TypeScript document model: blocks contain either more
//! blocks or inline runs. Table cells hold inline runs. A hard break is an
//! inline run whose text is `"\n"`. A task item's checkbox glyph is already
//! the first run of its first paragraph. Footnote definitions that a body
//! reference uses are blocks after the body, in first-reference order.

use crate::highlight::HighlightSpan;
use crate::options::{
    Alignment, PageGeometry, PageNumberDisplay, ResolvedPageNumbering, ResolvedStyle, SectionBreak,
    TocSettings,
};

/// One Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub children: Vec<Block>,
}

/// A block in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Paragraph),
    Heading(Heading),
    List(List),
    CodeBlock(CodeBlock),
    Blockquote(Blockquote),
    Table(Table),
    ThematicBreak,
    /// A Word page break from `\pagebreak` or an HTML block that contains `pagebreak`.
    PageBreak,
    /// A Word comment from an HTML comment `<!-- COMMENT: text -->`.
    Comment(String),
    /// A footnote body. `id` matches [`FootnoteMark::id`].
    FootnoteDefinition {
        id: String,
        children: Vec<Block>,
    },
    /// A figure or table caption. The number is a `SEQ` field inside `bookmark`.
    Caption(Caption),
    /// A Markdown image. `bytes` is `None` when the load failed.
    Image(Image),
    /// Display math (`$$...$$`) as OMML. The string is the `<m:oMath>` element.
    DisplayMath {
        omml: String,
    },
    /// A rendered `chart` or `chartjs` fence. The bytes are a PNG.
    /// `alt` is the drawing description. An omitted JSON `alt` uses the chart type.
    ChartImage {
        bytes: Vec<u8>,
        width_px: u32,
        height_px: u32,
        alt: String,
    },
}

/// A Markdown image after `load_markdown_image`.
///
/// `url` keeps the fragment so the writer can read `#WxH` hints. `alt` is the
/// drawing description and `title` is the drawing name. Empty `bytes` is the
/// placeholder paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub bytes: Option<Vec<u8>>,
    pub content_type: String,
    pub width_px: u32,
    pub height_px: u32,
    pub alt: String,
    pub title: Option<String>,
    pub url: String,
}

/// A paragraph of inline runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub children: Vec<Inline>,
}

/// A heading at level 1 through 6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    /// Word bookmark id from `crate::bookmarks::heading_bookmark`.
    pub bookmark: String,
    pub children: Vec<Inline>,
}

/// A bullet list or an ordered list.
///
/// `start` is set only for an ordered list, and it keeps the marker's number,
/// including 0. `sequence_id` counts ordered lists in document order, starting
/// at 1, so each one can have its own numbering definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub ordered: bool,
    pub start: Option<u64>,
    pub sequence_id: Option<u32>,
    pub children: Vec<ListItem>,
}

/// One item in a [`List`].
///
/// `checked` is `Some` for a task item. The checkbox text (`☐ ` or `☒ `,
/// including the trailing space) is already stored on the first paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub children: Vec<Block>,
}

/// A fenced or indented code block.
///
/// `language` is the first token of a fence info string. Indented code and a
/// fence with no info string leave it empty. `value` does not include a final
/// trailing newline. `colors` is set only when highlighting is on. Empty
/// `highlights` keeps the plain run, including an unknown language whose
/// highlighter span has no color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub value: String,
    pub colors: Option<CodeColors>,
    pub highlights: Vec<HighlightSpan>,
}

/// Theme colors for one highlighted code block. Hex digits, no `#`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeColors {
    pub label_color: String,
    pub plain_color: String,
    pub fill: String,
    pub border: String,
    pub show_label: bool,
}

impl CodeBlock {
    /// Courier block with no token colors.
    pub fn plain(language: Option<String>, value: String) -> Self {
        Self {
            language,
            value,
            colors: None,
            highlights: Vec::new(),
        }
    }
}

/// GitHub callout kinds that pulldown-cmark reports on a block quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// A block quote. `callout` is set when the quote is a GitHub callout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blockquote {
    pub callout: Option<CalloutKind>,
    pub children: Vec<Block>,
}

/// Column alignment from a GFM table delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlign {
    None,
    Left,
    Center,
    Right,
}

/// A GFM table or a rich `table` fence.
///
/// `header` is the first row when the table has one. `rows` are the body.
/// `column_widths` is set for a rich table that names widths, in twips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub align: Vec<TableAlign>,
    pub header: Vec<TableCell>,
    pub rows: Vec<Vec<TableCell>>,
    pub column_widths: Option<Vec<u32>>,
}

impl Table {
    /// A GFM table. Cells use span 1 and there is no width list.
    pub fn gfm(align: Vec<TableAlign>, header: Vec<TableCell>, rows: Vec<Vec<TableCell>>) -> Self {
        Self {
            align,
            header,
            rows,
            column_widths: None,
        }
    }
}

/// One table cell.
///
/// GFM cells fill `children` and leave `blocks` empty. A rich cell fills
/// `blocks` with the parsed cell Markdown. Spans default to 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCell {
    pub children: Vec<Inline>,
    pub blocks: Vec<Block>,
    pub column_span: u32,
    pub row_span: u32,
}

impl TableCell {
    /// A GFM cell of inline runs, spanning one column and one row.
    pub fn text(children: Vec<Inline>) -> Self {
        Self {
            children,
            blocks: Vec::new(),
            column_span: 1,
            row_span: 1,
        }
    }
}

/// A text run and the marks that apply to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    /// Set when this run is a footnote reference. `text` is empty.
    pub footnote: Option<FootnoteMark>,
    /// Set when this run points at a caption. Inline code stays ordinary text.
    pub cross_ref: Option<CrossRef>,
    /// OMML for inline `$...$` math. Empty `text` means the writer emits this
    /// instead of a text run. `None` keeps `text`, including an unsupported
    /// expression left as `$...$`.
    pub math_omml: Option<String>,
}

/// A reference to a defined footnote.
///
/// `first` is true for the first reference to `id` and false for each later
/// one. Those later references are the ones Word renders as `NOTEREF`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteMark {
    pub id: String,
    pub first: bool,
}

/// Figure and table counters are independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionKind {
    Figure,
    Table,
}

/// A caption paragraph the writer turns into a bookmark and a `SEQ` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caption {
    pub kind: CaptionKind,
    pub id: String,
    pub bookmark: String,
    pub children: Vec<Inline>,
}

/// A `REF` field pointing at [`Caption::bookmark`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRef {
    pub kind: CaptionKind,
    pub id: String,
    pub bookmark: String,
}

/// A document after section, header, footer, and TOC resolution.
///
/// `[TOC]` at the root of a section body is [`SectionBlock::TableOfContents`].
/// The first such block that has headings in range is `primary`. Later
/// placeholders stay in the tree so the writer can drop them.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedDocument {
    pub sections: Vec<PreparedSection>,
    pub update_fields: bool,
    pub toc: TocSettings,
}

/// One resolved section, including parsed header and footer slots.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSection {
    /// Section Markdown. The image repair reads footnote images from it.
    pub markdown: String,
    pub style: ResolvedStyle,
    pub page: PageGeometry,
    pub page_numbering: Option<ResolvedPageNumbering>,
    pub title_page: Option<bool>,
    pub break_type: Option<SectionBreak>,
    pub headers: ResolvedHeaderFooter,
    pub footers: ResolvedHeaderFooter,
    pub children: Vec<SectionBlock>,
}

/// Parsed `default`, `first`, and `even` slots. `None` means the slot is absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedHeaderFooter {
    pub default: Option<HeaderFooterBody>,
    pub first: Option<HeaderFooterBody>,
    pub even: Option<HeaderFooterBody>,
}

/// Header or footer content after its Markdown has been parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderFooterBody {
    pub blocks: Vec<Block>,
    pub text: Option<String>,
    pub alignment: Option<Alignment>,
    pub page_number_display: Option<PageNumberDisplay>,
}

/// A body block, or a TOC placeholder the writer turns into a TOC field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionBlock {
    Body(Block),
    TableOfContents(TocBlock),
}

/// A `[TOC]` placeholder and the settings that fill it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocBlock {
    /// The first placeholder that has headings to show. Only that one is inserted.
    pub primary: bool,
    pub headings: Vec<TocHeading>,
    pub settings: TocSettings,
}

/// One cached TOC entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    pub text: String,
    pub level: u8,
    pub bookmark: String,
}
