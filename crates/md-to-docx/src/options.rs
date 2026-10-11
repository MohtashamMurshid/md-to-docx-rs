//! Slice 3 conversion options.
//!
//! Precedence for style is the built-in default, then the global style, then
//! `template.style`, then the section style. Header and footer slots inherit
//! the same way. `Remove` is the JSON `null` that drops a slot.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::error::{ErrorContext, MarkdownConversionError};

/// Options for [`crate::convert::convert_with_options`].
///
/// Every field is optional. [`ConvertOptions::default`] is the single-string
/// conversion [`crate::convert::convert_markdown`] already performs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConvertOptions {
    pub style: Option<StyleOverrides>,
    pub template: Option<SectionConfig>,
    pub sections: Option<Vec<DocumentSection>>,
    pub toc: Option<TocOverrides>,
    pub captions: Option<CaptionOptions>,
    pub accessibility: Option<AccessibilityOptions>,
    pub math_rendering: Option<MathRenderingOptions>,
    pub chart_rendering: Option<ChartRenderingOptions>,
    pub mermaid_rendering: Option<MermaidRenderingOptions>,
    pub code_highlighting: Option<CodeHighlightingOptions>,
    pub text_replacements: Option<Vec<TextReplacement>>,
    pub text_replacement_mode: Option<TextReplacementMode>,
    pub max_input_length: Option<u32>,
    pub max_elements: Option<u32>,
    /// Package properties. `None` leaves the legacy package untouched.
    pub metadata: Option<DocumentMetadata>,
    /// Trusted Rust plugins. `None` and an empty list leave the document unchanged.
    pub plugins: Option<Vec<MarkdownDocxPlugin>>,
    /// `conflict_policy` defaults to [`PluginConflictPolicy::Error`].
    pub plugin_options: Option<PluginOptions>,
}

impl ConvertOptions {
    /// True when no section, style, TOC, or caption option is set.
    pub fn is_plain(&self) -> bool {
        self.style.is_none()
            && self.template.is_none()
            && self
                .sections
                .as_ref()
                .is_none_or(|sections| sections.is_empty())
            && self.toc.is_none()
            && self.captions.is_none()
            && self.accessibility.is_none()
            && self.math_rendering.is_none()
            && self.chart_rendering.is_none()
            && self.mermaid_rendering.is_none()
            && !self.highlighting_enabled()
            && self
                .text_replacements
                .as_ref()
                .is_none_or(|items| items.is_empty())
            && self.max_elements.is_none()
            && self
                .plugins
                .as_ref()
                .is_none_or(|plugins| plugins.is_empty())
    }

    /// `codeHighlighting.enabled === true`. `false` and an absent block stay off.
    pub fn highlighting_enabled(&self) -> bool {
        matches!(
            self.code_highlighting
                .as_ref()
                .and_then(|highlighting| highlighting.enabled),
            Some(OptionBool::Bool(true))
        )
    }

    /// Reject a blank `fontFamily` the way `validateStyleInput` does.
    ///
    /// A present family that trims to nothing is an error. `font_familly` is
    /// checked only when it supplies the family, matching `normalizeStyleInput`.
    pub fn validate(&self) -> Result<(), MarkdownConversionError> {
        validate_style(self.style.as_ref(), "options.style")?;
        if let Some(template) = &self.template {
            validate_style(template.style.as_ref(), "options.template.style")?;
        }
        if let Some(sections) = &self.sections {
            for (index, section) in sections.iter().enumerate() {
                validate_style(
                    section.config.style.as_ref(),
                    &format!("options.sections[{index}].style"),
                )?;
            }
        }
        validate_captions(self.captions.as_ref())?;
        validate_accessibility(self.accessibility.as_ref())?;
        validate_document_metadata(self.metadata.as_ref())?;
        validate_math_rendering(self.math_rendering.as_ref())?;
        validate_chart_rendering(self.chart_rendering.as_ref())?;
        validate_mermaid_rendering(self.mermaid_rendering.as_ref())?;
        validate_positive_integer(self.max_input_length, "maxInputLength")?;
        validate_positive_integer(self.max_elements, "maxElements")?;
        validate_text_replacements(self)?;
        validate_plugins(self)?;
        Ok(())
    }
}
