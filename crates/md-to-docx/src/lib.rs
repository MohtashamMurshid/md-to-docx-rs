//! Markdown to `.docx` bytes.
//!
//! Slice 1 accepts one Markdown string. Images, math, footnotes, comments,
//! TOC, page breaks, sections, headers, reference packages, patch, plugins,
//! and the CLI stay out of this crate until later slices.

mod bookmarks;
mod captions;
mod charts;
mod convert;
mod error;
pub mod highlight;
mod images;
mod math;
mod model;
mod ooxml;
pub mod options;
mod package;
mod parse;
mod patch;
mod reference;

pub use convert::{
    apply_document_metadata, convert_markdown, convert_with_options, convert_with_reference,
    patch_markdown, patch_placeholders,
};
pub use error::MarkdownConversionError;
pub use images::{
    load_markdown_image, ImageOptions, ImageResolveContext, ImageResolver, LoadedImage,
    MissingImageAltText, ResolvedImage,
};
pub use options::{
    ConvertOptions, DocumentMetadata, DuplicateStyleNameBehavior, MissingStyleBehavior,
    PatchMarkdownOptions, PlaceholderDelimiters, ReferenceDocxGenerationOptions,
    ReferenceDocxModeOptions, ReferenceDocxPackageLimits, ReferenceStyleSelector,
};
