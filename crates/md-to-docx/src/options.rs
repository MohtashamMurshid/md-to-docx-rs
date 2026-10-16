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
    /// Reject a request whose cancellation flag was already set.
    pub aborted: bool,
    /// `documentType`. Only `report` changes output: table header shading.
    pub document_type: Option<DocumentType>,
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
    /// Image configuration; budgets restart for every conversion.
    pub image_handling: Option<crate::images::ImageOptions>,
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
            && !matches!(self.document_type, Some(DocumentType::Report))
            && self
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.language.as_ref())
                .is_none()
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
            && self.image_handling.is_none()
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
        if self.aborted {
            return Err(MarkdownConversionError::with_context(
                "Markdown conversion was aborted",
                BTreeMap::from([("code".into(), "ABORTED".into())]),
            ));
        }
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
        if self.sections.as_ref().is_some_and(Vec::is_empty) {
            return Err(MarkdownConversionError::new(
                "Invalid sections input: options.sections must contain at least one section",
            ));
        }
        if let Some(toc) = &self.toc {
            for (name, value) in [("minDepth", toc.min_depth), ("maxDepth", toc.max_depth)] {
                if value.is_some_and(|value| !(1..=6).contains(&value)) {
                    return Err(MarkdownConversionError::new(format!(
                        "Invalid TOC {name}: Must be an integer between 1 and 6"
                    )));
                }
            }
            if matches!((toc.min_depth, toc.max_depth), (Some(min), Some(max)) if min > max) {
                return Err(MarkdownConversionError::new(
                    "Invalid TOC depth range: minDepth cannot be greater than maxDepth",
                ));
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

/// `accessibility.missingImageAltText`. `Invalid` is any other JSON string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingImageAltText {
    Permissive,
    Throw,
    Invalid,
}

/// Accessibility checks that run before an image is fetched.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilityOptions {
    pub missing_image_alt_text: Option<MissingImageAltText>,
}

fn validate_accessibility(
    accessibility: Option<&AccessibilityOptions>,
) -> Result<(), MarkdownConversionError> {
    if accessibility.and_then(|options| options.missing_image_alt_text)
        == Some(MissingImageAltText::Invalid)
    {
        return Err(MarkdownConversionError::new(
            "Invalid accessibility.missingImageAltText: must be permissive or throw",
        ));
    }
    Ok(())
}

/// Word proofing language from `runLanguage` in `src/accessibility.ts`.
///
/// East Asian tags set `east_asia`. Right-to-left sets `bidirectional`.
/// The language tag does not choose the paragraph direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLanguage {
    pub value: String,
    pub east_asia: Option<String>,
    pub bidirectional: Option<String>,
}

/// `runLanguage(style)`. `language` is the resolved tag, not a fence info string.
pub fn run_language(language: &str, direction: Direction) -> RunLanguage {
    let primary = language
        .split('-')
        .next()
        .unwrap_or(language)
        .to_ascii_lowercase();
    let east_asia = matches!(primary.as_str(), "zh" | "ja" | "ko").then(|| language.to_string());
    let bidirectional = (direction == Direction::Rtl).then(|| language.to_string());
    RunLanguage {
        value: language.to_string(),
        east_asia,
        bidirectional,
    }
}

/// A JSON boolean. `Invalid` is any other JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionBool {
    Bool(bool),
    Invalid,
}

/// `mathRendering`. Omitted `enabled` keeps math on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MathRenderingOptions {
    pub enabled: Option<OptionBool>,
    pub unsupported: Option<MathUnsupported>,
}

/// What to do with TeX the native renderer does not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathUnsupported {
    Text,
    Throw,
    Invalid,
}

/// `chartRendering`. Omitted or `enabled: false` leaves the fence as code.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartRenderingOptions {
    pub enabled: Option<OptionBool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub invalid_definition_behavior: Option<ChartInvalidBehavior>,
    pub renderer: Option<ChartRenderer>,
}

/// JSON `chartRendering.invalidDefinitionBehavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartInvalidBehavior {
    Placeholder,
    Throw,
    Invalid,
}

type ChartRenderFn =
    dyn Fn(&str, u32, u32) -> Result<Vec<u8>, MarkdownConversionError> + Send + Sync;
type MermaidRenderFn = dyn Fn(&MermaidRenderInput) -> Result<Option<MermaidRenderOutput>, MarkdownConversionError>
    + Send
    + Sync;
type PluginFenceRenderFn =
    dyn Fn(&PluginFenceInput) -> Result<Vec<PluginBlock>, MarkdownConversionError> + Send + Sync;
type PluginNodeRenderFn = dyn Fn(&PluginBlockNodeInput) -> Result<Vec<PluginBlock>, MarkdownConversionError>
    + Send
    + Sync;

/// Optional replacement for the built-in chart rasterizer.
///
/// The arguments are the fence body and the resolved pixel width and height.
#[derive(Clone)]
pub struct ChartRenderer {
    render: Arc<ChartRenderFn>,
}

impl ChartRenderer {
    pub fn new<F>(render: F) -> Self
    where
        F: Fn(&str, u32, u32) -> Result<Vec<u8>, MarkdownConversionError> + Send + Sync + 'static,
    {
        Self {
            render: Arc::new(render),
        }
    }

    pub fn render(
        &self,
        source: &str,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, MarkdownConversionError> {
        (self.render)(source, width, height)
    }
}

impl std::fmt::Debug for ChartRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChartRenderer")
    }
}

impl PartialEq for ChartRenderer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.render, &other.render)
    }
}

/// `mermaidRendering`. A fence stays a code block until `enabled` is true.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MermaidRenderingOptions {
    pub enabled: Option<OptionBool>,
    pub render: Option<MermaidRenderer>,
    pub failure_mode: Option<MermaidFailureMode>,
}

/// JSON `mermaidRendering.failureMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MermaidFailureMode {
    CodeBlock,
    Placeholder,
    Throw,
    Invalid,
}

/// Input for a caller-supplied Mermaid renderer. This crate does not run Mermaid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidRenderInput {
    pub code: String,
    pub meta: Option<String>,
}

/// Raster bytes from a Mermaid renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidRenderOutput {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub alt: String,
    pub title: Option<String>,
}

/// Caller-supplied Mermaid renderer. `Ok(None)` means no image was produced.
#[derive(Clone)]
pub struct MermaidRenderer {
    render: Arc<MermaidRenderFn>,
}

impl MermaidRenderer {
    pub fn new<F>(render: F) -> Self
    where
        F: Fn(&MermaidRenderInput) -> Result<Option<MermaidRenderOutput>, MarkdownConversionError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            render: Arc::new(render),
        }
    }

    pub fn render(
        &self,
        input: &MermaidRenderInput,
    ) -> Result<Option<MermaidRenderOutput>, MarkdownConversionError> {
        (self.render)(input)
    }
}

impl std::fmt::Debug for MermaidRenderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MermaidRenderer")
    }
}

impl PartialEq for MermaidRenderer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.render, &other.render)
    }
}

fn validate_math_rendering(
    math: Option<&MathRenderingOptions>,
) -> Result<(), MarkdownConversionError> {
    let Some(math) = math else {
        return Ok(());
    };
    if math.enabled == Some(OptionBool::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid mathRendering.enabled: Must be a boolean",
        ));
    }
    if math.unsupported == Some(MathUnsupported::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid mathRendering.unsupported: Must be \"text\" or \"throw\"",
        ));
    }
    Ok(())
}

fn validate_chart_rendering(
    chart: Option<&ChartRenderingOptions>,
) -> Result<(), MarkdownConversionError> {
    let Some(chart) = chart else {
        return Ok(());
    };
    if chart.enabled == Some(OptionBool::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid chartRendering.enabled: Must be a boolean",
        ));
    }
    validate_positive_integer(chart.width, "chartRendering.width")?;
    validate_positive_integer(chart.height, "chartRendering.height")?;
    validate_positive_integer(chart.max_width, "chartRendering.maxWidth")?;
    validate_positive_integer(chart.max_height, "chartRendering.maxHeight")?;
    if chart.invalid_definition_behavior == Some(ChartInvalidBehavior::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid chartRendering.invalidDefinitionBehavior: must be placeholder or throw",
        ));
    }
    Ok(())
}

fn validate_mermaid_rendering(
    mermaid: Option<&MermaidRenderingOptions>,
) -> Result<(), MarkdownConversionError> {
    let Some(mermaid) = mermaid else {
        return Ok(());
    };
    if mermaid.enabled == Some(OptionBool::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid mermaidRendering.enabled: Must be a boolean",
        ));
    }
    if mermaid.failure_mode == Some(MermaidFailureMode::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid mermaidRendering.failureMode: must be codeBlock, placeholder, or throw",
        ));
    }
    Ok(())
}

/// `codeHighlighting`. Omitted, or `enabled` other than `true`, leaves the
/// plain Courier path. Theme keys are the highlight.js names without `hljs-`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeHighlightingOptions {
    pub enabled: Option<OptionBool>,
    pub theme: Option<CodeHighlightTheme>,
    pub languages: Option<Vec<String>>,
    /// `None` shows the language label. Only `false` hides it.
    pub show_language_label: Option<OptionBool>,
    /// Replaces the built-in Syntect tokenizer. The Node binding uses it to
    /// run lowlight with the full theme, so colored runs match the
    /// TypeScript renderer exactly.
    pub tokenizer: Option<HighlightTokenizer>,
}

type HighlightFn = dyn Fn(&str, &str) -> Result<Option<Vec<crate::highlight::HighlightSpan>>, MarkdownConversionError>
    + Send
    + Sync;

/// A trusted tokenizer callback: `(code, language)` to colored spans.
/// `Ok(None)` means the language is unknown and the block stays plain.
#[derive(Clone)]
pub struct HighlightTokenizer {
    tokenize: Arc<HighlightFn>,
}

impl HighlightTokenizer {
    pub fn new<F>(tokenize: F) -> Self
    where
        F: Fn(
                &str,
                &str,
            )
                -> Result<Option<Vec<crate::highlight::HighlightSpan>>, MarkdownConversionError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            tokenize: Arc::new(tokenize),
        }
    }

    pub fn tokenize(
        &self,
        code: &str,
        language: &str,
    ) -> Result<Option<Vec<crate::highlight::HighlightSpan>>, MarkdownConversionError> {
        (self.tokenize)(code, language)
    }
}

impl std::fmt::Debug for HighlightTokenizer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HighlightTokenizer")
    }
}

impl PartialEq for HighlightTokenizer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.tokenize, &other.tokenize)
    }
}

impl Eq for HighlightTokenizer {}

/// Partial highlight theme. Absent keys keep the built-in GitHub-light colors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeHighlightTheme {
    pub default: Option<String>,
    pub background: Option<String>,
    pub border: Option<String>,
    pub language_label: Option<String>,
}

/// `textReplacementMode`. `None` means trusted. `Invalid` is any other value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextReplacementMode {
    Trusted,
    Untrusted,
    Invalid,
}

/// What a replacement searches for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFind {
    Literal(String),
    Regex(String),
}

