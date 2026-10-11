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
