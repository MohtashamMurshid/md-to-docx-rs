//! Map options JSON onto the library structs.
//!
//! A `plugins` key is rejected before any other field is read. JavaScript
//! plugins and function callbacks stay outside this crate. The binding does
//! not embed a JavaScript engine.

use std::collections::BTreeMap;

use md_to_docx::options::{
    AccessibilityOptions, Alignment, CaptionFailureMode, CaptionOptions, CaptionPlacement,
    ChartInvalidBehavior, ChartRenderingOptions, CodeHighlightTheme, CodeHighlightingOptions,
    ConvertOptions, Direction, DocumentMetadata, DocumentSection, DuplicateStyleNameBehavior,
    HeaderFooterContent, HeaderFooterGroup, HeaderFooterSlot, MarginInput, MathRenderingOptions,
    MathUnsupported, MermaidFailureMode, MermaidRenderingOptions, MissingImageAltText,
    MissingStyleBehavior, OptionBool, Orientation, PageInput, PageNumberDisplay,
    PageNumberFormat, PageNumberSeparator, PageNumberingInput, PatchMarkdownOptions,
    PlaceholderDelimiters, ReferenceDocxModeOptions, ReferenceDocxPackageLimits,
    ReferenceStyleSelector, SectionBreak, SectionConfig, SizeInput, StyleOverrides,
    TextFind, TextReplace, TextReplacement, TextReplacementMode, TocMode, TocOverrides,
    UpdateFieldsOnOpen,
};
use serde_json::Value;

const PLUGINS_ERROR: &str =
    "Plugins are trusted Rust traits and cannot be passed from JavaScript";

#[derive(Debug)]
pub struct LoadedOptions {
    pub convert: ConvertOptions,
    pub reference: Option<ReferenceDocxModeOptions>,
    pub patch: PatchMarkdownOptions,
    pub image_base_directory: Option<String>,
    /// `signal.aborted === true`. An absent signal is not aborted.
    pub aborted: bool,
}

pub fn load_options_json(text: &str, path: &str) -> Result<LoadedOptions, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|error| {
        format!("Invalid JSON in options file \"{path}\": {error}")
    })?;
    let Some(object) = parsed.as_object() else {
        return Err("Options JSON must be an object".to_string());
    };
    if object.contains_key("plugins") {
        return Err(PLUGINS_ERROR.to_string());
    }
    if object.contains_key("onWarning") {
        return Err("onWarning must be a function".to_string());
    }
    let mut loaded = LoadedOptions {
        convert: ConvertOptions::default(),
        reference: None,
        patch: PatchMarkdownOptions::default(),
        image_base_directory: None,
        aborted: false,
    };
    if let Some(signal) = object.get("signal") {
        loaded.aborted = signal
            .as_object()
            .and_then(|signal| signal.get("aborted"))
            .and_then(|aborted| aborted.as_bool())
            == Some(true);
    }
    if let Some(style) = object.get("style") {
        loaded.convert.style = Some(parse_style(style, "options.style")?);
    }
    if let Some(template) = object.get("template") {
        loaded.convert.template = Some(parse_section_config(template, "options.template")?);
    }
    if let Some(sections) = object.get("sections") {
        loaded.convert.sections = Some(parse_sections(sections)?);
    }
    if let Some(toc) = object.get("toc") {
        loaded.convert.toc = Some(parse_toc(toc)?);
    }
    if let Some(captions) = object.get("captions") {
        loaded.convert.captions = Some(parse_captions(captions)?);
    }
    if let Some(accessibility) = object.get("accessibility") {
        loaded.convert.accessibility = Some(parse_accessibility(accessibility)?);
    }
    if let Some(math) = object.get("mathRendering") {
        loaded.convert.math_rendering = Some(parse_math(math)?);
    }
    if let Some(charts) = object.get("chartRendering") {
        loaded.convert.chart_rendering = Some(parse_charts(charts)?);
    }
    if let Some(mermaid) = object.get("mermaidRendering") {
        loaded.convert.mermaid_rendering = Some(parse_mermaid(mermaid)?);
    }
    if let Some(highlighting) = object.get("codeHighlighting") {
        loaded.convert.code_highlighting = Some(parse_highlighting(highlighting)?);
    }
    if let Some(replacements) = object.get("textReplacements") {
        loaded.convert.text_replacements = Some(parse_replacements(replacements)?);
    }
    if let Some(mode) = object.get("textReplacementMode") {
        loaded.convert.text_replacement_mode = Some(parse_replacement_mode(mode));
    }
    if let Some(limit) = object.get("maxInputLength") {
        let limit = json_u32(limit, "maxInputLength")?;
        loaded.convert.max_input_length = Some(limit);
        loaded.patch.max_input_length = Some(limit);
    }
    if let Some(limit) = object.get("maxElements") {
        let limit = json_u32(limit, "maxElements")?;
        loaded.convert.max_elements = Some(limit);
        loaded.patch.max_elements = Some(limit);
    }
    if let Some(metadata) = object.get("metadata") {
        let metadata = parse_metadata(metadata)?;
        loaded.convert.metadata = Some(metadata.clone());
        loaded.patch.metadata = Some(metadata);
    }
    if let Some(reference) = object.get("reference") {
        loaded.reference = Some(parse_reference(reference)?);
    }
    if let Some(keep) = object.get("keepOriginalStyles") {
        loaded.patch.keep_original_styles = Some(json_bool(keep, "keepOriginalStyles")?);
    }
    if let Some(delimiters) = object.get("placeholderDelimiters") {
        loaded.patch.placeholder_delimiters = Some(parse_delimiters(delimiters)?);
    }
    if let Some(recursive) = object.get("recursive") {
        loaded.patch.recursive = Some(json_bool(recursive, "recursive")?);
    }
    if let Some(width) = object.get("tableWidthTwips") {
        loaded.patch.table_width_twips = Some(json_u32(width, "tableWidthTwips")?);
    }
    if let Some(images) = object.get("imageHandling") {
        loaded.image_base_directory = parse_image_handling(images)?;
    }
    Ok(loaded)
}