/// What a match is rewritten to.
#[derive(Debug, Clone, PartialEq)]
pub enum TextReplace {
    Text(String),
    Function(TextReplacementFunction),
}

/// One find-and-replace applied to text nodes, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct TextReplacement {
    pub find: TextFind,
    pub replace: TextReplace,
}

/// Trusted replacement callback. The argument is the matched text.
#[derive(Clone)]
pub struct TextReplacementFunction {
    replace: Arc<dyn Fn(&str) -> String + Send + Sync>,
}

impl TextReplacementFunction {
    pub fn new<F>(replace: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        Self {
            replace: Arc::new(replace),
        }
    }

    pub fn call(&self, matched: &str) -> String {
        (self.replace)(matched)
    }
}

impl std::fmt::Debug for TextReplacementFunction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TextReplacementFunction")
    }
}

impl PartialEq for TextReplacementFunction {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.replace, &other.replace)
    }
}

fn validate_text_replacements(options: &ConvertOptions) -> Result<(), MarkdownConversionError> {
    let mode = options
        .text_replacement_mode
        .unwrap_or(TextReplacementMode::Trusted);
    if mode == TextReplacementMode::Invalid {
        return Err(MarkdownConversionError::new(
            "Invalid textReplacementMode: Must be trusted or untrusted",
        ));
    }
    let Some(replacements) = options.text_replacements.as_ref() else {
        return Ok(());
    };
    if replacements.is_empty() {
        return Ok(());
    }
    if replacements.len() > 50 {
        return Err(MarkdownConversionError::new(
            "Failed to convert markdown to docx: textReplacements supports at most 50 entries",
        ));
    }
    for replacement in replacements {
        if mode == TextReplacementMode::Untrusted
            && matches!(replacement.replace, TextReplace::Function(_))
        {
            return Err(MarkdownConversionError::new(
                "Function textReplacements are not allowed when textReplacementMode is \"untrusted\"",
            ));
        }
        if let TextReplace::Text(text) = &replacement.replace {
            if utf16_len(text) > 4096 {
                return Err(MarkdownConversionError::new(
                    "Failed to convert markdown to docx: textReplacements replacement text is too large",
                ));
            }
        }
        match &replacement.find {
            TextFind::Regex(source) if is_unsafe_regex(source) => {
                return Err(MarkdownConversionError::new(
                    "Unsafe textReplacements RegExp rejected",
                ));
            }
            TextFind::Literal(source) if utf16_len(source) > 256 => {
                return Err(MarkdownConversionError::new(
                    "Failed to convert markdown to docx: textReplacements string pattern is too large",
                ));
            }
            TextFind::Regex(_) | TextFind::Literal(_) => {}
        }
    }
    Ok(())
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn is_unsafe_regex(source: &str) -> bool {
    if utf16_len(source) > 256 || nested_quantifier(source) {
        return true;
    }
    regex::Regex::new(source).is_err()
}

/// The two shapes `isUnsafeRegex` rejects: a quantified group that already
/// contains a quantifier, and a quantified group that contains an alternation.
fn nested_quantifier(source: &str) -> bool {
    const SHAPES: [&str; 2] = [
        r"\((?:[^()\\]|\\.)*[*+?](?:[^()\\]|\\.)*\)[*+?{]",
        r"\((?:[^()\\]|\\.)*\|(?:[^()\\]|\\.)*\)[*+?{]",
    ];
    SHAPES.iter().any(|shape| {
        regex::Regex::new(shape)
            .map(|pattern| pattern.is_match(source))
            .unwrap_or(false)
    })
}

fn validate_positive_integer(
    value: Option<u32>,
    name: &str,
) -> Result<(), MarkdownConversionError> {
    if value == Some(0) {
        return Err(MarkdownConversionError::new(format!(
            "{name} must be a positive integer"
        )));
    }
    Ok(())
}

/// Labels, placement, and failure behavior for figure and table captions.
///
/// Omitted fields keep the writer defaults: label `Figure` or `Table`,
/// placement below the anchor, center alignment, and field update on open.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CaptionOptions {
    pub figure_label: Option<String>,
    pub table_label: Option<String>,
    pub figure_placement: Option<CaptionPlacement>,
    pub table_placement: Option<CaptionPlacement>,
    pub alignment: Option<Alignment>,
    pub italic: Option<bool>,
    pub size: Option<u32>,
    pub failure_mode: Option<CaptionFailureMode>,
    /// `None` asks Word to update fields when a caption is defined.
    pub update_fields_on_open: Option<UpdateFieldsOnOpen>,
}

/// Where a caption sits relative to its image or table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionPlacement {
    Above,
    Below,
}

/// `preserve` leaves bad caption syntax as text. `throw` returns
/// [`MarkdownConversionError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionFailureMode {
    Preserve,
    Throw,
}

/// JSON `captions.updateFieldsOnOpen`.
///
/// `Bool(false)` suppresses `w:updateFields`. `Invalid` is a non-boolean JSON
/// value and fails validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateFieldsOnOpen {
    Bool(bool),
    Invalid,
}

fn validate_captions(captions: Option<&CaptionOptions>) -> Result<(), MarkdownConversionError> {
    let Some(captions) = captions else {
        return Ok(());
    };
    for (name, value) in [
        ("figureLabel", captions.figure_label.as_deref()),
        ("tableLabel", captions.table_label.as_deref()),
    ] {
        if value.is_some_and(|text| text.trim().is_empty()) {
            return Err(MarkdownConversionError::new(format!(
                "Invalid captions.{name}: Must be a non-empty string"
            )));
        }
    }
    if captions.size.is_some_and(|size| !(8..=144).contains(&size)) {
        return Err(MarkdownConversionError::new(
            "Invalid captions.size: Must be between 8 and 144 half-points",
        ));
    }
    if captions.update_fields_on_open == Some(UpdateFieldsOnOpen::Invalid) {
        return Err(MarkdownConversionError::new(
            "Invalid captions.updateFieldsOnOpen: Must be a boolean",
        ));
    }
    Ok(())
}

fn validate_style(
    style: Option<&StyleOverrides>,
    style_context: &str,
) -> Result<(), MarkdownConversionError> {
    let Some(style) = style else {
        return Ok(());
    };
    let range_error = |message: &str, key: &str, value: f64| {
        let mut context = ErrorContext::new();
        context.insert("styleContext".to_string(), style_context.to_string());
        context.insert(key.to_string(), js_number_string(value));
        MarkdownConversionError::with_context(message, context)
    };
    if let Some(size) = style.title_size.filter(|size| !(8..=72).contains(size)) {
        return Err(range_error(
            "Invalid title size: Must be between 8 and 72 points",
            "titleSize",
            f64::from(size),
        ));
    }
    if let Some(spacing) = style.heading_spacing.filter(|spacing| *spacing > 720) {
        return Err(range_error(
            "Invalid heading spacing: Must be between 0 and 720 twips",
            "headingSpacing",
            f64::from(spacing),
        ));
    }
    if let Some(spacing) = style.paragraph_spacing.filter(|spacing| *spacing > 720) {
        return Err(range_error(
            "Invalid paragraph spacing: Must be between 0 and 720 twips",
            "paragraphSpacing",
            f64::from(spacing),
        ));
    }
    if let Some(spacing) = style
        .line_spacing
        .filter(|spacing| !(1.0..=3.0).contains(spacing))
    {
        return Err(range_error(
            "Invalid line spacing: Must be between 1 and 3",
            "lineSpacing",
            spacing,
        ));
    }
    if let Some(font) = font_after_normalize(style) {
        if font.trim().is_empty() {
            return Err(MarkdownConversionError::new(
                "Invalid fontFamily: Must be a non-empty string",
            ));
        }
    }
    if let Some(language) = &style.language {
        validate_ooxml_text(
            language,
            &format!("{style_context}.language"),
            MAX_METADATA_LANGUAGE_LENGTH,
            false,
        )?;
        if !is_bcp47(language) {
            let mut context = ErrorContext::new();
            context.insert("styleContext".to_string(), style_context.to_string());
            context.insert("language".to_string(), language.clone());
            return Err(MarkdownConversionError::with_context(
                "Invalid style language: Must be a BCP 47-style language tag",
                context,
            ));
        }
    }
    if let Some(widths) = &style.table_column_widths {
        let valid = |width: &f64| width.fract() == 0.0 && (1.0..=31680.0).contains(width);
        if widths.is_empty() || !widths.iter().all(valid) {
            return Err(MarkdownConversionError::new("Invalid tableColumnWidths"));
        }
    }
    if style.table_allow_row_split == Some(OptionBool::Invalid) {
        return Err(MarkdownConversionError::new("Invalid tableAllowRowSplit"));
    }
    validate_hex_color(
        style.table_header_background.as_deref(),
        "tableHeaderBackground",
        style_context,
    )?;
    if let Some(margins) = &style.table_cell_margins {
        let valid = |value: f64| value.fract() == 0.0 && (0.0..=31680.0).contains(&value);
        if [margins.top, margins.bottom, margins.left, margins.right]
            .into_iter()
            .flatten()
            .any(|value| !valid(value))
        {
            return Err(MarkdownConversionError::new("Invalid tableCellMargins"));
        }
    }
    validate_hex_color(
        style.inline_code_color.as_deref(),
        "inlineCodeColor",
        style_context,
    )?;
    validate_hex_color(
        style.inline_code_background.as_deref(),
        "inlineCodeBackground",
        style_context,
    )?;
    if let Some(callouts) = &style.callout_styles {
        if !callouts.is_object {
            let mut context = ErrorContext::new();
            context.insert("styleContext".to_string(), style_context.to_string());
            return Err(MarkdownConversionError::with_context(
                "Invalid calloutStyles: Must be an object",
                context,
            ));
        }
        for (name, entry) in &callouts.entries {
            if !CALLOUT_NAMES.contains(&name.as_str()) {
                let mut context = ErrorContext::new();
                context.insert("styleContext".to_string(), style_context.to_string());
                context.insert("calloutType".to_string(), name.clone());
                return Err(MarkdownConversionError::with_context(
                    format!("Invalid calloutStyles key: {name}"),
                    context,
                ));
            }
            let Some(entry) = entry else {
                let mut context = ErrorContext::new();
                context.insert("styleContext".to_string(), style_context.to_string());
                return Err(MarkdownConversionError::with_context(
                    format!("Invalid calloutStyles.{name}: Must be an object"),
                    context,
                ));
            };
            validate_hex_color(
                entry.border_color.as_deref(),
                &format!("calloutStyles.{name}.borderColor"),
                style_context,
            )?;
            validate_hex_color(
                entry.background_color.as_deref(),
                &format!("calloutStyles.{name}.backgroundColor"),
                style_context,
            )?;
            validate_hex_color(
                entry.title_color.as_deref(),
                &format!("calloutStyles.{name}.titleColor"),
                style_context,
            )?;
        }
    }
    Ok(())
}

/// The valid `calloutStyles` keys.
pub const CALLOUT_NAMES: [&str; 5] = ["note", "tip", "important", "warning", "caution"];

