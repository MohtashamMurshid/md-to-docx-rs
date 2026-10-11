//! Slice 6 parity with the math and chart cases in `tests/rendering.test.ts`
//! and the Mermaid cases in `tests/diagram-rendering.test.ts`.
//!
//! ZIP bytes are not compared. Supported TeX becomes OMML. Chart PNGs are
//! judged by pixel size, not by a match with the TypeScript encoder.

#[path = "common/mod.rs"]
mod common;

use common::{Document, MathExpr};

use md_to_docx::options::{
    ChartInvalidBehavior, ChartRenderingOptions, MathRenderingOptions, MathUnsupported,
    MermaidFailureMode, MermaidRenderInput, MermaidRenderOutput, MermaidRenderer,
    MermaidRenderingOptions, OptionBool,
};
use md_to_docx::ConvertOptions;

const EMU_PER_PX: u64 = 9525;

const BAR_CHART: &str = r#"{"type":"bar","data":{"labels":["Q1","Q2","Q3"],"datasets":[{"label":"Revenue","data":[12,18,9]}]},"width":160,"height":90,"alt":"Revenue chart"}"#;
const LINE_CHART: &str = r#"{"type":"line","data":{"labels":["A","B","C"],"datasets":[{"data":[1,3,2]}]},"width":321,"height":123}"#;
const PIE_CHART: &str = r##"{"type":"pie","data":{"labels":["Only"],"datasets":[{"data":[1],"backgroundColor":["#E15759"]}]},"width":96,"height":96}"##;
const DOUGHNUT_CHART: &str = r##"{"type":"doughnut","data":{"labels":["Only"],"datasets":[{"data":[1],"backgroundColor":["#E15759"]}]},"width":96,"height":96}"##;
const DOUGHNUT_FULL: &str = r##"{"type":"doughnut","data":{"labels":["All","None"],"datasets":[{"data":[5,0],"backgroundColor":["#E15759","#4E79A7"]}]},"width":96,"height":96}"##;
const DEFAULT_CHART: &str = r#"{"type":"bar","data":{"labels":["Q1"],"datasets":[{"data":[1]}]}}"#;

const ONE_PX_PAYLOAD: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";

const MERMAID: &str = "Before.

```mermaid
graph TD
  A[Start] --> B[Done]
```

After.";

