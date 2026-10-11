//! CLI adapter for the shared options JSON decoder.
pub use md_to_docx::options_json::{load_patches_json, LoadedOptions};
pub fn load_options_json(text: &str, path: &str) -> Result<LoadedOptions, String> {
    md_to_docx::options_json::load_options_json(text, path).map_err(|error| {
        error.replace(
            "cannot be loaded from options JSON",
            "cannot be loaded from CLI JSON",
        )
    })
}