/// `validateHexColorOption`.
fn validate_hex_color(
    value: Option<&str>,
    name: &str,
    style_context: &str,
) -> Result<(), MarkdownConversionError> {
    let Some(value) = value else {
        return Ok(());
    };
    let is_hex = value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_hex {
        return Ok(());
    }
    let mut context = ErrorContext::new();
    context.insert("context".to_string(), style_context.to_string());
    context.insert("value".to_string(), value.to_string());
    Err(MarkdownConversionError::with_context(
        format!("{name} must be a 6-character hex color"),
        context,
    ))
}

/// `String(value)` for a finite number.
fn js_number_string(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// `normalizeStyleInput` keeps a set `fontFamily` when it is non-empty in the
/// JavaScript sense, otherwise it copies `fontFamilly`. An empty `fontFamily`
/// stays on the object when the alias does not replace it.
fn font_after_normalize(style: &StyleOverrides) -> Option<String> {
    if let Some(font) = js_truthy(&style.font_family) {
        return Some(font);
    }
    if let Some(font) = js_truthy(&style.font_familly) {
        return Some(font);
    }
    style.font_family.clone()
}

fn js_truthy(value: &Option<String>) -> Option<String> {
    value.clone().filter(|text| !text.is_empty())
}

/// One section's Markdown plus its config.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSection {
    pub markdown: String,
    pub config: SectionConfig,
}

/// Shared section settings. The template and each section use this shape.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SectionConfig {
    pub style: Option<StyleOverrides>,
    pub page: Option<PageInput>,
    pub headers: Option<HeaderFooterGroup>,
    pub footers: Option<HeaderFooterGroup>,
    pub page_numbering: Option<PageNumberingInput>,
    pub title_page: Option<bool>,
    pub break_type: Option<SectionBreak>,
}

/// `documentType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentType {
    Document,
    Report,
}

/// `tableLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLayout {
    Autofit,
    Fixed,
}

/// `tableCellMargins`. Values are kept as given so validation can reject
/// fractions and out-of-range numbers with the TypeScript messages.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellMargins {
    pub top: Option<f64>,
    pub bottom: Option<f64>,
    pub left: Option<f64>,
    pub right: Option<f64>,
}

/// One `calloutStyles` entry. Colours are kept as the JavaScript string of
/// the given value so the hex check sees what the TypeScript check sees.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalloutStyleInput {
    pub label: Option<String>,
    pub border_color: Option<String>,
    pub background_color: Option<String>,
    pub title_color: Option<String>,
}

/// `calloutStyles` as given. `entries` keeps unknown keys and non-object
/// values so validation can report them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalloutStylesInput {
    pub is_object: bool,
    pub entries: Vec<(String, Option<CalloutStyleInput>)>,
}

impl CalloutStylesInput {
    /// The entry for a valid callout name, if any.
    pub fn get(&self, name: &str) -> Option<&CalloutStyleInput> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_ref())
    }
}

/// Partial style. Absent fields do not override a lower-precedence value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleOverrides {
    pub title_size: Option<u32>,
    pub heading_spacing: Option<u32>,
    pub paragraph_spacing: Option<u32>,
    pub line_spacing: Option<f64>,
    pub paragraph_alignment: Option<Alignment>,
    pub direction: Option<Direction>,
    pub font_family: Option<String>,
    /// Typo alias of `font_family`. `font_family` wins when both are non-empty.
    pub font_familly: Option<String>,
    pub language: Option<String>,
    pub heading_size: [Option<u32>; 6],
    pub heading_alignment: Option<Alignment>,
    pub heading_alignments: [Option<Alignment>; 6],
    pub paragraph_size: Option<u32>,
    pub list_item_size: Option<u32>,
    pub code_block_size: Option<u32>,
    pub blockquote_size: Option<u32>,
    pub toc_font_size: Option<u32>,
    pub toc_heading_font_size: [Option<u32>; 6],
    pub toc_heading_bold: [Option<bool>; 6],
    pub toc_heading_italic: [Option<bool>; 6],
    pub inline_code_size: Option<u32>,
    pub inline_code_color: Option<String>,
    pub inline_code_background: Option<String>,
    pub blockquote_alignment: Option<Alignment>,
    pub code_block_alignment: Option<Alignment>,
    pub table_layout: Option<TableLayout>,
    pub table_column_widths: Option<Vec<f64>>,
    pub table_allow_row_split: Option<OptionBool>,
    pub table_header_background: Option<String>,
    pub table_cell_margins: Option<CellMargins>,
    pub callout_styles: Option<CalloutStylesInput>,
}

/// Paragraph alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justified,
}

/// Document direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// Page orientation. Landscape uses the height value as the displayed width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// Word section break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionBreak {
    NextPage,
    NextColumn,
    Continuous,
    EvenPage,
    OddPage,
}

/// How a footer shows page numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumberDisplay {
    None,
    Current,
    CurrentAndTotal,
    CurrentAndSectionTotal,
}

/// Page number format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumberFormat {
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperLetter,
    LowerLetter,
}

/// Separator between chapter and page numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageNumberSeparator {
    Hyphen,
    Period,
    Colon,
    EmDash,
    EnDash,
}

/// Native TOC field, or the older static linked list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocMode {
    Native,
    Links,
}

/// Partial TOC settings. Native mode is the default.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TocOverrides {
    pub mode: Option<TocMode>,
    pub page_numbers: Option<bool>,
    pub dot_leaders: Option<bool>,
    pub update_fields_on_open: Option<bool>,
    /// `None` keeps the default title. `Some("")` omits the title.
    pub title: Option<String>,
    pub min_depth: Option<u8>,
    pub max_depth: Option<u8>,
}

/// Page size and margins. Absent fields inherit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageInput {
    pub margin: Option<MarginInput>,
    pub size: Option<SizeInput>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarginInput {
    pub top: Option<u32>,
    pub right: Option<u32>,
    pub bottom: Option<u32>,
    pub left: Option<u32>,
    pub header: Option<u32>,
    pub footer: Option<u32>,
    pub gutter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SizeInput {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<Orientation>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageNumberingInput {
    pub start: Option<u32>,
    pub format: Option<PageNumberFormat>,
    pub separator: Option<PageNumberSeparator>,
    pub display: Option<PageNumberDisplay>,
    pub alignment: Option<Alignment>,
}

/// `default`, `first`, and `even`. `None` inherits. `Remove` drops the slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeaderFooterGroup {
    pub default: Option<HeaderFooterSlot>,
    pub first: Option<HeaderFooterSlot>,
    pub even: Option<HeaderFooterSlot>,
}

/// One header or footer slot.
#[derive(Debug, Clone, PartialEq)]
pub enum HeaderFooterSlot {
    /// JSON `null`. Drops this slot, including one inherited from the template.
    Remove,
    Content(HeaderFooterContent),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeaderFooterContent {
    pub markdown: Option<String>,
    pub text: Option<String>,
    pub alignment: Option<Alignment>,
    pub page_number_display: Option<PageNumberDisplay>,
}

pub const DEFAULT_PAGE_WIDTH: u32 = 11906;
pub const DEFAULT_PAGE_HEIGHT: u32 = 16838;
pub const DEFAULT_MARGIN_TOP: u32 = 1440;
pub const DEFAULT_MARGIN_RIGHT: u32 = 1080;
pub const DEFAULT_MARGIN_BOTTOM: u32 = 1440;
pub const DEFAULT_MARGIN_LEFT: u32 = 1080;

/// Style after precedence has been applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStyle {
    pub title_size: u32,
    pub heading_spacing: u32,
    pub paragraph_spacing: u32,
    pub line_spacing: f64,
    pub paragraph_alignment: Alignment,
    pub direction: Direction,
    pub font_family: Option<String>,
    pub language: Option<String>,
    pub heading_size: [Option<u32>; 6],
    pub heading_alignment: Option<Alignment>,
    pub heading_alignments: [Option<Alignment>; 6],
    pub paragraph_size: Option<u32>,
    pub list_item_size: Option<u32>,
    pub code_block_size: Option<u32>,
    pub blockquote_size: Option<u32>,
    pub toc_font_size: Option<u32>,
    pub toc_heading_font_size: [Option<u32>; 6],
    pub toc_heading_bold: [Option<bool>; 6],
    pub toc_heading_italic: [Option<bool>; 6],
    pub inline_code_size: Option<u32>,
    pub inline_code_color: Option<String>,
    pub inline_code_background: Option<String>,
    pub blockquote_alignment: Option<Alignment>,
    pub code_block_alignment: Option<Alignment>,
    pub table_layout: Option<TableLayout>,
    pub table_column_widths: Option<Vec<u32>>,
    pub table_allow_row_split: Option<bool>,
    pub table_header_background: Option<String>,
    pub table_cell_margins: Option<CellMargins>,
    pub callout_styles: Option<CalloutStylesInput>,
}

impl ResolvedStyle {
    /// Half-points for a heading run, matching `headingRunSize`.
    pub fn heading_run_size(&self, level: u8) -> u32 {
        let level = level.clamp(1, 6);
        let size = if level == 6 {
            self.heading_size[5].unwrap_or_else(|| self.title_size.saturating_sub(20))
        } else {
            self.title_size
                .saturating_sub(u32::from(level.saturating_sub(1)) * 4)
        };
        size.max(1)
    }
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        Self {
            title_size: 32,
            heading_spacing: 240,
            paragraph_spacing: 240,
            line_spacing: 1.15,
            paragraph_alignment: Alignment::Left,
            direction: Direction::Ltr,
            font_family: None,
            language: None,
            heading_size: [None; 6],
            heading_alignment: None,
            heading_alignments: [None; 6],
            paragraph_size: None,
            list_item_size: None,
            code_block_size: None,
            blockquote_size: None,
            toc_font_size: None,
            toc_heading_font_size: [None; 6],
            toc_heading_bold: [None; 6],
            toc_heading_italic: [None; 6],
            inline_code_size: None,
            inline_code_color: None,
            inline_code_background: None,
            blockquote_alignment: None,
            code_block_alignment: None,
            table_layout: None,
            table_column_widths: None,
            table_allow_row_split: None,
            table_header_background: None,
            table_cell_margins: None,
            callout_styles: None,
        }
    }
}

/// TOC settings after defaults are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocSettings {
    pub mode: TocMode,
    pub page_numbers: bool,
    pub dot_leaders: bool,
    pub update_fields_on_open: bool,
    pub title: String,
    pub min_depth: u8,
    pub max_depth: u8,
}

impl Default for TocSettings {
    fn default() -> Self {
        Self {
            mode: TocMode::Native,
            page_numbers: true,
            dot_leaders: true,
            update_fields_on_open: true,
            title: "Table of Contents".to_string(),
            min_depth: 1,
            max_depth: 6,
        }
    }
}

/// Page box after defaults and inheritance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageGeometry {
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
    pub margin_top: u32,
    pub margin_right: u32,
    pub margin_bottom: u32,
    pub margin_left: u32,
    pub margin_header: Option<u32>,
    pub margin_footer: Option<u32>,
    pub margin_gutter: Option<u32>,
    /// Usable width in twips. Tables use this as a DXA integer.
    pub content_width: u32,
}