pub fn load_patches_json(text: &str) -> Result<BTreeMap<String, String>, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let Some(object) = parsed.as_object() else {
        return Err("Patches JSON must be an object".to_string());
    };
    let mut patches = BTreeMap::new();
    for (name, value) in object {
        let markdown = match value {
            Value::String(markdown) => markdown.clone(),
            Value::Object(body) => match body.get("markdown") {
                Some(Value::String(markdown)) => markdown.clone(),
                _ => {
                    return Err(format!(
                        "Patch \"{name}\" must be a string or an object with markdown"
                    ))
                }
            },
            _ => {
                return Err(format!(
                    "Patch \"{name}\" must be a string or an object with markdown"
                ))
            }
        };
        patches.insert(name.clone(), markdown);
    }
    Ok(patches)
}

fn parse_sections(value: &Value) -> Result<Vec<DocumentSection>, String> {
    let Some(sections) = value.as_array() else {
        return Err("sections must be an array".to_string());
    };
    let mut parsed = Vec::with_capacity(sections.len());
    for (index, section) in sections.iter().enumerate() {
        let context = format!("options.sections[{index}]");
        let Some(object) = section.as_object() else {
            return Err(format!("{context} must be an object"));
        };
        let markdown = match object.get("markdown") {
            Some(Value::String(markdown)) => markdown.clone(),
            _ => return Err(format!("{context}.markdown must be a string")),
        };
        parsed.push(DocumentSection {
            markdown,
            config: parse_section_config(section, &context)?,
        });
    }
    Ok(parsed)
}

fn parse_section_config(value: &Value, context: &str) -> Result<SectionConfig, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{context} must be an object"));
    };
    let mut config = SectionConfig::default();
    if let Some(style) = object.get("style") {
        config.style = Some(parse_style(style, &format!("{context}.style"))?);
    }
    if let Some(page) = object.get("page") {
        config.page = Some(parse_page(page, &format!("{context}.page"))?);
    }
    if let Some(headers) = object.get("headers") {
        config.headers = Some(parse_header_footer_group(headers, &format!("{context}.headers"))?);
    }
    if let Some(footers) = object.get("footers") {
        config.footers = Some(parse_header_footer_group(footers, &format!("{context}.footers"))?);
    }
    if let Some(numbering) = object.get("pageNumbering") {
        config.page_numbering = Some(parse_page_numbering(numbering, context)?);
    }
    if let Some(title_page) = object.get("titlePage") {
        config.title_page = Some(json_bool(title_page, &format!("{context}.titlePage"))?);
    }
    if let Some(kind) = object.get("type") {
        config.break_type = Some(parse_section_break(kind)?);
    }
    Ok(config)
}

