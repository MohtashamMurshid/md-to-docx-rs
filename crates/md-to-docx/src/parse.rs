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