impl Default for PageGeometry {
    fn default() -> Self {
        geometry_from(None, None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedPageNumbering {
    pub start: Option<u32>,
    pub format: Option<PageNumberFormat>,
    pub separator: Option<PageNumberSeparator>,
    pub display: Option<PageNumberDisplay>,
    pub alignment: Option<Alignment>,
}

/// One header or footer slot after Markdown has not yet been parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSlotInput {
    pub content: HeaderFooterContent,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolvedHeaderFooterInput {
    pub default: Option<ResolvedSlotInput>,
    pub first: Option<ResolvedSlotInput>,
    pub even: Option<ResolvedSlotInput>,
}

/// One section after style, page, and slot inheritance, before parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSectionInput {
    pub markdown: String,
    pub style: ResolvedStyle,
    pub page: PageGeometry,
    pub page_numbering: Option<ResolvedPageNumbering>,
    pub title_page: Option<bool>,
    pub break_type: Option<SectionBreak>,
    pub headers: ResolvedHeaderFooterInput,
    pub footers: ResolvedHeaderFooterInput,
}

/// Resolve sections the way `resolveSections` does.
///
/// An empty or missing `sections` list uses `markdown` as the only section.
pub fn resolve_sections(markdown: &str, options: &ConvertOptions) -> Vec<ResolvedSectionInput> {
    let mut base = ResolvedStyle::default();
    if let Some(language) = options
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.language.clone())
    {
        base.language = Some(language);
    }
    let base = resolve_style(Some(&base), options.style.as_ref());
    let template = options.template.as_ref();
    let sections: Vec<(&str, Option<&SectionConfig>)> = match options
        .sections
        .as_ref()
        .filter(|sections| !sections.is_empty())
    {
        Some(sections) => sections
            .iter()
            .map(|section| (section.markdown.as_str(), Some(&section.config)))
            .collect(),
        None => vec![(markdown, None)],
    };
    sections
        .into_iter()
        .map(|(section_markdown, config)| resolve_one(section_markdown, &base, template, config))
        .collect()
}

pub fn resolve_toc(overrides: Option<&TocOverrides>) -> TocSettings {
    let mut settings = TocSettings::default();
    let Some(overrides) = overrides else {
        return settings;
    };
    if let Some(mode) = overrides.mode {
        settings.mode = mode;
    }
    if let Some(page_numbers) = overrides.page_numbers {
        settings.page_numbers = page_numbers;
    }
    if let Some(dot_leaders) = overrides.dot_leaders {
        settings.dot_leaders = dot_leaders;
    }
    if let Some(update) = overrides.update_fields_on_open {
        settings.update_fields_on_open = update;
    }
    if let Some(title) = &overrides.title {
        settings.title = title.clone();
    }
    if let Some(min_depth) = overrides.min_depth {
        settings.min_depth = min_depth;
    }
    if let Some(max_depth) = overrides.max_depth {
        settings.max_depth = max_depth;
    }
    settings
}

fn resolve_one(
    markdown: &str,
    base: &ResolvedStyle,
    template: Option<&SectionConfig>,
    section: Option<&SectionConfig>,
) -> ResolvedSectionInput {
    let mut style = base.clone();
    if let Some(template_style) = template.and_then(|template| template.style.as_ref()) {
        apply_style(&mut style, template_style);
    }
    if let Some(section_style) = section.and_then(|section| section.style.as_ref()) {
        apply_style(&mut style, section_style);
    }
    ResolvedSectionInput {
        markdown: markdown.to_string(),
        style,
        page: resolve_page(
            template.and_then(|template| template.page.as_ref()),
            section.and_then(|section| section.page.as_ref()),
        ),
        page_numbering: resolve_numbering(
            template.and_then(|template| template.page_numbering.as_ref()),
            section.and_then(|section| section.page_numbering.as_ref()),
        ),
        title_page: section
            .and_then(|section| section.title_page)
            .or_else(|| template.and_then(|template| template.title_page)),
        break_type: section
            .and_then(|section| section.break_type)
            .or_else(|| template.and_then(|template| template.break_type)),
        headers: resolve_group(
            template.and_then(|template| template.headers.as_ref()),
            section.and_then(|section| section.headers.as_ref()),
        ),
        footers: resolve_group(
            template.and_then(|template| template.footers.as_ref()),
            section.and_then(|section| section.footers.as_ref()),
        ),
    }
}

pub(crate) fn resolve_style(
    base: Option<&ResolvedStyle>,
    overrides: Option<&StyleOverrides>,
) -> ResolvedStyle {
    let mut style = base.cloned().unwrap_or_default();
    if let Some(overrides) = overrides {
        apply_style(&mut style, overrides);
    }
    style
}

fn apply_style(style: &mut ResolvedStyle, overrides: &StyleOverrides) {
    if let Some(value) = overrides.title_size {
        style.title_size = value;
    }
    if let Some(value) = overrides.heading_spacing {
        style.heading_spacing = value;
    }
    if let Some(value) = overrides.paragraph_spacing {
        style.paragraph_spacing = value;
    }
    if let Some(value) = overrides.line_spacing {
        style.line_spacing = value;
    }
    if let Some(value) = overrides.paragraph_alignment {
        style.paragraph_alignment = value;
    }
    if let Some(value) = overrides.direction {
        style.direction = value;
    }
    if let Some(font) = resolved_font(overrides) {
        style.font_family = Some(font);
    }
    if let Some(value) = &overrides.language {
        style.language = Some(value.clone());
    }
    for index in 0..6 {
        if overrides.heading_size[index].is_some() {
            style.heading_size[index] = overrides.heading_size[index];
        }
        if overrides.heading_alignments[index].is_some() {
            style.heading_alignments[index] = overrides.heading_alignments[index];
        }
        if overrides.toc_heading_font_size[index].is_some() {
            style.toc_heading_font_size[index] = overrides.toc_heading_font_size[index];
        }
        if overrides.toc_heading_bold[index].is_some() {
            style.toc_heading_bold[index] = overrides.toc_heading_bold[index];
        }
        if overrides.toc_heading_italic[index].is_some() {
            style.toc_heading_italic[index] = overrides.toc_heading_italic[index];
        }
    }
    if let Some(value) = overrides.heading_alignment {
        style.heading_alignment = Some(value);
    }
    if let Some(value) = overrides.paragraph_size {
        style.paragraph_size = Some(value);
    }
    if let Some(value) = overrides.list_item_size {
        style.list_item_size = Some(value);
    }
    if let Some(value) = overrides.code_block_size {
        style.code_block_size = Some(value);
    }
    if let Some(value) = overrides.blockquote_size {
        style.blockquote_size = Some(value);
    }
    if let Some(value) = overrides.toc_font_size {
        style.toc_font_size = Some(value);
    }
    if let Some(value) = overrides.inline_code_size {
        style.inline_code_size = Some(value);
    }
    if let Some(value) = &overrides.inline_code_color {
        style.inline_code_color = Some(value.clone());
    }
    if let Some(value) = &overrides.inline_code_background {
        style.inline_code_background = Some(value.clone());
    }
    if let Some(value) = overrides.blockquote_alignment {
        style.blockquote_alignment = Some(value);
    }
    if let Some(value) = overrides.code_block_alignment {
        style.code_block_alignment = Some(value);
    }
    if let Some(value) = overrides.table_layout {
        style.table_layout = Some(value);
    }
    if let Some(widths) = &overrides.table_column_widths {
        style.table_column_widths = Some(widths.iter().map(|width| *width as u32).collect());
    }
    if let Some(OptionBool::Bool(value)) = overrides.table_allow_row_split {
        style.table_allow_row_split = Some(value);
    }
    if let Some(value) = &overrides.table_header_background {
        style.table_header_background = Some(value.clone());
    }
    if let Some(value) = &overrides.table_cell_margins {
        style.table_cell_margins = Some(value.clone());
    }
    if let Some(value) = &overrides.callout_styles {
        style.callout_styles = Some(value.clone());
    }
}

fn resolved_font(overrides: &StyleOverrides) -> Option<String> {
    nonempty(&overrides.font_family).or_else(|| nonempty(&overrides.font_familly))
}

fn nonempty(value: &Option<String>) -> Option<String> {
    value.clone().filter(|text| !text.is_empty())
}

fn resolve_page(template: Option<&PageInput>, section: Option<&PageInput>) -> PageGeometry {
    geometry_from(
        merge_size(
            template.and_then(|page| page.size.as_ref()),
            section.and_then(|page| page.size.as_ref()),
        ),
        merge_margin(
            template.and_then(|page| page.margin.as_ref()),
            section.and_then(|page| page.margin.as_ref()),
        ),
    )
}

fn merge_size(template: Option<&SizeInput>, section: Option<&SizeInput>) -> Option<SizeInput> {
    match (template, section) {
        (None, None) => None,
        (Some(template), None) => Some(template.clone()),
        (None, Some(section)) => Some(section.clone()),
        (Some(template), Some(section)) => Some(SizeInput {
            width: section.width.or(template.width),
            height: section.height.or(template.height),
            orientation: section.orientation.or(template.orientation),
        }),
    }
}

fn merge_margin(
    template: Option<&MarginInput>,
    section: Option<&MarginInput>,
) -> Option<MarginInput> {
    match (template, section) {
        (None, None) => None,
        (Some(template), None) => Some(template.clone()),
        (None, Some(section)) => Some(section.clone()),
        (Some(template), Some(section)) => Some(MarginInput {
            top: section.top.or(template.top),
            right: section.right.or(template.right),
            bottom: section.bottom.or(template.bottom),
            left: section.left.or(template.left),
            header: section.header.or(template.header),
            footer: section.footer.or(template.footer),
            gutter: section.gutter.or(template.gutter),
        }),
    }
}

fn geometry_from(size: Option<SizeInput>, margin: Option<MarginInput>) -> PageGeometry {
    let width = size
        .as_ref()
        .and_then(|size| size.width)
        .unwrap_or(DEFAULT_PAGE_WIDTH);
    let height = size
        .as_ref()
        .and_then(|size| size.height)
        .unwrap_or(DEFAULT_PAGE_HEIGHT);
    let orientation = size
        .as_ref()
        .and_then(|size| size.orientation)
        .unwrap_or(Orientation::Portrait);
    let margin_top = margin
        .as_ref()
        .and_then(|margin| margin.top)
        .unwrap_or(DEFAULT_MARGIN_TOP);
    let margin_right = margin
        .as_ref()
        .and_then(|margin| margin.right)
        .unwrap_or(DEFAULT_MARGIN_RIGHT);
    let margin_bottom = margin
        .as_ref()
        .and_then(|margin| margin.bottom)
        .unwrap_or(DEFAULT_MARGIN_BOTTOM);
    let margin_left = margin
        .as_ref()
        .and_then(|margin| margin.left)
        .unwrap_or(DEFAULT_MARGIN_LEFT);
    let displayed = match orientation {
        Orientation::Landscape => height,
        Orientation::Portrait => width,
    };
    let content_width = displayed
        .saturating_sub(margin_left)
        .saturating_sub(margin_right)
        .max(1);
    PageGeometry {
        width,
        height,
        orientation,
        margin_top,
        margin_right,
        margin_bottom,
        margin_left,
        margin_header: margin.as_ref().and_then(|margin| margin.header),
        margin_footer: margin.as_ref().and_then(|margin| margin.footer),
        margin_gutter: margin.as_ref().and_then(|margin| margin.gutter),
        content_width,
    }
}

fn resolve_numbering(
    template: Option<&PageNumberingInput>,
    section: Option<&PageNumberingInput>,
) -> Option<ResolvedPageNumbering> {
    let numbering = ResolvedPageNumbering {
        start: section
            .and_then(|value| value.start)
            .or_else(|| template.and_then(|value| value.start)),
        format: section
            .and_then(|value| value.format)
            .or_else(|| template.and_then(|value| value.format)),
        separator: section
            .and_then(|value| value.separator)
            .or_else(|| template.and_then(|value| value.separator)),
        display: section
            .and_then(|value| value.display)
            .or_else(|| template.and_then(|value| value.display)),
        alignment: section
            .and_then(|value| value.alignment)
            .or_else(|| template.and_then(|value| value.alignment)),
    };
    if numbering.start.is_none()
        && numbering.format.is_none()
        && numbering.separator.is_none()
        && numbering.display.is_none()
        && numbering.alignment.is_none()
    {
        None
    } else {
        Some(numbering)
    }
}

fn resolve_group(
    template: Option<&HeaderFooterGroup>,
    section: Option<&HeaderFooterGroup>,
) -> ResolvedHeaderFooterInput {
    if template.is_none() && section.is_none() {
        return ResolvedHeaderFooterInput::default();
    }
    ResolvedHeaderFooterInput {
        default: resolve_slot(
            template.and_then(|group| group.default.as_ref()),
            section.and_then(|group| group.default.as_ref()),
        ),
        first: resolve_slot(
            template.and_then(|group| group.first.as_ref()),
            section.and_then(|group| group.first.as_ref()),
        ),
        even: resolve_slot(
            template.and_then(|group| group.even.as_ref()),
            section.and_then(|group| group.even.as_ref()),
        ),
    }
}

fn resolve_slot(
    template: Option<&HeaderFooterSlot>,
    section: Option<&HeaderFooterSlot>,
) -> Option<ResolvedSlotInput> {
    match section {
        Some(HeaderFooterSlot::Remove) => None,
        None => slot_content(template),
        Some(HeaderFooterSlot::Content(section_content)) => match template {
            Some(HeaderFooterSlot::Content(template_content)) => Some(ResolvedSlotInput {
                content: merge_content(template_content, section_content),
            }),
            _ => Some(ResolvedSlotInput {
                content: section_content.clone(),
            }),
        },
    }
}

fn slot_content(slot: Option<&HeaderFooterSlot>) -> Option<ResolvedSlotInput> {
    match slot {
        Some(HeaderFooterSlot::Content(content)) => Some(ResolvedSlotInput {
            content: content.clone(),
        }),
        Some(HeaderFooterSlot::Remove) | None => None,
    }
}

fn merge_content(
    template: &HeaderFooterContent,
    section: &HeaderFooterContent,
) -> HeaderFooterContent {
    HeaderFooterContent {
        markdown: section
            .markdown
            .clone()
            .or_else(|| template.markdown.clone()),
        text: section.text.clone().or_else(|| template.text.clone()),
        alignment: section.alignment.or(template.alignment),
        page_number_display: section.page_number_display.or(template.page_number_display),
    }
}

pub const MAX_METADATA_VALUE_LENGTH: usize = 4096;
pub const MAX_METADATA_LANGUAGE_LENGTH: usize = 64;
pub const MAX_METADATA_KEYWORDS: usize = 128;
pub const MAX_CUSTOM_PROPERTIES: usize = 64;
pub const MAX_CUSTOM_PROPERTY_NAME_LENGTH: usize = 255;

/// Word package properties. `creator` wins over `author` when both are set.
///
/// `None` on [`ConvertOptions::metadata`] leaves the package alone. `Some`
/// with every field omitted is still an explicit metadata set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub author: Option<String>,
    /// Joined with `"; "` in document order.
    pub keywords: Option<Vec<String>>,
    pub category: Option<String>,
    pub company: Option<String>,
    pub language: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    /// Omitted means leave custom properties alone. An empty map still writes
    /// the custom part on a new package.
    pub custom: Option<BTreeMap<String, String>>,
}