fn parse_style(value: &Value, context: &str) -> Result<StyleOverrides, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{context} must be an object"));
    };
    let mut style = StyleOverrides::default();
    if let Some(size) = object.get("titleSize") {
        style.title_size = Some(json_u32(size, "titleSize")?);
    }
    if let Some(spacing) = object.get("headingSpacing") {
        style.heading_spacing = Some(json_u32(spacing, "headingSpacing")?);
    }
    if let Some(spacing) = object.get("paragraphSpacing") {
        style.paragraph_spacing = Some(json_u32(spacing, "paragraphSpacing")?);
    }
    if let Some(spacing) = object.get("lineSpacing") {
        style.line_spacing = Some(json_f64(spacing, "lineSpacing")?);
    }
    if let Some(alignment) = object.get("paragraphAlignment") {
        style.paragraph_alignment = Some(parse_alignment(alignment, "paragraphAlignment")?);
    }
    if let Some(direction) = object.get("direction") {
        style.direction = Some(parse_direction(direction)?);
    }
    if let Some(font) = object.get("fontFamily") {
        style.font_family = Some(json_string(font, "fontFamily")?);
    }
    if let Some(font) = object.get("fontFamilly") {
        style.font_familly = Some(json_string(font, "fontFamilly")?);
    }
    if let Some(language) = object.get("language") {
        style.language = Some(json_string(language, &format!("{context}.language"))?);
    }
    if let Some(alignment) = object.get("headingAlignment") {
        style.heading_alignment = Some(parse_alignment(alignment, "headingAlignment")?);
    }
    for level in 1..=6 {
        let index = level - 1;
        if let Some(size) = object.get(&format!("heading{level}Size")) {
            style.heading_size[index] = Some(json_u32(size, &format!("heading{level}Size"))?);
        }
        if let Some(alignment) = object.get(&format!("heading{level}Alignment")) {
            style.heading_alignments[index] =
                Some(parse_alignment(alignment, &format!("heading{level}Alignment"))?);
        }
        if let Some(size) = object.get(&format!("tocHeading{level}FontSize")) {
            style.toc_heading_font_size[index] =
                Some(json_u32(size, &format!("tocHeading{level}FontSize"))?);
        }
        if let Some(bold) = object.get(&format!("tocHeading{level}Bold")) {
            style.toc_heading_bold[index] =
                Some(json_bool(bold, &format!("tocHeading{level}Bold"))?);
        }
        if let Some(italic) = object.get(&format!("tocHeading{level}Italic")) {
            style.toc_heading_italic[index] =
                Some(json_bool(italic, &format!("tocHeading{level}Italic"))?);
        }
    }
    if let Some(size) = object.get("paragraphSize") {
        style.paragraph_size = Some(json_u32(size, "paragraphSize")?);
    }
    if let Some(size) = object.get("listItemSize") {
        style.list_item_size = Some(json_u32(size, "listItemSize")?);
    }
    if let Some(size) = object.get("codeBlockSize") {
        style.code_block_size = Some(json_u32(size, "codeBlockSize")?);
    }
    if let Some(size) = object.get("blockquoteSize") {
        style.blockquote_size = Some(json_u32(size, "blockquoteSize")?);
    }
    if let Some(size) = object.get("tocFontSize") {
        style.toc_font_size = Some(json_u32(size, "tocFontSize")?);
    }
    Ok(style)
}

fn parse_page(value: &Value, context: &str) -> Result<PageInput, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{context} must be an object"));
    };
    let mut page = PageInput::default();
    if let Some(margin) = object.get("margin") {
        page.margin = Some(parse_margin(margin)?);
    }
    if let Some(size) = object.get("size") {
        page.size = Some(parse_size(size)?);
    }
    Ok(page)
}

fn parse_margin(value: &Value) -> Result<MarginInput, String> {
    let Some(object) = value.as_object() else {
        return Err("page.margin must be an object".to_string());
    };
    let mut margin = MarginInput::default();
    if let Some(top) = object.get("top") {
        margin.top = Some(json_u32(top, "margin.top")?);
    }
    if let Some(right) = object.get("right") {
        margin.right = Some(json_u32(right, "margin.right")?);
    }
    if let Some(bottom) = object.get("bottom") {
        margin.bottom = Some(json_u32(bottom, "margin.bottom")?);
    }
    if let Some(left) = object.get("left") {
        margin.left = Some(json_u32(left, "margin.left")?);
    }
    if let Some(header) = object.get("header") {
        margin.header = Some(json_u32(header, "margin.header")?);
    }
    if let Some(footer) = object.get("footer") {
        margin.footer = Some(json_u32(footer, "margin.footer")?);
    }
    if let Some(gutter) = object.get("gutter") {
        margin.gutter = Some(json_u32(gutter, "margin.gutter")?);
    }
    Ok(margin)
}

fn parse_size(value: &Value) -> Result<SizeInput, String> {
    let Some(object) = value.as_object() else {
        return Err("page.size must be an object".to_string());
    };
    let mut size = SizeInput::default();
    if let Some(width) = object.get("width") {
        size.width = Some(json_u32(width, "size.width")?);
    }
    if let Some(height) = object.get("height") {
        size.height = Some(json_u32(height, "size.height")?);
    }
    if let Some(orientation) = object.get("orientation") {
        size.orientation = Some(match orientation.as_str() {
            Some("PORTRAIT") => Orientation::Portrait,
            Some("LANDSCAPE") => Orientation::Landscape,
            _ => return Err("Invalid page orientation: Must be PORTRAIT or LANDSCAPE".to_string()),
        });
    }
    Ok(size)
}

