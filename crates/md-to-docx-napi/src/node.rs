//! N-API exports. Each function returns a Node.js `Buffer` of `.docx` bytes.
//!
//! The generated module register calls these functions. A `cargo test` build
//! does not emit that register, so rustc reports them as unused there.

#![cfg_attr(test, allow(dead_code))]

use napi::bindgen_prelude::{Buffer, Either, Uint8Array};
use napi::{Error, Result, Status};
use napi_derive::napi;

use crate::{
    convert_markdown_to_array_buffer as convert_to_array_buffer,
    convert_markdown_to_buffer as convert_to_buffer, convert_markdown_to_docx as convert_to_docx,
    convert_markdown_with_reference_docx as convert_with_reference_bytes,
    convert_markdown_with_reference_docx_to_array_buffer as convert_with_reference_to_array_buffer,
    convert_markdown_with_reference_docx_to_buffer as convert_with_reference_to_buffer,
    patch_markdown_in_docx as patch_in_docx,
    patch_markdown_in_docx_to_array_buffer as patch_to_array_buffer,
    patch_markdown_in_docx_to_buffer as patch_to_buffer,
};

fn to_buffer(
    result: std::result::Result<Vec<u8>, md_to_docx::MarkdownConversionError>,
) -> Result<Buffer> {
    match result {
        Ok(bytes) => Ok(bytes.into()),
        Err(error) => Err(Error::new(
            Status::GenericFailure,
            error.message().to_string(),
        )),
    }
}

fn input_bytes(value: Either<Buffer, Uint8Array>) -> Vec<u8> {
    match value {
        Either::A(buffer) => Vec::<u8>::from(buffer),
        Either::B(array) => array.as_ref().to_vec(),
    }
}

#[napi]
pub fn convert_markdown_to_docx(markdown: String, options_json: Option<String>) -> Result<Buffer> {
    to_buffer(convert_to_docx(&markdown, options_json.as_deref()))
}

#[napi]
pub fn convert_markdown_to_array_buffer(
    markdown: String,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(convert_to_array_buffer(&markdown, options_json.as_deref()))
}

#[napi]
pub fn convert_markdown_to_buffer(
    markdown: String,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(convert_to_buffer(&markdown, options_json.as_deref()))
}

#[napi]
pub fn convert_markdown_with_reference_docx(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(convert_with_reference_bytes(
        &markdown,
        &input_bytes(reference_docx),
        options_json.as_deref(),
    ))
}

#[napi]
pub fn convert_markdown_with_reference_docx_to_array_buffer(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(convert_with_reference_to_array_buffer(
        &markdown,
        &input_bytes(reference_docx),
        options_json.as_deref(),
    ))
}

#[napi]
pub fn convert_markdown_with_reference_docx_to_buffer(
    markdown: String,
    reference_docx: Either<Buffer, Uint8Array>,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(convert_with_reference_to_buffer(
        &markdown,
        &input_bytes(reference_docx),
        options_json.as_deref(),
    ))
}

#[napi]
pub fn patch_markdown_in_docx(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(patch_in_docx(
        &input_bytes(reference_docx),
        &patches_json,
        options_json.as_deref(),
    ))
}

#[napi]
pub fn patch_markdown_in_docx_to_array_buffer(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(patch_to_array_buffer(
        &input_bytes(reference_docx),
        &patches_json,
        options_json.as_deref(),
    ))
}

#[napi]
pub fn patch_markdown_in_docx_to_buffer(
    reference_docx: Either<Buffer, Uint8Array>,
    patches_json: String,
    options_json: Option<String>,
) -> Result<Buffer> {
    to_buffer(patch_to_buffer(
        &input_bytes(reference_docx),
        &patches_json,
        options_json.as_deref(),
    ))
}