/// Metadata after validation. Timestamps are UTC `toISOString` values.
/// `custom: None` means the field was omitted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedMetadata {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub keywords: Option<String>,
    pub category: Option<String>,
    pub company: Option<String>,
    pub language: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub custom: Option<BTreeMap<String, String>>,
}

pub fn validate_document_metadata(
    metadata: Option<&DocumentMetadata>,
) -> Result<(), MarkdownConversionError> {
    if let Some(metadata) = metadata {
        normalize_document_metadata(metadata)?;
    }
    Ok(())
}

pub fn normalize_document_metadata(
    metadata: &DocumentMetadata,
) -> Result<NormalizedMetadata, MarkdownConversionError> {
    for (value, context) in [
        (metadata.title.as_deref(), "metadata.title"),
        (metadata.subject.as_deref(), "metadata.subject"),
        (metadata.description.as_deref(), "metadata.description"),
        (metadata.creator.as_deref(), "metadata.creator"),
        (metadata.author.as_deref(), "metadata.author"),
        (metadata.category.as_deref(), "metadata.category"),
        (metadata.company.as_deref(), "metadata.company"),
    ] {
        if let Some(value) = value {
            validate_ooxml_text(value, context, MAX_METADATA_VALUE_LENGTH, false)?;
        }
    }
    if let Some(language) = &metadata.language {
        validate_ooxml_text(
            language,
            "metadata.language",
            MAX_METADATA_LANGUAGE_LENGTH,
            false,
        )?;
        if !is_bcp47(language) {
            let mut context = ErrorContext::new();
            context.insert("language".to_string(), language.clone());
            return Err(MarkdownConversionError::with_context(
                "metadata.language must be a BCP 47-style language tag",
                context,
            ));
        }
    }
    validate_custom_properties(metadata.custom.as_ref())?;
    Ok(NormalizedMetadata {
        title: metadata.title.clone(),
        subject: metadata.subject.clone(),
        description: metadata.description.clone(),
        creator: metadata.creator.clone().or_else(|| metadata.author.clone()),
        keywords: normalize_keywords(metadata.keywords.as_deref())?,
        category: metadata.category.clone(),
        company: metadata.company.clone(),
        language: metadata.language.clone(),
        created: normalize_optional_timestamp(metadata.created.as_deref(), "metadata.created")?,
        modified: normalize_optional_timestamp(metadata.modified.as_deref(), "metadata.modified")?,
        custom: metadata.custom.clone(),
    })
}

fn normalize_keywords(
    keywords: Option<&[String]>,
) -> Result<Option<String>, MarkdownConversionError> {
    let Some(items) = keywords else {
        return Ok(None);
    };
    if items.is_empty() || items.len() > MAX_METADATA_KEYWORDS {
        let mut context = ErrorContext::new();
        context.insert("count".to_string(), items.len().to_string());
        context.insert("maxCount".to_string(), MAX_METADATA_KEYWORDS.to_string());
        return Err(MarkdownConversionError::with_context(
            format!("metadata.keywords must contain between 1 and {MAX_METADATA_KEYWORDS} entries"),
            context,
        ));
    }
    for (index, keyword) in items.iter().enumerate() {
        validate_ooxml_text(
            keyword,
            &format!("metadata.keywords[{index}]"),
            MAX_METADATA_VALUE_LENGTH,
            false,
        )?;
    }
    let joined = items.join("; ");
    validate_ooxml_text(
        &joined,
        "metadata.keywords",
        MAX_METADATA_VALUE_LENGTH,
        false,
    )?;
    Ok(Some(joined))
}

fn validate_custom_properties(
    custom: Option<&BTreeMap<String, String>>,
) -> Result<(), MarkdownConversionError> {
    let Some(custom) = custom else {
        return Ok(());
    };
    if custom.len() > MAX_CUSTOM_PROPERTIES {
        let mut context = ErrorContext::new();
        context.insert("count".to_string(), custom.len().to_string());
        context.insert("maxCount".to_string(), MAX_CUSTOM_PROPERTIES.to_string());
        return Err(MarkdownConversionError::with_context(
            format!("metadata.custom exceeds the maximum of {MAX_CUSTOM_PROPERTIES} properties"),
            context,
        ));
    }
    let mut names = BTreeMap::<String, ()>::new();
    for (name, value) in custom {
        validate_ooxml_text(
            name,
            "metadata.custom property name",
            MAX_CUSTOM_PROPERTY_NAME_LENGTH,
            false,
        )?;
        let folded = name.to_lowercase();
        if names.insert(folded, ()).is_some() {
            let mut context = ErrorContext::new();
            context.insert("name".to_string(), name.clone());
            return Err(MarkdownConversionError::with_context(
                "metadata.custom property names must be unique ignoring case",
                context,
            ));
        }
        validate_ooxml_text(
            value,
            &format!("metadata.custom.{name}"),
            MAX_METADATA_VALUE_LENGTH,
            true,
        )?;
    }
    Ok(())
}

fn normalize_optional_timestamp(
    value: Option<&str>,
    context: &str,
) -> Result<Option<String>, MarkdownConversionError> {
    let Some(value) = value else {
        return Ok(None);
    };
    match normalize_timestamp(value) {
        Ok(text) => Ok(Some(text)),
        Err(TimestampError::Format) => Err(metadata_error(
            format!("{context} must be a valid ISO date"),
            &[("context", context)],
        )),
        Err(TimestampError::Invalid) => Err(metadata_error(
            format!("{context} must be a valid date"),
            &[("context", context)],
        )),
    }
}

#[derive(Debug)]
enum TimestampError {
    Format,
    Invalid,
}

struct TimestampParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
    offset_minutes: i32,
}

fn normalize_timestamp(value: &str) -> Result<String, TimestampError> {
    let parts = parse_timestamp(value)?;
    timestamp_to_utc(parts)
}

