//! Markdown to `.docx` bytes.
//!
//! Convert Markdown with typed options, sections, images, math, plugins,
//! reference documents, and placeholder patching.

mod bookmarks;
mod captions;
mod charts;
mod convert;
mod error;
pub mod highlight;
mod images;
mod math;
pub mod model;
mod ooxml;
pub mod options;
pub mod options_json;
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