fn parse_page_numbering(value: &Value, context: &str) -> Result<PageNumberingInput, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{context}.pageNumbering must be an object"));
    };
    let mut numbering = PageNumberingInput::default();
    if let Some(start) = object.get("start") {
        numbering.start = Some(json_u32(start, "pageNumbering.start")?);
    }
    if let Some(format) = object.get("formatType") {
        numbering.format = Some(parse_page_format(format)?);
    }
    if let Some(separator) = object.get("separator") {
        numbering.separator = Some(parse_page_separator(separator)?);
    }
    if let Some(display) = object.get("display") {
        numbering.display = Some(parse_page_display(display)?);
    }
    if let Some(alignment) = object.get("alignment") {
        numbering.alignment = Some(parse_alignment(alignment, "page number alignment")?);
    }
    Ok(numbering)
}

fn parse_header_footer_group(value: &Value, context: &str) -> Result<HeaderFooterGroup, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{context} must be an object"));
    };
    let mut group = HeaderFooterGroup::default();
    if let Some(slot) = object.get("default") {
        group.default = Some(parse_header_footer_slot(slot, &format!("{context}.default"))?);
    }
    if let Some(slot) = object.get("first") {
        group.first = Some(parse_header_footer_slot(slot, &format!("{context}.first"))?);
    }
    if let Some(slot) = object.get("even") {
        group.even = Some(parse_header_footer_slot(slot, &format!("{context}.even"))?);
    }
    Ok(group)
}

fn parse_header_footer_slot(value: &Value, context: &str) -> Result<HeaderFooterSlot, String> {
    if value.is_null() {
        return Ok(HeaderFooterSlot::Remove);
    }
    let Some(object) = value.as_object() else {
        return Err(format!("{context} must be an object or null"));
    };
    let mut content = HeaderFooterContent::default();
    if let Some(markdown) = object.get("markdown") {
        content.markdown = Some(json_string(markdown, &format!("{context}.markdown"))?);
    }
    if let Some(text) = object.get("text") {
        content.text = Some(json_string(text, &format!("{context}.text"))?);
    }
    if let Some(alignment) = object.get("alignment") {
        content.alignment = Some(parse_alignment(alignment, "header/footer alignment")?);
    }
    if let Some(display) = object.get("pageNumberDisplay") {
        content.page_number_display = Some(parse_page_display(display)?);
    }
    Ok(HeaderFooterSlot::Content(content))
}

fn parse_toc(value: &Value) -> Result<TocOverrides, String> {
    let Some(object) = value.as_object() else {
        return Err("toc must be an object".to_string());
    };
    let mut toc = TocOverrides::default();
    if let Some(mode) = object.get("mode") {
        toc.mode = Some(match mode.as_str() {
            Some("native") => TocMode::Native,
            Some("links") => TocMode::Links,
            _ => return Err("Invalid toc.mode: Must be native or links".to_string()),
        });
    }
    if let Some(page_numbers) = object.get("pageNumbers") {
        toc.page_numbers = Some(json_bool(page_numbers, "toc.pageNumbers")?);
    }
    if let Some(leaders) = object.get("dotLeaders") {
        toc.dot_leaders = Some(json_bool(leaders, "toc.dotLeaders")?);
    }
    if let Some(update) = object.get("updateFieldsOnOpen") {
        toc.update_fields_on_open = Some(json_bool(update, "toc.updateFieldsOnOpen")?);
    }
    if let Some(title) = object.get("title") {
        toc.title = Some(json_string(title, "toc.title")?);
    }
    if let Some(depth) = object.get("minDepth") {
        toc.min_depth = Some(json_u8(depth, "toc.minDepth")?);
    }
    if let Some(depth) = object.get("maxDepth") {
        toc.max_depth = Some(json_u8(depth, "toc.maxDepth")?);
    }
    Ok(toc)
}

fn parse_captions(value: &Value) -> Result<CaptionOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("captions must be an object".to_string());
    };
    let mut captions = CaptionOptions::default();
    if let Some(label) = object.get("figureLabel") {
        captions.figure_label = Some(json_string(label, "captions.figureLabel")?);
    }
    if let Some(label) = object.get("tableLabel") {
        captions.table_label = Some(json_string(label, "captions.tableLabel")?);
    }
    if let Some(placement) = object.get("figurePlacement") {
        captions.figure_placement = Some(parse_caption_placement(placement, "figurePlacement")?);
    }
    if let Some(placement) = object.get("tablePlacement") {
        captions.table_placement = Some(parse_caption_placement(placement, "tablePlacement")?);
    }
    if let Some(alignment) = object.get("alignment") {
        captions.alignment = Some(parse_alignment(alignment, "captions.alignment")?);
    }
    if let Some(italic) = object.get("italic") {
        captions.italic = Some(json_bool(italic, "captions.italic")?);
    }
    if let Some(size) = object.get("size") {
        captions.size = Some(json_u32(size, "captions.size")?);
    }
    if let Some(mode) = object.get("failureMode") {
        captions.failure_mode = Some(match mode.as_str() {
            Some("preserve") => CaptionFailureMode::Preserve,
            Some("throw") => CaptionFailureMode::Throw,
            _ => {
                return Err(
                    "Invalid captions.failureMode: Must be preserve or throw".to_string()
                )
            }
        });
    }
    if let Some(update) = object.get("updateFieldsOnOpen") {
        captions.update_fields_on_open = Some(match update.as_bool() {
            Some(value) => UpdateFieldsOnOpen::Bool(value),
            None => UpdateFieldsOnOpen::Invalid,
        });
    }
    Ok(captions)
}

