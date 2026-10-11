use std::io::{Cursor, Read};

use md_to_docx::{
    convert_with_options, convert_with_reference, ReferenceDocxGenerationOptions,
    ReferenceDocxModeOptions,
};
use zip::ZipArchive;

const TEMPLATE: &[u8] = include_bytes!("oracle/fixtures/reference-template.docx");

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn tag(xml: &str, name: &str) -> String {
    let start = xml.find(&format!("<w:{name} ")).unwrap();
    let end = start + xml[start..].find("/>").unwrap() + 2;
    xml[start..end].to_string()
}

#[test]
fn reference_flags_independently_control_layout_and_header_footer_import() {
    let loaded = md_to_docx::options_json::load_options_json(
        r#"{"template":{"page":{"margin":{"top":1234,"left":1600}},"headers":{"default":{"text":"Own header"}},"footers":{"default":{"text":"Own footer"}}}}"#,
        "test",
    ).unwrap();
    let original = convert_with_options("body", &loaded.convert).unwrap();
    let original_xml = part(&original, "word/document.xml");
    let reference_xml = part(TEMPLATE, "word/document.xml");
    assert_ne!(tag(&original_xml, "pgMar"), tag(&reference_xml, "pgMar"));

    for (layout, headers) in [(false, false), (false, true), (true, false), (true, true)] {
        let options = ReferenceDocxGenerationOptions {
            convert: loaded.convert.clone(),
            reference: Some(ReferenceDocxModeOptions {
                preserve_page_layout: Some(layout),
                preserve_headers_and_footers: Some(headers),
                ..Default::default()
            }),
            ..Default::default()
        };
        let output = convert_with_reference("body", TEMPLATE, &options).unwrap();
        let document = part(&output, "word/document.xml");
        let relationships = part(&output, "word/_rels/document.xml.rels");
        let expected_layout = if layout {
            &reference_xml
        } else {
            &original_xml
        };
        assert_eq!(tag(&document, "pgMar"), tag(expected_layout, "pgMar"));
        assert_eq!(tag(&document, "pgSz"), tag(expected_layout, "pgSz"));
        for kind in ["header", "footer"] {
            assert_eq!(
                relationships.contains(&format!("reference-package/word/{kind}1.xml")),
                headers,
                "layout={layout}, headers={headers}, kind={kind}"
            );
            if !headers {
                assert!(document.contains(&format!("w:{kind}Reference")));
                let story = part(&output, &format!("word/{kind}1.xml"));
                assert!(story.contains(&format!("Own {kind}")));
            }
        }
    }
}

#[test]
fn disabling_header_footer_import_keeps_a_body_without_template_headers() {
    let options = ReferenceDocxGenerationOptions {
        reference: Some(ReferenceDocxModeOptions {
            preserve_page_layout: Some(false),
            preserve_headers_and_footers: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    let output = convert_with_reference("body", TEMPLATE, &options).unwrap();
    let document = part(&output, "word/document.xml");
    let relationships = part(&output, "word/_rels/document.xml.rels");
    assert!(!document.contains("w:headerReference"));
    assert!(!relationships.contains("reference-package/word/header"));
    assert!(!relationships.contains("reference-package/word/footer"));
}
