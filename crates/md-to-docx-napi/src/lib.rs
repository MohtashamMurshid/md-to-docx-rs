//! Node binding for the Markdown to `.docx` bytes API.
//!
//! The functions here re-export the public conversion entry points and return
//! bytes. `downloadDocx` stays in `index.js` and throws a plain `Error` in Node.
//! This crate does not embed a JavaScript engine.

mod convert;
mod options_json;

#[cfg(feature = "node")]
mod node;

pub use convert::{
    convert_markdown_to_array_buffer, convert_markdown_to_buffer, convert_markdown_to_docx,
    convert_markdown_with_reference_docx, convert_markdown_with_reference_docx_to_array_buffer,
    convert_markdown_with_reference_docx_to_buffer, patch_markdown_in_docx,
    patch_markdown_in_docx_to_array_buffer, patch_markdown_in_docx_to_buffer,
};