fn convert(markdown: &str) -> Document {
    project(
        &md_to_docx::convert_markdown(markdown)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn convert_with(markdown: &str, options: &ConvertOptions) -> Document {
    project(
        &md_to_docx::convert_with_options(markdown, options)
            .unwrap_or_else(|err| panic!("convert failed: {err}")),
    )
}

fn project(bytes: &[u8]) -> Document {
    assert!(
        bytes.starts_with(b"PK"),
        "conversion should return a zip package"
    );
    common::project_docx(bytes)
}

fn math_off() -> ConvertOptions {
    ConvertOptions {
        math_rendering: Some(MathRenderingOptions {
            enabled: Some(OptionBool::Bool(false)),
            ..MathRenderingOptions::default()
        }),
        ..ConvertOptions::default()
    }
}

fn math_throw() -> ConvertOptions {
    ConvertOptions {
        math_rendering: Some(MathRenderingOptions {
            unsupported: Some(MathUnsupported::Throw),
            ..MathRenderingOptions::default()
        }),
        ..ConvertOptions::default()
    }
}

fn charts_on() -> ConvertOptions {
    ConvertOptions {
        chart_rendering: Some(ChartRenderingOptions {
            enabled: Some(OptionBool::Bool(true)),
            ..ChartRenderingOptions::default()
        }),
        ..ConvertOptions::default()
    }
}

fn charts_throw() -> ConvertOptions {
    ConvertOptions {
        chart_rendering: Some(ChartRenderingOptions {
            enabled: Some(OptionBool::Bool(true)),
            invalid_definition_behavior: Some(ChartInvalidBehavior::Throw),
            ..ChartRenderingOptions::default()
        }),
        ..ConvertOptions::default()
    }
}

fn mermaid_options(
    failure_mode: Option<MermaidFailureMode>,
    render: MermaidRenderer,
) -> ConvertOptions {
    ConvertOptions {
        mermaid_rendering: Some(MermaidRenderingOptions {
            enabled: Some(OptionBool::Bool(true)),
            render: Some(render),
            failure_mode,
        }),
        ..ConvertOptions::default()
    }
}

fn math_objects(doc: &Document) -> Vec<&MathExpr> {
    doc.paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.math.iter())
        .collect()
}

fn joined(doc: &Document) -> String {
    doc.paragraph_texts().join("\n")
}

fn assert_no_math(doc: &Document, source: &str) {
    assert!(
        math_objects(doc).is_empty(),
        "math leaked into {source:?}\n{}",
        doc.render()
    );
    assert!(
        joined(doc).contains(source),
        "missing literal {source:?}\n{}",
        doc.render()
    );
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "media is not a PNG, first bytes {:02x?}",
        &bytes[..bytes.len().min(8)]
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("png width"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("png height"));
    (width, height)
}

fn assert_chart_png(doc: &Document, width: u32, height: u32) {
    assert_eq!(
        doc.media.len(),
        1,
        "media {:?}\n{}",
        doc.media_parts,
        doc.render()
    );
    let media = &doc.media[0];
    assert!(media.path.ends_with(".png"), "path {}", media.path);
    assert_eq!(png_size(&media.bytes), (width, height));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
}

fn decode_base64(input: &str) -> Vec<u8> {
    fn value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("bad base64 byte {byte}"),
        }
    }
    let mut packed: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    while packed.len() % 4 != 0 {
        packed.push(b'=');
    }
    let mut out = Vec::new();
    for chunk in packed.chunks(4) {
        let pad = chunk.iter().filter(|byte| **byte == b'=').count();
        let mut sextets = [0_u8; 4];
        for (index, byte) in chunk.iter().take(4 - pad).enumerate() {
            sextets[index] = value(*byte);
        }
        let triple = (u32::from(sextets[0]) << 18)
            | (u32::from(sextets[1]) << 12)
            | (u32::from(sextets[2]) << 6)
            | u32::from(sextets[3]);
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
    }
    out
}

fn one_px_png() -> Vec<u8> {
    decode_base64(ONE_PX_PAYLOAD)
}

fn png_renderer() -> MermaidRenderer {
    MermaidRenderer::new(|input: &MermaidRenderInput| {
        assert!(input.code.contains("graph TD"));
        Ok(Some(MermaidRenderOutput {
            bytes: one_px_png(),
            content_type: "image/png".to_string(),
            width_px: Some(320),
            height_px: Some(180),
            alt: String::new(),
            title: None,
        }))
    })
}

fn failing_renderer(message: &'static str) -> MermaidRenderer {
    MermaidRenderer::new(move |_input: &MermaidRenderInput| {
        Err(md_to_docx::MarkdownConversionError::new(message))
    })
}

fn empty_renderer() -> MermaidRenderer {
    MermaidRenderer::new(|_input: &MermaidRenderInput| Ok(None))
}

fn zero_spacers(doc: &Document) -> usize {
    doc.paragraphs()
        .into_iter()
        .filter(|paragraph| {
            paragraph.spacing_before.as_deref() == Some("0")
                && paragraph.spacing_after.as_deref() == Some("0")
        })
        .count()
}

// Jest: "renders inline and block math as native Word math objects"
#[test]
fn inline_and_block_math_become_omml() {
    let doc = convert(
        "Inline $x^2$ appears here.

$$
\\frac{1}{2}
$$",
    );
    let math = math_objects(&doc);
    assert_eq!(math.len(), 2, "math\n{}", doc.render());
    assert!(
        math[0].tags.iter().any(|tag| tag == "sSup"),
        "superscript {:?}\n{}",
        math[0].tags,
        doc.render()
    );
    assert!(math[0].text.contains('x') && math[0].text.contains('2'));
    assert!(
        math[1].tags.iter().any(|tag| tag == "f"),
        "fraction {:?}\n{}",
        math[1].tags,
        doc.render()
    );
    assert!(math[1].text.contains('1') && math[1].text.contains('2'));
    let text = joined(&doc);
    assert!(!text.contains("$x^2$"), "{text}");
    assert!(!text.contains("\\frac"), "{text}");
}