fn parse_accessibility(value: &Value) -> Result<AccessibilityOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("accessibility must be an object".to_string());
    };
    let mut accessibility = AccessibilityOptions::default();
    if let Some(policy) = object.get("missingImageAltText") {
        accessibility.missing_image_alt_text = Some(match policy.as_str() {
            Some("permissive") => MissingImageAltText::Permissive,
            Some("throw") => MissingImageAltText::Throw,
            _ => MissingImageAltText::Invalid,
        });
    }
    Ok(accessibility)
}

fn parse_math(value: &Value) -> Result<MathRenderingOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("mathRendering must be an object".to_string());
    };
    let mut math = MathRenderingOptions::default();
    if let Some(enabled) = object.get("enabled") {
        math.enabled = Some(parse_option_bool(enabled));
    }
    if let Some(unsupported) = object.get("unsupported") {
        math.unsupported = Some(match unsupported.as_str() {
            Some("text") => MathUnsupported::Text,
            Some("throw") => MathUnsupported::Throw,
            _ => MathUnsupported::Invalid,
        });
    }
    Ok(math)
}

fn parse_charts(value: &Value) -> Result<ChartRenderingOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("chartRendering must be an object".to_string());
    };
    if object.contains_key("renderer") {
        return Err(
            "chartRendering.renderer is a function and cannot be passed from JavaScript"
                .to_string(),
        );
    }
    let mut charts = ChartRenderingOptions::default();
    if let Some(enabled) = object.get("enabled") {
        charts.enabled = Some(parse_option_bool(enabled));
    }
    if let Some(width) = object.get("width") {
        charts.width = Some(json_u32(width, "chartRendering.width")?);
    }
    if let Some(height) = object.get("height") {
        charts.height = Some(json_u32(height, "chartRendering.height")?);
    }
    if let Some(width) = object.get("maxWidth") {
        charts.max_width = Some(json_u32(width, "chartRendering.maxWidth")?);
    }
    if let Some(height) = object.get("maxHeight") {
        charts.max_height = Some(json_u32(height, "chartRendering.maxHeight")?);
    }
    if let Some(behavior) = object.get("invalidDefinitionBehavior") {
        charts.invalid_definition_behavior = Some(match behavior.as_str() {
            Some("placeholder") => ChartInvalidBehavior::Placeholder,
            Some("throw") => ChartInvalidBehavior::Throw,
            _ => ChartInvalidBehavior::Invalid,
        });
    }
    Ok(charts)
}

fn parse_mermaid(value: &Value) -> Result<MermaidRenderingOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("mermaidRendering must be an object".to_string());
    };
    if object.contains_key("render") {
        return Err(
            "mermaidRendering.render is a function and cannot be passed from JavaScript"
                .to_string(),
        );
    }
    let mut mermaid = MermaidRenderingOptions::default();
    if let Some(enabled) = object.get("enabled") {
        mermaid.enabled = Some(parse_option_bool(enabled));
    }
    if let Some(mode) = object.get("failureMode") {
        mermaid.failure_mode = Some(match mode.as_str() {
            Some("codeBlock") => MermaidFailureMode::CodeBlock,
            Some("placeholder") => MermaidFailureMode::Placeholder,
            Some("throw") => MermaidFailureMode::Throw,
            _ => MermaidFailureMode::Invalid,
        });
    }
    Ok(mermaid)
}

fn parse_highlighting(value: &Value) -> Result<CodeHighlightingOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("codeHighlighting must be an object".to_string());
    };
    let mut highlighting = CodeHighlightingOptions::default();
    if let Some(enabled) = object.get("enabled") {
        highlighting.enabled = Some(parse_option_bool(enabled));
    }
    if let Some(theme) = object.get("theme") {
        highlighting.theme = Some(parse_theme(theme)?);
    }
    if let Some(languages) = object.get("languages") {
        highlighting.languages = Some(json_string_array(languages, "codeHighlighting.languages")?);
    }
    if let Some(show) = object.get("showLanguageLabel") {
        highlighting.show_language_label = Some(parse_option_bool(show));
    }
    Ok(highlighting)
}

fn parse_theme(value: &Value) -> Result<CodeHighlightTheme, String> {
    let Some(object) = value.as_object() else {
        return Err("codeHighlighting.theme must be an object".to_string());
    };
    let mut theme = CodeHighlightTheme::default();
    if let Some(color) = object.get("default") {
        theme.default = Some(json_string(color, "theme.default")?);
    }
    if let Some(color) = object.get("background") {
        theme.background = Some(json_string(color, "theme.background")?);
    }
    if let Some(color) = object.get("border") {
        theme.border = Some(json_string(color, "theme.border")?);
    }
    if let Some(color) = object.get("languageLabel") {
        theme.language_label = Some(json_string(color, "theme.languageLabel")?);
    }
    Ok(theme)
}