fn parse_timestamp(value: &str) -> Result<TimestampParts, TimestampError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let year = take_digits(bytes, &mut index, 4).ok_or(TimestampError::Format)?;
    take_byte(bytes, &mut index, b'-').ok_or(TimestampError::Format)?;
    let month = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
    take_byte(bytes, &mut index, b'-').ok_or(TimestampError::Format)?;
    let day = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
    take_byte(bytes, &mut index, b'T').ok_or(TimestampError::Format)?;
    let hour = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
    take_byte(bytes, &mut index, b':').ok_or(TimestampError::Format)?;
    let minute = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
    take_byte(bytes, &mut index, b':').ok_or(TimestampError::Format)?;
    let second = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
    let millis = if take_byte(bytes, &mut index, b'.').is_some() {
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let width = index - start;
        if !(1..=9).contains(&width) {
            return Err(TimestampError::Format);
        }
        let mut digits = value[start..index].to_string();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits[..3].parse::<u32>().unwrap_or(0)
    } else {
        0
    };
    let offset_minutes = if take_byte(bytes, &mut index, b'Z').is_some() {
        0
    } else {
        let sign = if take_byte(bytes, &mut index, b'+').is_some() {
            1
        } else if take_byte(bytes, &mut index, b'-').is_some() {
            -1
        } else {
            return Err(TimestampError::Format);
        };
        let offset_hour = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
        take_byte(bytes, &mut index, b':').ok_or(TimestampError::Format)?;
        let offset_minute = take_digits(bytes, &mut index, 2).ok_or(TimestampError::Format)?;
        sign * (offset_hour * 60 + offset_minute)
    };
    if index != bytes.len() {
        return Err(TimestampError::Format);
    }
    let month = u32::try_from(month).unwrap_or(0);
    let day = u32::try_from(day).unwrap_or(0);
    let hour = u32::try_from(hour).unwrap_or(99);
    let minute = u32::try_from(minute).unwrap_or(99);
    let second = u32::try_from(second).unwrap_or(99);
    let offset_hour = offset_minutes.unsigned_abs() / 60;
    let offset_minute = offset_minutes.unsigned_abs() % 60;
    let clock_ok = hour < 24 || (hour == 24 && minute == 0 && second == 0 && millis == 0);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !clock_ok
        || minute > 59
        || second > 59
        || offset_hour > 23
        || offset_minute > 59
    {
        return Err(TimestampError::Invalid);
    }
    Ok(TimestampParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        millis,
        offset_minutes,
    })
}

fn take_digits(bytes: &[u8], index: &mut usize, width: usize) -> Option<i32> {
    let end = index.checked_add(width)?;
    let text = bytes.get(*index..end)?;
    if !text.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    *index = end;
    std::str::from_utf8(text).ok()?.parse().ok()
}

fn take_byte(bytes: &[u8], index: &mut usize, expected: u8) -> Option<()> {
    if bytes.get(*index).copied() != Some(expected) {
        return None;
    }
    *index += 1;
    Some(())
}

fn timestamp_to_utc(parts: TimestampParts) -> Result<String, TimestampError> {
    let days = days_from_civil(parts.year, parts.month, parts.day);
    let local_ms = days
        .checked_mul(86_400_000)
        .and_then(|ms| ms.checked_add(i64::from(parts.hour) * 3_600_000))
        .and_then(|ms| ms.checked_add(i64::from(parts.minute) * 60_000))
        .and_then(|ms| ms.checked_add(i64::from(parts.second) * 1_000))
        .and_then(|ms| ms.checked_add(i64::from(parts.millis)))
        .ok_or(TimestampError::Invalid)?;
    let utc_ms = local_ms
        .checked_sub(i64::from(parts.offset_minutes) * 60_000)
        .ok_or(TimestampError::Invalid)?;
    let days = utc_ms.div_euclid(86_400_000);
    let time = utc_ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let hour = u32::try_from(time / 3_600_000).unwrap_or(0);
    let minute = u32::try_from((time % 3_600_000) / 60_000).unwrap_or(0);
    let second = u32::try_from((time % 60_000) / 1_000).unwrap_or(0);
    let millis = u32::try_from(time % 1_000).unwrap_or(0);
    Ok(format!(
        "{}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z",
        format_year(year)
    ))
}

fn format_year(year: i32) -> String {
    if (0..10_000).contains(&year) {
        format!("{year:04}")
    } else if year >= 0 {
        format!("+{year:06}")
    } else {
        format!("-{:06}", year.unsigned_abs())
    }
}

fn days_from_civil(mut year: i32, month: u32, day: u32) -> i64 {
    year -= i32::from(month <= 2);
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = u64::try_from(year - era * 400).unwrap_or(0);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * u64::from(shifted) + 2) / 5 + u64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era) * 146_097 + i64::try_from(doe).unwrap_or(0) - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = i32::try_from(yoe).unwrap_or(0) + i32::try_from(era).unwrap_or(0) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn validate_ooxml_text(
    value: &str,
    context: &str,
    max_length: usize,
    allow_empty: bool,
) -> Result<(), MarkdownConversionError> {
    let length = utf16_units(value);
    if !allow_empty && value.is_empty() {
        return Err(metadata_error(
            format!("{context} must not be empty"),
            &[("context", context)],
        ));
    }
    if length > max_length {
        return Err(metadata_error(
            format!("{context} exceeds the maximum length of {max_length} characters"),
            &[
                ("context", context),
                ("length", &length.to_string()),
                ("maxLength", &max_length.to_string()),
            ],
        ));
    }
    if value.chars().any(invalid_ooxml_char) {
        return Err(metadata_error(
            format!("{context} contains characters that are not valid in OOXML"),
            &[("context", context)],
        ));
    }
    Ok(())
}

fn invalid_ooxml_char(ch: char) -> bool {
    matches!(ch,
        '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' | '\u{FFFE}' | '\u{FFFF}'
    )
}

fn is_bcp47(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    if !(2..=8).contains(&primary.len()) || !primary.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    parts.all(|part| {
        (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn utf16_units(value: &str) -> usize {
    value.encode_utf16().count()
}

fn metadata_error(message: String, pairs: &[(&str, &str)]) -> MarkdownConversionError {
    let mut context = ErrorContext::new();
    for (key, value) in pairs {
        context.insert((*key).to_string(), (*value).to_string());
    }
    MarkdownConversionError::with_context(message, context)
}

/// Markdown role that can adopt a style from a reference document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceStyleRole {
    Normal,
    Title,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    Blockquote,
    CodeBlock,
    Caption,
    ListParagraph,
    Table,
    Strong,
    Emphasis,
    InlineCode,
    Hyperlink,
}

/// Conventional style ids and names from `AUTO_STYLE_CANDIDATES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoStyleCandidates {
    pub ids: &'static [&'static str],
    pub names: &'static [&'static str],
}

pub fn auto_style_candidates(role: ReferenceStyleRole) -> AutoStyleCandidates {
    let (ids, names): (&'static [&'static str], &'static [&'static str]) = match role {
        ReferenceStyleRole::Normal => (&["Normal"], &["Normal"]),
        ReferenceStyleRole::Title => (&["Title"], &["Title"]),
        ReferenceStyleRole::Heading1 => (&["Heading1"], &["Heading 1", "heading 1"]),
        ReferenceStyleRole::Heading2 => (&["Heading2"], &["Heading 2", "heading 2"]),
        ReferenceStyleRole::Heading3 => (&["Heading3"], &["Heading 3", "heading 3"]),
        ReferenceStyleRole::Heading4 => (&["Heading4"], &["Heading 4", "heading 4"]),
        ReferenceStyleRole::Heading5 => (&["Heading5"], &["Heading 5", "heading 5"]),
        ReferenceStyleRole::Heading6 => (&["Heading6"], &["Heading 6", "heading 6"]),
        ReferenceStyleRole::Blockquote => (
            &["BlockQuote", "Quote", "IntenseQuote"],
            &["Block Quote", "Quote", "Intense Quote"],
        ),
        ReferenceStyleRole::CodeBlock => (&["CodeBlock", "Code"], &["Code Block", "Code"]),
        ReferenceStyleRole::Caption => (&["Caption"], &["Caption"]),
        ReferenceStyleRole::ListParagraph => (&["ListParagraph"], &["List Paragraph"]),
        ReferenceStyleRole::Table => (&["TableNormal"], &["Normal Table", "Table Normal"]),
        ReferenceStyleRole::Strong => (&["Strong"], &["Strong"]),
        ReferenceStyleRole::Emphasis => (&["Emphasis"], &["Emphasis"]),
        ReferenceStyleRole::InlineCode => {
            (&["InlineCode", "CodeChar"], &["Inline Code", "Code Char"])
        }
        ReferenceStyleRole::Hyperlink => (&["Hyperlink"], &["Hyperlink"]),
    };
    AutoStyleCandidates { ids, names }
}

/// Explicit `{ id }` or `{ name }` selector. The two forms are exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceStyleSelector {
    Id(String),
    Name(String),
}

/// `null` keeps generated styling. A selector adopts that reference style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceStyleChoice {
    Generated,
    Selector(ReferenceStyleSelector),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingStyleBehavior {
    #[default]
    Fallback,
    Throw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DuplicateStyleNameBehavior {
    #[default]
    Throw,
    First,
}

pub const DEFAULT_MAX_COMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
pub const DEFAULT_MAX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
pub const DEFAULT_MAX_ENTRIES: u64 = 512;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferenceDocxPackageLimits {
    pub max_compressed_bytes: Option<u64>,
    pub max_uncompressed_bytes: Option<u64>,
    pub max_entry_uncompressed_bytes: Option<u64>,
    pub max_entries: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedReferenceLimits {
    pub max_compressed_bytes: u64,
    pub max_uncompressed_bytes: u64,
    pub max_entry_uncompressed_bytes: u64,
    pub max_entries: u64,
}

impl Default for ResolvedReferenceLimits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: DEFAULT_MAX_COMPRESSED_BYTES,
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            max_entry_uncompressed_bytes: DEFAULT_MAX_ENTRY_UNCOMPRESSED_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

/// Reference-mode options. Absent roles use [`auto_style_candidates`].
///
/// Page layout and headers default to preserved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferenceDocxModeOptions {
    pub styles: Option<BTreeMap<String, ReferenceStyleSelector>>,
    pub missing_style_behavior: Option<MissingStyleBehavior>,
    pub duplicate_style_name_behavior: Option<DuplicateStyleNameBehavior>,
    pub preserve_page_layout: Option<bool>,
    pub preserve_headers_and_footers: Option<bool>,
    pub limits: Option<ReferenceDocxPackageLimits>,
}

/// Bytes-API options for generating a document from a reference package.
///
/// `aborted` stands in for an already-aborted `AbortSignal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReferenceDocxGenerationOptions {
    /// Conversion settings applied before adopting reference presentation.
    pub convert: ConvertOptions,
    pub reference: Option<ReferenceDocxModeOptions>,
    pub aborted: bool,
}

impl ReferenceDocxModeOptions {
    pub fn missing_style_behavior(&self) -> MissingStyleBehavior {
        self.missing_style_behavior
            .unwrap_or(MissingStyleBehavior::Fallback)
    }

    pub fn duplicate_style_name_behavior(&self) -> DuplicateStyleNameBehavior {
        self.duplicate_style_name_behavior
            .unwrap_or(DuplicateStyleNameBehavior::Throw)
    }

    pub fn preserve_page_layout(&self) -> bool {
        self.preserve_page_layout.unwrap_or(true)
    }

    pub fn preserve_headers_and_footers(&self) -> bool {
        self.preserve_headers_and_footers.unwrap_or(true)
    }