// Slice 6 plan: `_`, a combined script, and `\sqrt` are in the TeX subset.
// Jest: "renders inline and block math as native Word math objects"
#[test]
fn scripts_and_square_root_become_omml() {
    let doc = convert("$x_i$ and $x_i^2$ and $\\sqrt{x}$");
    let math = math_objects(&doc);
    assert_eq!(math.len(), 3, "math\n{}", doc.render());
    assert!(
        math[0].tags.iter().any(|tag| tag == "sSub"),
        "{:?}",
        math[0]
    );
    assert!(math[0].text.contains('x') && math[0].text.contains('i'));
    assert!(
        math[1].tags.iter().any(|tag| tag == "sSubSup"),
        "{:?}",
        math[1]
    );
    assert!(math[2].tags.iter().any(|tag| tag == "rad"), "{:?}", math[2]);
    assert!(math[2].text.contains('x'));
    assert!(!joined(&doc).contains("\\sqrt"));
}

// Slice 6 plan: `COMMAND_REPLACEMENTS` become math runs, not the command names.
// Jest: "renders inline and block math as native Word math objects"
#[test]
fn command_replacements_become_math_text() {
    let doc = convert(r"$\alpha \times \leq \sum \sin$");
    let math = math_objects(&doc);
    assert_eq!(math.len(), 1, "math\n{}", doc.render());
    assert_eq!(math[0].text, "α × ≤ ∑ sin", "math text {}", math[0].text);
    assert!(!math[0].text.contains('\\'));
    assert!(!joined(&doc).contains('\\'));
}

// Jest: "keeps escaped dollar math syntax as literal text"
#[test]
fn escaped_dollars_stay_text() {
    let doc = convert("Only escaped \\$x^2$ remains text.");
    assert_no_math(&doc, "$x^2$ remains text.");
}

// Jest: "falls back to literal text for unsupported TeX by default"
#[test]
fn unsupported_tex_stays_source() {
    let doc = convert("Unsupported $\\overline{x}$ stays literal.");
    assert_no_math(&doc, "$\\overline{x}$");
}

// Jest: "falls back to literal text for unsupported TeX by default"
#[test]
fn root_degree_stays_source() {
    let doc = convert("Degree $\\sqrt[3]{x}$ stays literal.");
    assert_no_math(&doc, "$\\sqrt[3]{x}$");
}

// Jest: "throws for unsupported TeX when configured"
#[test]
fn unsupported_tex_throws_when_configured() {
    let err = md_to_docx::convert_with_options("Unsupported $\\overline{x}$", &math_throw())
        .expect_err("unsupported TeX should fail");
    assert!(
        err.to_string().contains("Unsupported math expression"),
        "message {err}"
    );
}

// Jest: "can disable math parsing"
#[test]
fn math_parsing_can_be_disabled() {
    let doc = convert_with("Inline $x^2$ remains literal.", &math_off());
    assert_no_math(&doc, "$x^2$ remains literal.");
}

// Jest: "embeds enabled chart fences as DOCX images"
#[test]
fn enabled_bar_chart_is_a_png_of_the_block_size() {
    let doc = convert_with(&format!("```chart\n{BAR_CHART}\n```"), &charts_on());
    assert_chart_png(&doc, 160, 90);
    let drawing = doc
        .paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.drawings.iter())
        .next()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.descr.as_deref(), Some("Revenue chart"));
}

// Jest: "keeps chart fences as ordinary code blocks when disabled"
#[test]
fn chart_fences_stay_code_when_disabled() {
    let doc = convert(&format!("```chart\n{BAR_CHART}\n```"));
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
    assert!(doc.media.is_empty(), "media {:?}", doc.media_parts);
    let text = joined(&doc);
    assert!(text.contains("chart"), "{text}");
    assert!(text.contains("\"bar\""), "{text}");
    let code = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.text.contains("chart"))
        .unwrap_or_else(|| panic!("no code paragraph\n{}", doc.render()));
    assert!(
        code.runs.iter().any(|run| run.code),
        "code runs\n{}",
        doc.render()
    );
}

