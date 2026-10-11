//! Byte-returning conversion functions.
//!
//! The three TypeScript output names (`Docx`, `ArrayBuffer`, and `Buffer`)
//! all return the same `.docx` bytes. `downloadDocx` is not here.

use std::collections::BTreeMap;

use md_to_docx::{
    convert_with_options, convert_with_reference, patch_markdown, ConvertOptions,
    MarkdownConversionError, PatchMarkdownOptions, ReferenceDocxGenerationOptions,
};

use crate::options_json::{self, LoadedOptions};

fn load_options(options_json: Option<&str>) -> Result<Option<LoadedOptions>, MarkdownConversionError> {
    let Some(text) = options_json else {
        return Ok(None);
    };
    options_json::load_options_json(text, "options")
        .map(Some)
        .map_err(MarkdownConversionError::new)
}

fn convert_options(loaded: Option<&LoadedOptions>) -> ConvertOptions {
    loaded
        .map(|loaded| loaded.convert.clone())
        .unwrap_or_default()
}

fn patch_options(loaded: Option<&LoadedOptions>) -> PatchMarkdownOptions {
    loaded
        .map(|loaded| loaded.patch.clone())
        .unwrap_or_default()
}

fn reference_options(loaded: Option<&LoadedOptions>) -> ReferenceDocxGenerationOptions {
    match loaded {
        Some(loaded) => ReferenceDocxGenerationOptions {
            reference: loaded.reference.clone(),
            aborted: loaded.aborted,
        },
        None => ReferenceDocxGenerationOptions::default(),
    }
}

/// `convertMarkdownToDocx`. Returns `.docx` bytes.
pub fn convert_markdown_to_docx(
    markdown: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    convert_markdown_to_buffer(markdown, options_json)
}

/// `convertMarkdownToArrayBuffer`. Returns `.docx` bytes.
pub fn convert_markdown_to_array_buffer(
    markdown: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    convert_markdown_to_buffer(markdown, options_json)
}

/// `convertMarkdownToBuffer`. Returns `.docx` bytes.
pub fn convert_markdown_to_buffer(
    markdown: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let loaded = load_options(options_json)?;
    convert_with_options(markdown, &convert_options(loaded.as_ref()))
}

/// `convertMarkdownWithReferenceDocx`. Returns `.docx` bytes.
pub fn convert_markdown_with_reference_docx(
    markdown: &str,
    reference_docx: &[u8],
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let loaded = load_options(options_json)?;
    convert_with_reference(
        markdown,
        reference_docx,
        &reference_options(loaded.as_ref()),
    )
}

/// `convertMarkdownWithReferenceDocxToArrayBuffer`. Returns `.docx` bytes.
pub fn convert_markdown_with_reference_docx_to_array_buffer(
    markdown: &str,
    reference_docx: &[u8],
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    convert_markdown_with_reference_docx(markdown, reference_docx, options_json)
}

/// `convertMarkdownWithReferenceDocxToBuffer`. Returns `.docx` bytes.
pub fn convert_markdown_with_reference_docx_to_buffer(
    markdown: &str,
    reference_docx: &[u8],
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    convert_markdown_with_reference_docx(markdown, reference_docx, options_json)
}

fn patches_from_json(patches_json: &str) -> Result<BTreeMap<String, String>, MarkdownConversionError> {
    options_json::load_patches_json(patches_json).map_err(MarkdownConversionError::new)
}

/// `patchMarkdownInDocx`. `patches_json` is a JSON object of placeholder names
/// to Markdown strings, or to `{ "markdown": "..." }`. Returns `.docx` bytes.
pub fn patch_markdown_in_docx(
    reference_docx: &[u8],
    patches_json: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let loaded = load_options(options_json)?;
    let patches = patches_from_json(patches_json)?;
    patch_markdown(reference_docx, &patches, &patch_options(loaded.as_ref()))
}

/// `patchMarkdownInDocxToArrayBuffer`. Returns `.docx` bytes.
pub fn patch_markdown_in_docx_to_array_buffer(
    reference_docx: &[u8],
    patches_json: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    patch_markdown_in_docx(reference_docx, patches_json, options_json)
}

/// `patchMarkdownInDocxToBuffer`. Returns `.docx` bytes.
pub fn patch_markdown_in_docx_to_buffer(
    reference_docx: &[u8],
    patches_json: &str,
    options_json: Option<&str>,
) -> Result<Vec<u8>, MarkdownConversionError> {
    patch_markdown_in_docx(reference_docx, patches_json, options_json)
}

#[cfg(test)]
mod tests {
    use super::{
        convert_markdown_to_array_buffer, convert_markdown_to_buffer, convert_markdown_to_docx,
    };

    #[test]
    fn convert_returns_zip_bytes() {
        let bytes = convert_markdown_to_buffer("# Hello", None).unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }

    #[test]
    fn output_names_return_the_same_bytes() {
        let docx = convert_markdown_to_docx("Hello", None).unwrap();
        let array_buffer = convert_markdown_to_array_buffer("Hello", None).unwrap();
        let buffer = convert_markdown_to_buffer("Hello", None).unwrap();
        assert_eq!(docx, array_buffer);
        assert_eq!(array_buffer, buffer);
    }

    #[test]
    fn blank_font_family_is_rejected() {
        let error = convert_markdown_to_buffer(
            "Hello",
            Some(r#"{"style":{"fontFamily":"   "}}"#),
        )
        .unwrap_err();
        assert!(
            error.message().contains("Invalid fontFamily"),
            "{error}"
        );
    }

    #[test]
    fn plugins_are_not_accepted() {
        let error = convert_markdown_to_buffer("Hello", Some(r#"{"plugins":[]}"#)).unwrap_err();
        assert!(
            error.message().contains("cannot be passed from JavaScript"),
            "{error}"
        );
    }
}