fn parse_replacements(value: &Value) -> Result<Vec<TextReplacement>, String> {
    let Some(items) = value.as_array() else {
        return Err("textReplacements must be an array".to_string());
    };
    let mut replacements = Vec::with_capacity(items.len());
    for item in items {
        let Some(object) = item.as_object() else {
            return Err("textReplacements entries must be objects".to_string());
        };
        let find = match object.get("find") {
            Some(Value::String(pattern)) => TextFind::Literal(pattern.clone()),
            Some(_) => {
                return Err(
                    "Function or RegExp textReplacements cannot be passed from JavaScript"
                        .to_string(),
                )
            }
            None => return Err("textReplacements.find is required".to_string()),
        };
        let replace = match object.get("replace") {
            Some(Value::String(text)) => TextReplace::Text(text.clone()),
            Some(_) => {
                return Err(
                    "Function textReplacements cannot be passed from JavaScript".to_string(),
                )
            }
            None => return Err("textReplacements.replace is required".to_string()),
        };
        replacements.push(TextReplacement { find, replace });
    }
    Ok(replacements)
}

fn parse_replacement_mode(value: &Value) -> TextReplacementMode {
    match value.as_str() {
        Some("trusted") => TextReplacementMode::Trusted,
        Some("untrusted") => TextReplacementMode::Untrusted,
        _ => TextReplacementMode::Invalid,
    }
}

fn parse_metadata(value: &Value) -> Result<DocumentMetadata, String> {
    let Some(object) = value.as_object() else {
        return Err("metadata must be an object".to_string());
    };
    let mut metadata = DocumentMetadata::default();
    if let Some(title) = object.get("title") {
        metadata.title = Some(json_string(title, "metadata.title")?);
    }
    if let Some(subject) = object.get("subject") {
        metadata.subject = Some(json_string(subject, "metadata.subject")?);
    }
    if let Some(description) = object.get("description") {
        metadata.description = Some(json_string(description, "metadata.description")?);
    }
    if let Some(creator) = object.get("creator") {
        metadata.creator = Some(json_string(creator, "metadata.creator")?);
    }
    if let Some(author) = object.get("author") {
        metadata.author = Some(json_string(author, "metadata.author")?);
    }
    if let Some(keywords) = object.get("keywords") {
        metadata.keywords = Some(parse_keywords(keywords)?);
    }
    if let Some(category) = object.get("category") {
        metadata.category = Some(json_string(category, "metadata.category")?);
    }
    if let Some(company) = object.get("company") {
        metadata.company = Some(json_string(company, "metadata.company")?);
    }
    if let Some(language) = object.get("language") {
        metadata.language = Some(json_string(language, "metadata.language")?);
    }
    if let Some(created) = object.get("created") {
        metadata.created = Some(json_string(created, "metadata.created")?);
    }
    if let Some(modified) = object.get("modified") {
        metadata.modified = Some(json_string(modified, "metadata.modified")?);
    }
    if let Some(custom) = object.get("custom") {
        metadata.custom = Some(parse_string_map(custom, "metadata.custom")?);
    }
    Ok(metadata)
}

fn parse_keywords(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(keywords) => Ok(vec![keywords.clone()]),
        Value::Array(_) => json_string_array(value, "metadata.keywords"),
        _ => Err("metadata.keywords must be a string or an array of strings".to_string()),
    }
}

fn parse_reference(value: &Value) -> Result<ReferenceDocxModeOptions, String> {
    let Some(object) = value.as_object() else {
        return Err("reference must be an object".to_string());
    };
    let mut reference = ReferenceDocxModeOptions::default();
    if let Some(styles) = object.get("styles") {
        reference.styles = Some(parse_style_selectors(styles)?);
    }
    if let Some(behavior) = object.get("missingStyleBehavior") {
        reference.missing_style_behavior = Some(match behavior.as_str() {
            Some("fallback") => MissingStyleBehavior::Fallback,
            Some("throw") => MissingStyleBehavior::Throw,
            _ => {
                return Err(
                    "Invalid reference.missingStyleBehavior: Must be fallback or throw".to_string(),
                )
            }
        });
    }
    if let Some(behavior) = object.get("duplicateStyleNameBehavior") {
        reference.duplicate_style_name_behavior = Some(match behavior.as_str() {
            Some("throw") => DuplicateStyleNameBehavior::Throw,
            Some("first") => DuplicateStyleNameBehavior::First,
            _ => {
                return Err(
                    "Invalid reference.duplicateStyleNameBehavior: Must be first or throw"
                        .to_string(),
                )
            }
        });
    }
    if let Some(preserve) = object.get("preservePageLayout") {
        reference.preserve_page_layout = Some(json_bool(preserve, "reference.preservePageLayout")?);
    }
    if let Some(preserve) = object.get("preserveHeadersAndFooters") {
        reference.preserve_headers_and_footers =
            Some(json_bool(preserve, "reference.preserveHeadersAndFooters")?);
    }
    if let Some(limits) = object.get("limits") {
        reference.limits = Some(parse_limits(limits)?);
    }
    Ok(reference)
}