    pub fn resolved_limits(&self) -> Result<ResolvedReferenceLimits, MarkdownConversionError> {
        let limits = self.limits.clone().unwrap_or_default();
        Ok(ResolvedReferenceLimits {
            max_compressed_bytes: resolve_limit(
                "reference.limits.maxCompressedBytes",
                limits.max_compressed_bytes,
                DEFAULT_MAX_COMPRESSED_BYTES,
            )?,
            max_uncompressed_bytes: resolve_limit(
                "reference.limits.maxUncompressedBytes",
                limits.max_uncompressed_bytes,
                DEFAULT_MAX_UNCOMPRESSED_BYTES,
            )?,
            max_entry_uncompressed_bytes: resolve_limit(
                "reference.limits.maxEntryUncompressedBytes",
                limits.max_entry_uncompressed_bytes,
                DEFAULT_MAX_ENTRY_UNCOMPRESSED_BYTES,
            )?,
            max_entries: resolve_limit(
                "reference.limits.maxEntries",
                limits.max_entries,
                DEFAULT_MAX_ENTRIES,
            )?,
        })
    }
}

pub(crate) fn validate_reference_options(
    reference: Option<&ReferenceDocxModeOptions>,
) -> Result<(), MarkdownConversionError> {
    if let Some(reference) = reference {
        reference.resolved_limits()?;
    }
    Ok(())
}

fn resolve_limit(
    name: &str,
    value: Option<u64>,
    default: u64,
) -> Result<u64, MarkdownConversionError> {
    let value = value.unwrap_or(default);
    if value == 0 || value > MAX_SAFE_INTEGER {
        let mut context = ErrorContext::new();
        context.insert("phase".to_string(), "reference-docx".to_string());
        context.insert("code".to_string(), "INVALID_INPUT".to_string());
        context.insert("actual".to_string(), value.to_string());
        return Err(MarkdownConversionError::with_context(
            format!("{name} must be a positive safe integer"),
            context,
        ));
    }
    Ok(value)
}

/// One placeholder replacement. A Markdown string is this with `style: None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkdownPatch {
    pub markdown: String,
    pub style: Option<StyleOverrides>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderDelimiters {
    pub start: String,
    pub end: String,
}

impl Default for PlaceholderDelimiters {
    fn default() -> Self {
        Self {
            start: "{{".to_string(),
            end: "}}".to_string(),
        }
    }
}

/// Options for placeholder patch.
///
/// `recursive` defaults to true. `keep_original_styles` is accepted and does
/// not replace explicit formatting. Delimiters default to `{{` and `}}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatchMarkdownOptions {
    /// Conversion settings for the Markdown inserted into each placeholder.
    pub convert: ConvertOptions,
    pub metadata: Option<DocumentMetadata>,
    pub keep_original_styles: Option<bool>,
    pub placeholder_delimiters: Option<PlaceholderDelimiters>,
    pub recursive: Option<bool>,
    pub table_width_twips: Option<u32>,
    pub max_input_length: Option<u32>,
    pub max_elements: Option<u32>,
}

impl PatchMarkdownOptions {
    pub fn recursive(&self) -> bool {
        self.recursive.unwrap_or(true)
    }

    pub fn delimiters(&self) -> (&str, &str) {
        match &self.placeholder_delimiters {
            Some(delimiters) => (delimiters.start.as_str(), delimiters.end.as_str()),
            None => ("{{", "}}"),
        }
    }

    pub fn validate(&self) -> Result<(), MarkdownConversionError> {
        self.convert.validate()?;
        validate_document_metadata(self.metadata.as_ref())?;
        validate_positive_integer(self.max_input_length, "maxInputLength")?;
        validate_positive_integer(self.max_elements, "maxElements")?;
        if let Some(delimiters) = &self.placeholder_delimiters {
            if delimiters.start.trim().is_empty() || delimiters.end.trim().is_empty() {
                return Err(MarkdownConversionError::new(
                    "Invalid placeholderDelimiters: start and end must be non-empty strings",
                ));
            }
        }
        if self.table_width_twips == Some(0) {
            let mut context = ErrorContext::new();
            context.insert("tableWidthTwips".to_string(), "0".to_string());
            return Err(MarkdownConversionError::with_context(
                "Invalid tableWidthTwips: Must be a positive integer",
                context,
            ));
        }
        Ok(())
    }
}

/// `pluginOptions.conflictPolicy`. The default is [`PluginConflictPolicy::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginConflictPolicy {
    #[default]
    Error,
    UsePriority,
}

/// What a fence or block-node handler does when its renderer returns an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginFailureMode {
    #[default]
    Fallback,
    Skip,
    Throw,
}

/// `pluginOptions`. Only `conflict_policy` is set in this slice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginOptions {
    pub conflict_policy: Option<PluginConflictPolicy>,
}

/// One inline run returned by a plugin renderer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginInline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<String>,
}

/// Semantic block a plugin renderer may return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginBlock {
    Paragraph { children: Vec<PluginInline> },
    Skip,
}

/// Fence body handed to a fence renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginFenceInput {
    pub language: String,
    pub value: String,
    pub meta: Option<String>,
}

/// Custom block handed to a block-node renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBlockNodeInput {
    pub node_type: String,
    pub value: String,
}

/// Called once, in priority order, before any section is rendered.
#[derive(Clone)]
pub struct PluginSetup {
    setup: Arc<dyn Fn() + Send + Sync>,
}

impl PluginSetup {
    pub fn new<F>(setup: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        Self {
            setup: Arc::new(setup),
        }
    }

    pub(crate) fn call(&self) {
        (self.setup)();
    }
}

impl std::fmt::Debug for PluginSetup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginSetup")
    }
}

impl PartialEq for PluginSetup {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.setup, &other.setup)
    }
}

/// Runs after text replacements and before the element limit, once per section.
#[derive(Clone)]
pub struct PluginAstTransform {
    transform: Arc<dyn Fn(&mut crate::model::Document) + Send + Sync>,
}

impl PluginAstTransform {
    pub fn new<F>(transform: F) -> Self
    where
        F: Fn(&mut crate::model::Document) + Send + Sync + 'static,
    {
        Self {
            transform: Arc::new(transform),
        }
    }

    pub(crate) fn call(&self, document: &mut crate::model::Document) {
        (self.transform)(document);
    }
}

impl std::fmt::Debug for PluginAstTransform {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginAstTransform")
    }
}

impl PartialEq for PluginAstTransform {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.transform, &other.transform)
    }
}

/// Fence renderer. `render_fn` builds one from a closure.
#[derive(Clone)]
pub struct PluginFenceRender {
    render: Arc<PluginFenceRenderFn>,
}

impl PluginFenceRender {
    pub(crate) fn call(
        &self,
        input: &PluginFenceInput,
    ) -> Result<Vec<PluginBlock>, MarkdownConversionError> {
        (self.render)(input)
    }
}

impl std::fmt::Debug for PluginFenceRender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginFenceRender")
    }
}

impl PartialEq for PluginFenceRender {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.render, &other.render)
    }
}

/// Block-node renderer. `render_fn` builds one from a closure.
#[derive(Clone)]
pub struct PluginBlockNodeRender {
    render: Arc<PluginNodeRenderFn>,
}

impl PluginBlockNodeRender {
    pub fn call(
        &self,
        input: &PluginBlockNodeInput,
    ) -> Result<Vec<PluginBlock>, MarkdownConversionError> {
        (self.render)(input)
    }
}

impl std::fmt::Debug for PluginBlockNodeRender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginBlockNodeRender")
    }
}

impl PartialEq for PluginBlockNodeRender {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.render, &other.render)
    }
}

/// Languages are matched after trim and lowercase.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginFenceHandler {
    pub languages: Vec<String>,
    pub failure_mode: Option<PluginFailureMode>,
    pub render: PluginFenceRender,
}

impl PluginFenceHandler {
    pub fn render_fn<F>(render: F) -> PluginFenceRender
    where
        F: Fn(&PluginFenceInput) -> Result<Vec<PluginBlock>, MarkdownConversionError>
            + Send
            + Sync
            + 'static,
    {
        PluginFenceRender {
            render: Arc::new(render),
        }
    }
}

/// Node types are matched exactly, after trim, with case preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginBlockNodeHandler {
    pub node_types: Vec<String>,
    pub failure_mode: Option<PluginFailureMode>,
    pub render: PluginBlockNodeRender,
}

impl PluginBlockNodeHandler {
    pub fn render_fn<F>(render: F) -> PluginBlockNodeRender
    where
        F: Fn(&PluginBlockNodeInput) -> Result<Vec<PluginBlock>, MarkdownConversionError>
            + Send
            + Sync
            + 'static,
    {
        PluginBlockNodeRender {
            render: Arc::new(render),
        }
    }
}

/// Trusted plugin passed on [`ConvertOptions::plugins`].
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownDocxPlugin {
    pub api_version: u32,
    pub name: String,
    pub priority: Option<i32>,
    pub setup: Option<PluginSetup>,
    pub transform_ast: Option<PluginAstTransform>,
    pub fenced_blocks: Option<Vec<PluginFenceHandler>>,
    pub block_nodes: Option<Vec<PluginBlockNodeHandler>>,
}

/// Fence handler selected for one language.
#[derive(Clone)]
pub(crate) struct PluginFenceBinding {
    pub(crate) plugin_name: String,
    pub(crate) failure_mode: PluginFailureMode,
    render: PluginFenceRender,
}

impl PluginFenceBinding {
    pub(crate) fn render(
        &self,
        input: &PluginFenceInput,
    ) -> Result<Vec<PluginBlock>, MarkdownConversionError> {
        self.render.call(input)
    }
}

pub(crate) struct PluginNodeBinding {
    pub(crate) plugin_name: String,
    pub(crate) failure_mode: PluginFailureMode,
    render: PluginBlockNodeRender,
}
impl PluginNodeBinding {
    pub(crate) fn render(
        &self,
        input: &PluginBlockNodeInput,
    ) -> Result<Vec<PluginBlock>, MarkdownConversionError> {
        self.render.call(input)
    }
}

/// Registered plugins for one conversion.
///
/// `create` returns `None` when the host passed no plugins. Setup runs once.
/// [`PluginRuntime::transform`] runs once per section, before fence handlers.
pub(crate) struct PluginRuntime {
    transforms: Vec<PluginAstTransform>,
    fences: BTreeMap<String, PluginFenceBinding>,
    nodes: BTreeMap<String, PluginNodeBinding>,
}