// Jest: "keeps chart fences as ordinary code blocks when disabled"
#[test]
fn chartjs_fences_stay_code_when_disabled() {
    let doc = convert(&format!("```chartjs\n{LINE_CHART}\n```"));
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
    assert!(doc.media.is_empty(), "media {:?}", doc.media_parts);
    let text = joined(&doc);
    assert!(text.contains("chartjs"), "{text}");
    assert!(text.contains("\"line\""), "{text}");
}

// Jest: "renders invalid chart definitions as clear placeholders by default"
#[test]
fn invalid_chart_is_a_placeholder() {
    let doc = convert_with(
        "```chart\n{\"type\":\"bar\",\"data\":{\"datasets\":[{\"data\":[\"bad\"]}]}}\n```",
        &charts_on(),
    );
    let text = joined(&doc);
    assert!(text.contains("Chart could not be rendered"), "{text}");
    assert!(text.contains("finite numbers"), "{text}");
    assert!(doc.media.is_empty(), "media {:?}", doc.media_parts);
}

// Jest: "can throw on invalid chart definitions"
#[test]
fn invalid_chart_can_throw() {
    md_to_docx::convert_with_options("```chart\n{\"type\":\"scatter\"}\n```", &charts_throw())
        .expect_err("scatter chart should fail");
}

// Jest: "uses chart block dimensions for DOCX image sizing"
#[test]
fn line_chart_png_uses_block_dimensions() {
    let doc = convert_with(&format!("```chartjs\n{LINE_CHART}\n```"), &charts_on());
    assert_chart_png(&doc, 321, 123);
    let drawing = doc
        .paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.drawings.iter())
        .next()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.cx, Some(321 * EMU_PER_PX));
    assert_eq!(drawing.cy, Some(123 * EMU_PER_PX));
}

// Jest: "fills a single-slice pie chart instead of rendering a blank image"
#[test]
fn pie_chart_is_a_png_of_the_block_size() {
    let doc = convert_with(&format!("```chart\n{PIE_CHART}\n```"), &charts_on());
    assert_chart_png(&doc, 96, 96);
}

// Jest: "fills a single-slice doughnut chart instead of rendering a blank image"
#[test]
fn doughnut_chart_is_a_png_of_the_block_size() {
    let doc = convert_with(&format!("```chart\n{DOUGHNUT_CHART}\n```"), &charts_on());
    assert_chart_png(&doc, 96, 96);
}

// Jest: "fills a full-total doughnut slice in a multi-value chart"
#[test]
fn full_doughnut_slice_is_a_png_of_the_block_size() {
    let doc = convert_with(&format!("```chart\n{DOUGHNUT_FULL}\n```"), &charts_on());
    assert_chart_png(&doc, 96, 96);
}

// Slice 6 plan: omitted chart width and height are 640 by 360.
#[test]
fn omitted_chart_size_is_the_builtin_default() {
    let doc = convert_with(&format!("```chart\n{DEFAULT_CHART}\n```"), &charts_on());
    assert_chart_png(&doc, 640, 360);
}

// Jest: "keeps mermaid fences as ordinary code blocks by default"
#[test]
fn mermaid_fences_stay_code_by_default() {
    let doc = convert(MERMAID);
    let text = joined(&doc);
    assert!(text.contains("graph TD"), "{text}");
    assert!(text.contains("mermaid"), "{text}");
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
    assert!(doc.media.is_empty(), "media {:?}", doc.media_parts);
}

// Jest: "keeps mermaid fences as ordinary code blocks when disabled explicitly"
#[test]
fn mermaid_fences_stay_code_when_disabled() {
    let options = ConvertOptions {
        mermaid_rendering: Some(MermaidRenderingOptions {
            enabled: Some(OptionBool::Bool(false)),
            ..MermaidRenderingOptions::default()
        }),
        ..ConvertOptions::default()
    };
    let doc = convert_with(MERMAID, &options);
    assert!(joined(&doc).contains("graph TD"), "{}", doc.render());
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
}