fn parse_style_selectors(
    value: &Value,
) -> Result<BTreeMap<String, ReferenceStyleSelector>, String> {
    let Some(object) = value.as_object() else {
        return Err("reference.styles must be an object".to_string());
    };
    let mut styles = BTreeMap::new();
    for (role, selector) in object {
        if selector.is_null() {
            continue;
        }
        let Some(body) = selector.as_object() else {
            return Err(format!("reference.styles.{role} must be an object"));
        };
        let id = body.get("id").filter(|value| !value.is_null());
        let name = body.get("name").filter(|value| !value.is_null());
        let selector = match (id, name) {
            (Some(id), None) => ReferenceStyleSelector::Id(json_string(id, "id")?),
            (None, Some(name)) => ReferenceStyleSelector::Name(json_string(name, "name")?),
            _ => {
                return Err(format!(
                    "reference.styles.{role} must set either id or name"
                ))
            }
        };
        styles.insert(role.clone(), selector);
    }
    Ok(styles)
}

fn parse_limits(value: &Value) -> Result<ReferenceDocxPackageLimits, String> {
    let Some(object) = value.as_object() else {
        return Err("reference.limits must be an object".to_string());
    };
    let mut limits = ReferenceDocxPackageLimits::default();
    if let Some(limit) = object.get("maxCompressedBytes") {
        limits.max_compressed_bytes = Some(json_u64(limit, "reference.limits.maxCompressedBytes")?);
    }
    if let Some(limit) = object.get("maxUncompressedBytes") {
        limits.max_uncompressed_bytes =
            Some(json_u64(limit, "reference.limits.maxUncompressedBytes")?);
    }
    if let Some(limit) = object.get("maxEntryUncompressedBytes") {
        limits.max_entry_uncompressed_bytes = Some(json_u64(
            limit,
            "reference.limits.maxEntryUncompressedBytes",
        )?);
    }
    if let Some(limit) = object.get("maxEntries") {
        limits.max_entries = Some(json_u64(limit, "reference.limits.maxEntries")?);
    }
    Ok(limits)
}

fn parse_delimiters(value: &Value) -> Result<PlaceholderDelimiters, String> {
    let Some(object) = value.as_object() else {
        return Err("placeholderDelimiters must be an object".to_string());
    };
    let start = match object.get("start") {
        Some(start) => json_string(start, "placeholderDelimiters.start")?,
        None => return Err("placeholderDelimiters.start is required".to_string()),
    };
    let end = match object.get("end") {
        Some(end) => json_string(end, "placeholderDelimiters.end")?,
        None => return Err("placeholderDelimiters.end is required".to_string()),
    };
    Ok(PlaceholderDelimiters { start, end })
}

fn parse_image_handling(value: &Value) -> Result<Option<String>, String> {
    let Some(object) = value.as_object() else {
        return Err("imageHandling must be an object".to_string());
    };
    if object.contains_key("resolve") {
        return Err("imageHandling.resolve must be a function".to_string());
    }
    match object.get("baseDirectory") {
        Some(directory) => Ok(Some(json_string(directory, "imageHandling.baseDirectory")?)),
        None => Ok(None),
    }
}

fn parse_alignment(value: &Value, name: &str) -> Result<Alignment, String> {
    match value.as_str() {
        Some("LEFT") => Ok(Alignment::Left),
        Some("CENTER") => Ok(Alignment::Center),
        Some("RIGHT") => Ok(Alignment::Right),
        Some("JUSTIFIED") => Ok(Alignment::Justified),
        _ => Err(format!(
            "Invalid {name}: Must be LEFT, CENTER, RIGHT, or JUSTIFIED"
        )),
    }
}

fn parse_direction(value: &Value) -> Result<Direction, String> {
    match value.as_str() {
        Some("LTR") => Ok(Direction::Ltr),
        Some("RTL") => Ok(Direction::Rtl),
        _ => Err("Invalid direction: Must be LTR or RTL".to_string()),
    }
}

fn parse_section_break(value: &Value) -> Result<SectionBreak, String> {
    match value.as_str() {
        Some("NEXT_PAGE") => Ok(SectionBreak::NextPage),
        Some("NEXT_COLUMN") => Ok(SectionBreak::NextColumn),
        Some("CONTINUOUS") => Ok(SectionBreak::Continuous),
        Some("EVEN_PAGE") => Ok(SectionBreak::EvenPage),
        Some("ODD_PAGE") => Ok(SectionBreak::OddPage),
        _ => Err(
            "Invalid section type: Must be NEXT_PAGE, NEXT_COLUMN, CONTINUOUS, EVEN_PAGE, or ODD_PAGE"
                .to_string(),
        ),
    }
}

