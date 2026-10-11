use md_to_docx::model::Block;
use md_to_docx::options::{
    MarkdownDocxPlugin, PluginAstTransform, PluginBlock, PluginBlockNodeHandler, PluginFailureMode,
    PluginInline,
};
use md_to_docx::{convert_with_options, ConvertOptions, ImageOptions, ResolvedImage};
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

const REFERENCE_DOCX: &[u8] = include_bytes!("oracle/fixtures/reference-template.docx");
fn parts(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut body = Vec::new();
            file.read_to_end(&mut body).unwrap();
            (name, body)
        })
        .collect()
}
fn xml(bytes: &[u8]) -> String {
    String::from_utf8(
        parts(bytes)
            .into_iter()
            .find(|(name, _)| name == "word/document.xml")
            .unwrap()
            .1,
    )
    .unwrap()
}
fn media_count(bytes: &[u8]) -> usize {
    parts(bytes)
        .iter()
        .filter(|(name, _)| name.starts_with("word/media/"))
        .count()
}
fn tiny_png() -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
// ImageOptions owns private counters, so external callers configure a default value.
#[allow(clippy::field_reassign_with_default)]
#[test]
fn image_resolver_settings_reach_conversion_and_budgets_restart() {
    let mut images = ImageOptions::default();
    images.max_images = 1;
    images.resolve = Some(Arc::new(|_, _| {
        Ok(Some(ResolvedImage {
            bytes: tiny_png(),
            content_type: Some("image/png".into()),
        }))
    }));
    let options = ConvertOptions {
        image_handling: Some(images),
        ..Default::default()
    };
    for _ in 0..2 {
        let bytes = convert_with_options("![a](virtual:a)\n\n![b](virtual:b)", &options).unwrap();
        assert_eq!(media_count(&bytes), 1);
    }
}
#[test]
fn image_json_settings_reach_generated_document() {
    let options=md_to_docx::options_json::load_options_json(r#"{"imageHandling":{"dataUrls":{"enabled":false},"maxImages":0,"remote":{"enabled":true,"allowedHosts":[]}}}"#,"test").unwrap();
    let images = options.convert.image_handling.as_ref().unwrap();
    assert!(!images.data_urls_enabled);
    assert!(images.remote_enabled);
    assert_eq!(images.allowed_hosts, Some(vec![]));
    let bytes =
        convert_with_options("![a](data:image/png;base64,invalid)", &options.convert).unwrap();
    assert_eq!(media_count(&bytes), 0);
}
#[test]
fn ast_transform_changes_content_and_custom_nodes_render() {
    let plugin = MarkdownDocxPlugin {
        name: "custom-block".into(),
        api_version: 1,
        priority: None,
        setup: None,
        transform_ast: Some(PluginAstTransform::new(|document| {
            document.children.clear();
            document.children.push(Block::Custom {
                node_type: "notice".into(),
                value: "replacement".into(),
            });
        })),
        fenced_blocks: None,
        block_nodes: Some(vec![PluginBlockNodeHandler {
            node_types: vec!["notice".into()],
            failure_mode: Some(PluginFailureMode::Throw),
            render: PluginBlockNodeHandler::render_fn(|input| {
                Ok(vec![PluginBlock::Paragraph {
                    children: vec![PluginInline {
                        text: format!("rendered {}", input.value),
                        bold: true,
                        ..Default::default()
                    }],
                }])
            }),
        }]),
    };
    let options = ConvertOptions {
        plugins: Some(vec![plugin]),
        ..Default::default()
    };
    let document = xml(&convert_with_options("original", &options).unwrap());
    assert!(document.contains("rendered replacement"));
    assert!(!document.contains("original"));
    assert!(document.contains("<w:b"));
}
#[test]
fn already_aborted_json_conversion_fails_before_rendering() {
    let options =
        md_to_docx::options_json::load_options_json(r#"{"signal":{"aborted":true}}"#, "test")
            .unwrap();
    let err = convert_with_options("hello", &options.convert).unwrap_err();
    assert_eq!(err.context().unwrap().get("code").unwrap(), "ABORTED");
}
#[test]
fn reference_and_patch_apply_conversion_options() {
    let template = md_to_docx::convert_markdown("{{body}}").unwrap();
    let loaded = md_to_docx::options_json::load_options_json(
        r#"{"codeHighlighting":{"enabled":true}}"#,
        "test",
    )
    .unwrap();
    let reference = md_to_docx::ReferenceDocxGenerationOptions {
        convert: loaded.convert.clone(),
        ..Default::default()
    };
    let code = "```ts\ninterface Report { count: number }\nconst count: number = 42;\n```";
    let bytes = md_to_docx::convert_with_reference(code, REFERENCE_DOCX, &reference).unwrap();
    assert!(xml(&bytes).contains("D73A49"));
    let patches = std::collections::BTreeMap::from([("body".into(), code.into())]);
    let bytes = md_to_docx::patch_markdown(&template, &patches, &loaded.patch).unwrap();
    assert!(xml(&bytes).contains("D73A49"));
}

#[test]
fn patch_limit_applies_after_plugin_expansion() {
    let plugin = MarkdownDocxPlugin {
        name: "expand".into(),
        api_version: 1,
        priority: None,
        setup: None,
        fenced_blocks: None,
        block_nodes: None,
        transform_ast: Some(PluginAstTransform::new(|document| {
            let first = document.children[0].clone();
            document.children.extend([first.clone(), first]);
        })),
    };
    let options = md_to_docx::PatchMarkdownOptions {
        max_elements: Some(2),
        convert: ConvertOptions {
            plugins: Some(vec![plugin]),
            max_elements: Some(100),
            ..Default::default()
        },
        ..Default::default()
    };
    let template = md_to_docx::convert_markdown("{{body}}").unwrap();
    let patches = std::collections::BTreeMap::from([("body".into(), "one".into())]);
    assert!(md_to_docx::patch_markdown(&template, &patches, &options).is_err());
}
#[allow(clippy::field_reassign_with_default)]
#[test]
fn image_handling_strict_alt_policy_reaches_parser() {
    let mut images = ImageOptions::default();
    images.missing_image_alt_text = md_to_docx::MissingImageAltText::Throw;
    let options = ConvertOptions {
        image_handling: Some(images),
        ..Default::default()
    };
    assert!(convert_with_options("![](missing.png)", &options).is_err());
}