// Jest: "embeds renderer output as a DOCX image when enabled"
#[test]
fn mermaid_renderer_output_becomes_an_image() {
    let doc = convert_with(MERMAID, &mermaid_options(None, png_renderer()));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert_eq!(doc.media.len(), 1, "media {:?}", doc.media_parts);
    assert!(!joined(&doc).contains("graph TD"), "{}", doc.render());
}

// Jest: "falls back to the original code block when rendering fails"
#[test]
fn mermaid_render_failure_stays_code() {
    let doc = convert_with(
        MERMAID,
        &mermaid_options(None, failing_renderer("Mermaid runtime unavailable")),
    );
    let text = joined(&doc);
    assert!(text.contains("graph TD"), "{text}");
    assert!(text.contains("mermaid"), "{text}");
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
}

// Jest: "preserves a list marker when Mermaid fallback is the first list item block"
#[test]
fn mermaid_list_fallback_keeps_one_marker() {
    let markdown = "- ```mermaid
  graph TD
    A --> B
  ```";
    let doc = convert_with(markdown, &mermaid_options(None, empty_renderer()));
    assert!(joined(&doc).contains("graph TD"), "{}", doc.render());
    assert_eq!(
        doc.paragraphs()
            .into_iter()
            .filter(|paragraph| paragraph.list.is_some())
            .count(),
        1,
        "lists\n{}",
        doc.render()
    );
}

// Jest: "keeps a spacer between adjacent Mermaid blocks that fall back to code"
#[test]
fn adjacent_mermaid_fallbacks_keep_a_spacer() {
    let markdown = "```mermaid
graph TD
  A --> B
```

```mermaid
graph TD
  C --> D
```";
    let doc = convert_with(
        markdown,
        &mermaid_options(None, failing_renderer("renderer unavailable")),
    );
    let text = joined(&doc);
    assert!(text.contains("A --> B"), "{text}");
    assert!(text.contains("C --> D"), "{text}");
    assert!(zero_spacers(&doc) >= 1, "spacers\n{}", doc.render());
}

// Jest: "does not add a spacer after nested Mermaid fallback code"
#[test]
fn nested_mermaid_fallback_has_no_spacer() {
    let options = mermaid_options(None, empty_renderer());
    let list = convert_with(
        "- ```mermaid
  graph TD
    A --> B
  ```

```typescript
const topLevel = true;
```",
        &options,
    );
    let quote = convert_with(
        "> ```mermaid
> graph TD
>   A --> B
> ```

```typescript
const topLevel = true;
```",
        &options,
    );
    assert!(joined(&list).contains("graph TD"), "{}", list.render());
    assert!(
        joined(&list).contains("const topLevel"),
        "{}",
        list.render()
    );
    assert_eq!(zero_spacers(&list), 0, "list spacers\n{}", list.render());
    assert!(joined(&quote).contains("graph TD"), "{}", quote.render());
    assert!(
        joined(&quote).contains("const topLevel"),
        "{}",
        quote.render()
    );
    assert_eq!(zero_spacers(&quote), 0, "quote spacers\n{}", quote.render());
}

// Jest: "can render a placeholder or throw on render failure"
#[test]
fn mermaid_failure_can_be_a_placeholder_or_a_throw() {
    let placeholder = convert_with(
        MERMAID,
        &mermaid_options(Some(MermaidFailureMode::Placeholder), empty_renderer()),
    );
    let text = joined(&placeholder);
    assert!(
        text.contains("Mermaid diagram could not be rendered"),
        "{text}"
    );
    assert!(!text.contains("graph TD"), "{text}");

    md_to_docx::convert_with_options(
        MERMAID,
        &mermaid_options(
            Some(MermaidFailureMode::Throw),
            failing_renderer("bad diagram"),
        ),
    )
    .expect_err("mermaid throw mode should fail");
}