fn parse_page_format(value: &Value) -> Result<PageNumberFormat, String> {
    match value.as_str() {
        Some("decimal") => Ok(PageNumberFormat::Decimal),
        Some("upperRoman") => Ok(PageNumberFormat::UpperRoman),
        Some("lowerRoman") => Ok(PageNumberFormat::LowerRoman),
        Some("upperLetter") => Ok(PageNumberFormat::UpperLetter),
        Some("lowerLetter") => Ok(PageNumberFormat::LowerLetter),
        _ => Err(
            "Invalid page number formatType: Must be one of decimal, upperRoman, lowerRoman, upperLetter, lowerLetter"
                .to_string(),
        ),
    }
}

fn parse_page_separator(value: &Value) -> Result<PageNumberSeparator, String> {
    match value.as_str() {
        Some("hyphen") => Ok(PageNumberSeparator::Hyphen),
        Some("period") => Ok(PageNumberSeparator::Period),
        Some("colon") => Ok(PageNumberSeparator::Colon),
        Some("emDash") => Ok(PageNumberSeparator::EmDash),
        Some("endash") => Ok(PageNumberSeparator::EnDash),
        _ => Err(
            "Invalid page number separator: Must be hyphen, period, colon, emDash, or endash"
                .to_string(),
        ),
    }
}

fn parse_page_display(value: &Value) -> Result<PageNumberDisplay, String> {
    match value.as_str() {
        Some("none") => Ok(PageNumberDisplay::None),
        Some("current") => Ok(PageNumberDisplay::Current),
        Some("currentAndTotal") => Ok(PageNumberDisplay::CurrentAndTotal),
        Some("currentAndSectionTotal") => Ok(PageNumberDisplay::CurrentAndSectionTotal),
        _ => Err(
            "Invalid page number display: Must be none, current, currentAndTotal, or currentAndSectionTotal"
                .to_string(),
        ),
    }
}

fn parse_caption_placement(value: &Value, name: &str) -> Result<CaptionPlacement, String> {
    match value.as_str() {
        Some("above") => Ok(CaptionPlacement::Above),
        Some("below") => Ok(CaptionPlacement::Below),
        _ => Err(format!("Invalid captions.{name}: Must be above or below")),
    }
}

fn parse_option_bool(value: &Value) -> OptionBool {
    match value.as_bool() {
        Some(value) => OptionBool::Bool(value),
        None => OptionBool::Invalid,
    }
}

fn parse_string_map(value: &Value, name: &str) -> Result<BTreeMap<String, String>, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{name} must be an object"));
    };
    let mut map = BTreeMap::new();
    for (key, item) in object {
        map.insert(key.clone(), json_string(item, &format!("{name}.{key}"))?);
    }
    Ok(map)
}

fn json_string_array(value: &Value, name: &str) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!("{name} must be an array"));
    };
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        strings.push(json_string(item, name)?);
    }
    Ok(strings)
}

fn json_string(value: &Value, name: &str) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn json_bool(value: &Value, name: &str) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("{name} must be a boolean"))
}

fn json_u32(value: &Value, name: &str) -> Result<u32, String> {
    let number = json_u64(value, name)?;
    u32::try_from(number).map_err(|_| format!("{name} must be a positive integer"))
}

fn json_u64(value: &Value, name: &str) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a positive integer"))
}

fn json_u8(value: &Value, name: &str) -> Result<u8, String> {
    let number = json_u64(value, name)?;
    u8::try_from(number).map_err(|_| format!("{name} must be a positive integer"))
}

fn json_f64(value: &Value, name: &str) -> Result<f64, String> {
    value
        .as_f64()
        .ok_or_else(|| format!("{name} must be a number"))
}

#[cfg(test)]
mod tests {
    use super::load_options_json;

    #[test]
    fn plugins_key_cannot_be_loaded_from_cli_json() {
        let error = load_options_json(r#"{\"plugins\":[]}"#, "options.json").unwrap_err();
        assert!(
            error.contains("cannot be passed from JavaScript"),
            "{error}"
        );
        let null_plugin = load_options_json(r#"{\"plugins\":null}"#, "options.json").unwrap_err();
        assert!(null_plugin.contains("cannot be passed from JavaScript"));
    }

    #[test]
    fn rejects_non_objects_and_invalid_json() {
        let invalid = load_options_json("not valid json", "options.json").unwrap_err();
        assert!(invalid.contains("Invalid JSON"), "{invalid}");
        let array = load_options_json(r#"[\"not\", \"an\", \"object\"]"#, "options.json").unwrap_err();
        assert!(array.contains("Options JSON must be an object"), "{array}");
    }
}