impl PluginRuntime {
    pub(crate) fn create(
        options: &ConvertOptions,
    ) -> Result<Option<Self>, MarkdownConversionError> {
        validate_plugins(options)?;
        let Some(plugins) = options
            .plugins
            .as_ref()
            .filter(|plugins| !plugins.is_empty())
        else {
            return Ok(None);
        };
        let mut order: Vec<usize> = (0..plugins.len()).collect();
        order.sort_by(|&left, &right| {
            let left_priority = plugins[left].priority.unwrap_or(0);
            let right_priority = plugins[right].priority.unwrap_or(0);
            right_priority
                .cmp(&left_priority)
                .then_with(|| left.cmp(&right))
        });
        let mut transforms = Vec::new();
        let mut fences = BTreeMap::new();
        let mut nodes = BTreeMap::new();
        for index in order {
            let plugin = &plugins[index];
            if let Some(setup) = &plugin.setup {
                setup.call();
            }
            if let Some(transform) = &plugin.transform_ast {
                transforms.push(transform.clone());
            }
            for handler in plugin.block_nodes.as_deref().unwrap_or_default() {
                for name in &handler.node_types {
                    nodes
                        .entry(name.trim().to_string())
                        .or_insert_with(|| PluginNodeBinding {
                            plugin_name: plugin.name.clone(),
                            failure_mode: handler
                                .failure_mode
                                .unwrap_or(PluginFailureMode::Fallback),
                            render: handler.render.clone(),
                        });
                }
            }
            let Some(handlers) = plugin.fenced_blocks.as_ref() else {
                continue;
            };
            for handler in handlers {
                let failure_mode = handler.failure_mode.unwrap_or(PluginFailureMode::Fallback);
                for language in &handler.languages {
                    let language = language.trim().to_ascii_lowercase();
                    fences
                        .entry(language)
                        .or_insert_with(|| PluginFenceBinding {
                            plugin_name: plugin.name.clone(),
                            failure_mode,
                            render: handler.render.clone(),
                        });
                }
            }
        }
        Ok(Some(Self {
            transforms,
            fences,
            nodes,
        }))
    }

    pub(crate) fn transform(&self, document: &mut crate::model::Document) {
        for transform in &self.transforms {
            transform.call(document);
        }
    }

    pub(crate) fn node(&self, name: &str) -> Option<&PluginNodeBinding> {
        self.nodes.get(name)
    }

    pub(crate) fn fence(&self, language: &str) -> Option<&PluginFenceBinding> {
        self.fences.get(language)
    }
}

const BUILT_IN_BLOCK_NODES: &[&str] = &[
    "root",
    "heading",
    "paragraph",
    "list",
    "listItem",
    "code",
    "math",
    "blockquote",
    "image",
    "table",
    "tableRow",
    "tableCell",
    "html",
    "thematicBreak",
    "footnoteDefinition",
    "footnoteReference",
    "definition",
    "yaml",
];

fn validate_plugins(options: &ConvertOptions) -> Result<(), MarkdownConversionError> {
    let Some(plugins) = options.plugins.as_ref() else {
        return Ok(());
    };
    let conflict_policy = options
        .plugin_options
        .as_ref()
        .and_then(|plugin_options| plugin_options.conflict_policy)
        .unwrap_or(PluginConflictPolicy::Error);
    let mut names = BTreeSet::new();
    let mut fence_owners: BTreeMap<String, String> = BTreeMap::new();
    let mut node_owners: BTreeMap<String, String> = BTreeMap::new();
    for plugin in plugins {
        if plugin.api_version != 1 {
            return Err(MarkdownConversionError::new(
                "Unsupported plugin apiVersion",
            ));
        }
        if !valid_plugin_name(&plugin.name) {
            return Err(MarkdownConversionError::new(
                "Invalid plugin name: use a stable lowercase identifier",
            ));
        }
        if !names.insert(plugin.name.clone()) {
            return Err(MarkdownConversionError::new("Duplicate plugin name"));
        }
        if let Some(priority) = plugin.priority {
            if !(-1000..=1000).contains(&priority) {
                return Err(MarkdownConversionError::new(
                    "Invalid plugin priority: must be an integer from -1000 to 1000",
                ));
            }
        }
        if let Some(handlers) = plugin.fenced_blocks.as_ref() {
            for handler in handlers {
                let languages = normalize_handler_names(&handler.languages, "fence", true)?;
                for language in languages {
                    if reserved_fence(options, &language) {
                        return Err(MarkdownConversionError::new(
                            "Plugin fence conflicts with an enabled built-in renderer",
                        ));
                    }
                    let owner = fence_owners.get(&language).cloned();
                    match owner {
                        Some(owner) if owner == plugin.name => {
                            return Err(MarkdownConversionError::new(
                                "Duplicate plugin fence handler",
                            ));
                        }
                        Some(_) if conflict_policy == PluginConflictPolicy::Error => {
                            return Err(MarkdownConversionError::new(
                                "Conflicting plugin fence handlers",
                            ));
                        }
                        Some(_) => {}
                        None => {
                            fence_owners.insert(language, plugin.name.clone());
                        }
                    }
                }
            }
        }
        if let Some(handlers) = plugin.block_nodes.as_ref() {
            for handler in handlers {
                let node_types = normalize_handler_names(&handler.node_types, "blockNode", false)?;
                for node_type in &handler.node_types {
                    if BUILT_IN_BLOCK_NODES.contains(&node_type.as_str()) {
                        return Err(MarkdownConversionError::new(
                            "Plugin block-node handler cannot override a built-in node type",
                        ));
                    }
                }
                for node_type in node_types {
                    let owner = node_owners.get(&node_type).cloned();
                    match owner {
                        Some(owner) if owner == plugin.name => {
                            return Err(MarkdownConversionError::new(
                                "Duplicate plugin block-node handler",
                            ));
                        }
                        Some(_) if conflict_policy == PluginConflictPolicy::Error => {
                            return Err(MarkdownConversionError::new(
                                "Conflicting plugin block-node handlers",
                            ));
                        }
                        Some(_) => {}
                        None => {
                            node_owners.insert(node_type, plugin.name.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn normalize_handler_names(
    names: &[String],
    hook: &str,
    fence: bool,
) -> Result<Vec<String>, MarkdownConversionError> {
    if names.is_empty() {
        return Err(MarkdownConversionError::new(format!(
            "Plugin {hook} handler must declare at least one name"
        )));
    }
    let normalized: Vec<String> = names
        .iter()
        .map(|name| {
            let trimmed = name.trim();
            if fence {
                trimmed.to_ascii_lowercase()
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    let valid = normalized.iter().all(|name| {
        if fence {
            valid_fence_name(name)
        } else {
            valid_block_node_name(name)
        }
    });
    if !valid {
        return Err(MarkdownConversionError::new(format!(
            "Invalid plugin {hook} handler name"
        )));
    }
    let unique = normalized.iter().collect::<BTreeSet<_>>();
    if unique.len() != normalized.len() {
        return Err(MarkdownConversionError::new(format!(
            "Duplicate plugin {hook} handler name"
        )));
    }
    Ok(normalized)
}

fn reserved_fence(options: &ConvertOptions, language: &str) -> bool {
    match language {
        "mermaid" => matches!(
            options
                .mermaid_rendering
                .as_ref()
                .and_then(|mermaid| mermaid.enabled),
            Some(OptionBool::Bool(true))
        ),
        "chart" | "chartjs" => matches!(
            options
                .chart_rendering
                .as_ref()
                .and_then(|chart| chart.enabled),
            Some(OptionBool::Bool(true))
        ),
        _ => false,
    }
}

fn valid_plugin_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.first().is_none_or(|byte| !byte.is_ascii_lowercase()) {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() && is_plugin_tail(bytes[index]) {
        index += 1;
    }
    while index < bytes.len() {
        if !matches!(bytes[index], b'.' | b'_' | b'-') {
            return false;
        }
        index += 1;
        let start = index;
        while index < bytes.len() && is_plugin_tail(bytes[index]) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    true
}

fn is_plugin_tail(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn valid_fence_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'-'))
}

fn valid_block_node_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod metadata_normalization {
    use super::{normalize_document_metadata, normalize_timestamp, DocumentMetadata};

    #[test]
    fn timestamps_match_javascript_to_iso_string() {
        let cases = [
            ("2026-01-02T03:04:05Z", "2026-01-02T03:04:05.000Z"),
            ("2026-01-02T03:04:05.1Z", "2026-01-02T03:04:05.100Z"),
            ("2026-01-02T03:04:05.123456789Z", "2026-01-02T03:04:05.123Z"),
            ("2026-01-02T03:04:05+01:00", "2026-01-02T02:04:05.000Z"),
            ("2026-01-02T03:04:05-05:30", "2026-01-02T08:34:05.000Z"),
            ("2026-02-31T00:00:00Z", "2026-03-03T00:00:00.000Z"),
            ("2023-02-29T00:00:00Z", "2023-03-01T00:00:00.000Z"),
            ("2026-01-02T24:00:00Z", "2026-01-03T00:00:00.000Z"),
            ("2026-04-31T00:00:00Z", "2026-05-01T00:00:00.000Z"),
            ("0000-01-01T00:00:00+01:00", "-000001-12-31T23:00:00.000Z"),
            ("9999-12-31T23:00:00-02:00", "+010000-01-01T01:00:00.000Z"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_timestamp(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn creator_wins_and_keywords_join() {
        let metadata = DocumentMetadata {
            creator: Some("Primary creator".to_string()),
            author: Some("Ignored alias".to_string()),
            keywords: Some(vec![
                "docx".to_string(),
                "a11y".to_string(),
                "Unicode 二".to_string(),
            ]),
            created: Some("2026-01-02T03:04:05Z".to_string()),
            ..DocumentMetadata::default()
        };
        let normalized = normalize_document_metadata(&metadata).unwrap();
        assert_eq!(normalized.creator.as_deref(), Some("Primary creator"));
        assert_eq!(
            normalized.keywords.as_deref(),
            Some("docx; a11y; Unicode 二")
        );
        assert_eq!(
            normalized.created.as_deref(),
            Some("2026-01-02T03:04:05.000Z")
        );
    }

    #[test]
    fn rejects_controls_length_dates_language_and_custom_count() {
        let control = normalize_document_metadata(&DocumentMetadata {
            title: Some("bad\u{0001}value".to_string()),
            ..DocumentMetadata::default()
        })
        .unwrap_err();
        assert_eq!(
            control
                .context()
                .unwrap()
                .get("context")
                .map(String::as_str),
            Some("metadata.title")
        );
        let oversized = normalize_document_metadata(&DocumentMetadata {
            description: Some("x".repeat(4097)),
            ..DocumentMetadata::default()
        })
        .unwrap_err();
        assert!(oversized.message().contains("maximum length of 4096"));
        let date = normalize_document_metadata(&DocumentMetadata {
            created: Some("not-a-date".to_string()),
            ..DocumentMetadata::default()
        })
        .unwrap_err();
        assert_eq!(date.message(), "metadata.created must be a valid ISO date");
        let language = normalize_document_metadata(&DocumentMetadata {
            language: Some("not a tag".to_string()),
            ..DocumentMetadata::default()
        })
        .unwrap_err();
        assert!(language.message().contains("BCP 47-style"));
        let custom = normalize_document_metadata(&DocumentMetadata {
            custom: Some(
                (0..65)
                    .map(|index| (format!("P{index}"), "value".to_string()))
                    .collect(),
            ),
            ..DocumentMetadata::default()
        })
        .unwrap_err();
        assert!(custom.message().contains("maximum of 64 properties"));
    }
}
