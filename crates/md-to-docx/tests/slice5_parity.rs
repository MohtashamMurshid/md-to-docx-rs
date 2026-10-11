//! Slice 5 parity with `tests/rendering.test.ts` image cases,
//! `tests/image-security.test.ts`, and the SVG/WebP case in
//! `tests/feature-completeness.test.ts`.
//!
//! Media bytes for PNG and JPEG are compared exactly. SVG and WebP are
//! expected to become a PNG of the source pixel size, not a Sharp pixel match.
//! ZIP bytes are not compared.
//!
//! `ConvertOptions` has no image-handling field. SSRF and limit cases that can
//! reject before a socket call `md_to_docx::load_markdown_image` with
//! [`md_to_docx::ImageOptions`]. These Jest cases need a mocked HTTPS response
//! or `imageHandling` on the converter, so they are not sent:
//! - "rejects remote images with oversized content-length"
//! - "rejects remote images that exceed the streamed byte cap"
//! - "re-checks redirected remote image targets"
//! - "stops following redirects after maxRedirects"
//! - "aborts slow remote image fetches"
//! - "aborts slow remote image response bodies"
//! - "still fetches hosts present in the allowedHosts list"
//! - "applies maximum image count across document sections"
//! - "applies maximum image count across top-level and nested images"
//!
//! Strict alt text uses `accessibility.missing_image_alt_text`. Remote checks
//! use a local listener. No public host is contacted.

#[path = "common/mod.rs"]
mod common;

use std::io::ErrorKind;
use std::net::TcpListener;

use common::{Document, Drawing, MediaFile, Paragraph};
use md_to_docx::options::{AccessibilityOptions, MissingImageAltText};
use md_to_docx::{ConvertOptions, ImageOptions};

const ONE_PX_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";
const ONE_PX_PAYLOAD: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";
const JPEG_PAYLOAD: &str = "/9j/4AAQSkZJRgABAgAAAQABAAD//gAQTGF2YzYwLjMxLjEwMgD/2wBDAAgGBgcGBwgICAgICAkJCQoKCgkJCQkKCgoKCgoMDAwKCgoKCgoKDAwMDA0ODQ0NDA0ODg8PDxISEREVFRUZGR//xABMAAEBAAAAAAAAAAAAAAAAAAAABgEBAQAAAAAAAAAAAAAAAAAABgcQAQAAAAAAAAAAAAAAAAAAAAARAQAAAAAAAAAAAAAAAAAAAAD/wAARCAAIAAgDASIAAhEAAxEA/9oADAMBAAIRAxEAPwCLAFF/f//Z";
const WEBP_PAYLOAD: &str = "UklGRhwAAABXRUJQVlA4TA8AAAAvE0ACAAcQ0f/+ByKi/wEA";
const SVG_PAYLOAD: &str = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjIwIiBoZWlnaHQ9IjEwIiBmaWxsPSJyZWQiLz48L3N2Zz4=";

const EMU_PER_PX: u64 = 9525;

fn convert(markdown: &str) -> Document {
    let bytes = md_to_docx::convert_markdown(markdown)
        .unwrap_or_else(|err| panic!("convert_markdown failed: {err}"));
    assert!(
        bytes.starts_with(b"PK"),
        "convert_markdown should return a zip package"
    );
    common::project_docx(&bytes)
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

fn drawings(doc: &Document) -> Vec<&Drawing> {
    doc.paragraphs()
        .into_iter()
        .flat_map(|paragraph| paragraph.drawings.iter())
        .collect()
}

fn only_media(doc: &Document) -> &MediaFile {
    assert_eq!(
        doc.media.len(),
        1,
        "media {:?}\n{}",
        doc.media_parts,
        doc.render()
    );
    &doc.media[0]
}

fn assert_fallback(doc: &Document) {
    let text = doc.paragraph_texts().join("\n");
    assert!(
        text.contains("Image could not be displayed") || text.contains("Image could not be loaded"),
        "missing image fallback\n{}",
        doc.render()
    );
    assert!(
        doc.media.is_empty(),
        "failed image should not add media {:?}",
        doc.media_parts
    );
    assert_eq!(doc.drawing_count, 0, "drawings\n{}", doc.render());
}

fn assert_no_connection(listener: &TcpListener) {
    match listener.accept() {
        Err(err) if err.kind() == ErrorKind::WouldBlock => {}
        Ok((stream, addr)) => panic!("image fetch connected from {addr} on {stream:?}"),
        Err(err) => panic!("listener failed: {err}"),
    }
}

fn local_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    (listener, port)
}

fn remote_options() -> ImageOptions {
    let mut options = ImageOptions::default();
    options.remote_enabled = true;
    options.fetch_timeout_ms = 200;
    options
}

fn load_remote(source: &str, options: &ImageOptions) -> String {
    md_to_docx::load_markdown_image(source, "remote", None, options)
        .expect_err("remote image should fail")
        .to_string()
}

// Jest: "embeds a data-URL image with explicit width and height"
#[test]
fn data_url_image_has_explicit_width_and_height() {
    let doc = convert(&format!("![one px]({ONE_PX_PNG}#w=50&h=50)"));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert!(!doc.media_parts.is_empty(), "media\n{}", doc.render());
    let drawing = drawings(&doc)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.cx, Some(50 * EMU_PER_PX));
    assert_eq!(drawing.cy, Some(50 * EMU_PER_PX));
    assert_eq!(drawing.descr.as_deref(), Some("one px"));
}

// Jest: "embeds images inside list items"
#[test]
fn images_inside_list_items() {
    let doc = convert(&format!("- ![list image]({ONE_PX_PNG})"));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert!(!doc.media.is_empty());
    let paragraph = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| !paragraph.drawings.is_empty())
        .unwrap_or_else(|| panic!("no image paragraph\n{}", doc.render()));
    assert!(paragraph.list.is_some(), "list\n{}", doc.render());
    assert_eq!(drawings(&doc)[0].descr.as_deref(), Some("list image"));
}

// Jest: "embeds images inside blockquotes"
#[test]
fn images_inside_blockquotes() {
    let doc = convert(&format!("> ![quoted image]({ONE_PX_PNG})"));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert!(!doc.media.is_empty());
    let paragraph = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| !paragraph.drawings.is_empty())
        .unwrap_or_else(|| panic!("no image paragraph\n{}", doc.render()));
    assert!(
        paragraph.left_border_color.is_some(),
        "blockquote border\n{}",
        doc.render()
    );
    assert_eq!(drawings(&doc)[0].descr.as_deref(), Some("quoted image"));
}

// Jest: "preserves images mixed with paragraph text"
#[test]
fn images_mixed_with_paragraph_text() {
    let doc = convert(&format!("Before ![mixed image]({ONE_PX_PNG}) after"));
    let paragraph = paragraphs_one(&doc);
    assert!(paragraph.text.contains("Before "));
    assert!(paragraph.text.contains(" after"));
    assert_eq!(paragraph.drawings.len(), 1);
    assert_eq!(paragraph.drawings[0].descr.as_deref(), Some("mixed image"));
    assert!(!doc.media.is_empty());
}

// Jest: "does not duplicate markers for split list item content"
#[test]
fn list_item_keeps_one_marker_around_an_image() {
    let doc = convert(&format!("- Before ![mixed image]({ONE_PX_PNG}) after"));
    let lists: Vec<&Paragraph> = doc
        .paragraphs()
        .into_iter()
        .filter(|paragraph| paragraph.list.is_some())
        .collect();
    assert_eq!(lists.len(), 1, "list paragraphs\n{}", doc.render());
    assert!(lists[0].text.contains("Before "));
    assert!(lists[0].text.contains(" after"));
    assert_eq!(lists[0].drawings.len(), 1);
}

// Jest: "keeps a task marker and standalone image in the same list paragraph"
#[test]
fn task_marker_and_image_share_a_list_paragraph() {
    let doc = convert(&format!("- [ ] ![task image]({ONE_PX_PNG})"));
    let paragraph = doc
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.text.contains('☐') && !paragraph.drawings.is_empty())
        .unwrap_or_else(|| panic!("task image paragraph\n{}", doc.render()));
    assert!(paragraph.list.is_some());
    assert_eq!(paragraph.drawings[0].descr.as_deref(), Some("task image"));
}

// Jest: "renders a fallback for invalid nested images"
#[test]
fn invalid_nested_image_falls_back() {
    let doc = convert("> ![broken](not-a-url)");
    let text = doc.paragraph_texts().join("\n");
    assert!(
        text.contains("Image could not be displayed") || text.contains("Image could not be loaded"),
        "{text}\n{}",
        doc.render()
    );
    assert!(text.contains("broken"), "{text}");
    assert!(doc.media.is_empty());
}

// Jest: "clamps oversized dimension hints from URL fragments"
#[test]
fn oversized_dimension_hints_are_clamped() {
    let doc = convert(&format!("![huge]({ONE_PX_PNG}#99999999999x99999999999)"));
    let drawing = drawings(&doc)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    let cap = 10_000 * EMU_PER_PX;
    assert!(drawing.cx.unwrap_or(u64::MAX) <= cap);
    assert!(drawing.cy.unwrap_or(u64::MAX) <= cap);
}

// Jest: "keeps reasonable dimension hints intact"
#[test]
fn reasonable_dimension_hints_stay_intact() {
    let doc = convert(&format!("![sized]({ONE_PX_PNG}#300x150)"));
    let drawing = drawings(&doc)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.cx, Some(300 * EMU_PER_PX));
    assert_eq!(drawing.cy, Some(150 * EMU_PER_PX));
}

// Jest: "rejects remote images by default"
#[test]
fn remote_images_are_rejected_by_default() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let local = convert(&format!("![remote](https://127.0.0.1:{port}/image.png)"));
    assert_no_connection(&listener);
    assert_fallback(&local);
}

// Jest: "rejects http URLs even when remote image fetching is enabled"
#[test]
fn http_urls_are_rejected_when_remote_is_enabled() {
    let (listener, port) = local_listener();
    let err = load_remote(
        &format!("http://127.0.0.1:{port}/image.png"),
        &remote_options(),
    );
    assert!(
        err.contains("https"),
        "http remote image should fail before fetch: {err}"
    );
    assert_no_connection(&listener);
}

// Jest: "rejects loopback and private-network addresses before fetch"
#[test]
fn loopback_addresses_are_rejected_before_fetch() {
    let (listener, port) = local_listener();
    let source = format!("https://127.0.0.1:{port}/image.png");
    let doc = convert(&format!("![remote]({source})"));
    assert_no_connection(&listener);
    assert_fallback(&doc);
    let err = load_remote(&source, &remote_options());
    assert!(
        err.contains("blocked network address"),
        "loopback should fail before fetch: {err}"
    );
    assert_no_connection(&listener);
}

// Jest: "rejects IPv4-mapped IPv6 private addresses before fetch"
#[test]
fn ipv4_mapped_ipv6_private_addresses_are_rejected() {
    let err = load_remote("https://[::ffff:192.168.1.10]/image.png", &remote_options());
    assert!(
        err.contains("blocked network address"),
        "mapped address should fail before fetch: {err}"
    );
}

// Jest: "rejects hex IPv4-mapped IPv6 hostnames before fetch"
#[test]
fn hex_ipv4_mapped_ipv6_hostnames_are_rejected() {
    let err = load_remote("https://[::ffff:c0a8:010a]/image.png", &remote_options());
    assert!(
        err.contains("blocked network address"),
        "hex mapped address should fail before fetch: {err}"
    );
}

// Jest: "rejects uncompressed IPv6-mapped private addresses before fetch"
#[test]
fn uncompressed_ipv6_mapped_private_addresses_are_rejected() {
    let err = load_remote(
        "https://[0:0:0:0:0:ffff:c0a8:010a]/image.png",
        &remote_options(),
    );
    assert!(
        err.contains("blocked network address"),
        "uncompressed mapped address should fail before fetch: {err}"
    );
}

// Jest: "rejects oversized data URLs before embedding"
#[test]
fn oversized_data_urls_are_rejected() {
    let mut options = ImageOptions::default();
    options.max_image_bytes = 16;
    let err = load_remote(
        &format!("data:image/png;base64,{}", "A".repeat(128)),
        &options,
    );
    assert!(
        err.contains("Data URL image exceeds maximum size"),
        "oversized data URL: {err}"
    );
}

// Jest: "does not count failed images toward maxImages"
#[test]
fn failed_images_do_not_count_toward_max_images() {
    let (listener, port) = local_listener();
    let mut options = remote_options();
    options.max_images = 1;
    let first = load_remote(&format!("https://127.0.0.1:{port}/a.png"), &options);
    let second = load_remote(&format!("https://127.0.0.1:{port}/b.png"), &options);
    assert!(first.contains("blocked network address"), "{first}");
    assert!(
        second.contains("blocked network address") || second.contains("Image budget reached"),
        "{second}"
    );
    md_to_docx::load_markdown_image(ONE_PX_PNG, "ok", None, &options)
        .unwrap_or_else(|err| panic!("data URL should still load: {err}"));
    assert_no_connection(&listener);
}

// Jest: "stops attempting remote fetches once the failed-image budget is spent"
#[test]
fn failed_image_budget_stops_later_attempts() {
    let (listener, port) = local_listener();
    let mut options = remote_options();
    options.max_images = 2;
    let mut errors = Vec::new();
    for index in 0..6 {
        errors.push(load_remote(
            &format!("https://127.0.0.1:{port}/missing-{index}.png"),
            &options,
        ));
    }
    assert!(
        errors[..2]
            .iter()
            .all(|err| err.contains("blocked network address")),
        "first attempts {errors:?}"
    );
    assert!(
        errors[2..]
            .iter()
            .all(|err| err.contains("Image budget reached")),
        "later attempts {errors:?}"
    );
    assert_no_connection(&listener);
}

// Jest: "denies every remote host when an empty allowedHosts list is provided"
#[test]
fn empty_allowed_hosts_denies_every_host() {
    let (listener, port) = local_listener();
    let mut options = remote_options();
    options.allowed_hosts = Some(Vec::new());
    let err = load_remote(&format!("https://127.0.0.1:{port}/image.png"), &options);
    assert!(err.contains("not allowed"), "empty allowed hosts: {err}");
    assert_no_connection(&listener);
}

// Jest: "enforces the maximum image count"
#[test]
fn maximum_image_count_stops_embeds() {
    let mut options = ImageOptions::default();
    options.max_images = 1;
    md_to_docx::load_markdown_image(ONE_PX_PNG, "one", None, &options)
        .unwrap_or_else(|err| panic!("first image should load: {err}"));
    let err = load_remote(ONE_PX_PNG, &options);
    assert!(err.contains("Image budget reached"), "second image: {err}");
}

// Jest: "continues to embed valid data URL images by default"
#[test]
fn data_url_images_embed_by_default() {
    let doc = convert(&format!("![one px]({ONE_PX_PNG})"));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    assert!(!doc.media.is_empty());
    assert_eq!(drawings(&doc)[0].descr.as_deref(), Some("one px"));
}

// Jest: "writes Markdown image alt text and title into Word drawing properties"
#[test]
fn alt_text_and_title_become_drawing_properties() {
    let doc = convert(&format!(
        r#"![Revenue & growth]({ONE_PX_PNG} "Chart <Q1>")"#
    ));
    let drawing = drawings(&doc)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.descr.as_deref(), Some("Revenue & growth"));
    assert_eq!(drawing.name.as_deref(), Some("Chart <Q1>"));
    assert_eq!(drawing.title.as_deref(), Some("Chart <Q1>"));
}

// Jest: "allows empty alt text by default without claiming decorative semantics"
#[test]
fn empty_alt_text_is_allowed_by_default() {
    let doc = convert(&format!("![]({ONE_PX_PNG})"));
    assert_eq!(doc.drawing_count, 1, "drawings\n{}", doc.render());
    let drawing = drawings(&doc)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing drawing\n{}", doc.render()));
    assert_eq!(drawing.descr, None);
    assert_eq!(drawing.name.as_deref(), Some("Image"));
    assert!(!doc.paragraph_texts().join("\n").contains("decorative"));
}

// Jest: "throws for empty normal and remote image alt text in strict mode before lookup"
#[test]
fn strict_missing_alt_text_throws_before_lookup() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let options = ConvertOptions {
        accessibility: Some(AccessibilityOptions {
            missing_image_alt_text: Some(MissingImageAltText::Throw),
        }),
        ..ConvertOptions::default()
    };
    let data = md_to_docx::convert_with_options(&format!("![]({ONE_PX_PNG})"), &options)
        .expect_err("blank data-URL alt text should fail");
    assert!(
        data.to_string().contains("strict accessibility mode"),
        "message {data}"
    );
    let remote = md_to_docx::convert_with_options(
        &format!("![](https://127.0.0.1:{port}/image.png)"),
        &options,
    )
    .expect_err("blank remote alt text should fail");
    assert!(
        remote.to_string().contains("strict accessibility mode"),
        "message {remote}"
    );
    assert_no_connection(&listener);
}

// Jest: "rejects invalid and oversized image descriptions with conversion context"
#[test]
fn oversized_alt_text_is_rejected() {
    let alt = "x".repeat(2049);
    let err = md_to_docx::convert_markdown(&format!("![{alt}]({ONE_PX_PNG})"))
        .expect_err("oversized alt text should fail");
    assert!(
        err.to_string().contains("maximum length of 2048"),
        "message {}",
        err
    );
}

// Jest: "rejects invalid and oversized image descriptions with conversion context"
#[test]
fn control_character_alt_text_is_rejected() {
    let err = md_to_docx::convert_markdown(&format!("![bad\u{1}alt]({ONE_PX_PNG})"))
        .expect_err("control character alt text should fail");
    assert!(
        err.to_string().contains("not valid in OOXML"),
        "message {}",
        err
    );
}

// Slice 5 plan: PNG stays the data-URL bytes. Jest does not compare that payload.
#[test]
fn png_media_bytes_match_the_data_url() {
    let doc = convert(&format!("![one px]({ONE_PX_PNG})"));
    let media = only_media(&doc);
    assert!(media.path.ends_with(".png"), "path {}", media.path);
    assert_eq!(media.bytes, decode_base64(ONE_PX_PAYLOAD));
    assert_eq!(png_size(&media.bytes), (1, 1));
}

// Slice 5 plan: JPEG stays the data-URL bytes. Jest does not compare that payload.
#[test]
fn jpeg_media_bytes_stay_jpeg() {
    let doc = convert(&format!("![photo](data:image/jpeg;base64,{JPEG_PAYLOAD})"));
    let media = only_media(&doc);
    assert!(
        media.path.ends_with(".jpeg") || media.path.ends_with(".jpg"),
        "path {}",
        media.path
    );
    let expected = decode_base64(JPEG_PAYLOAD);
    assert_eq!(media.bytes, expected);
    assert!(media.bytes.starts_with(&[0xff, 0xd8, 0xff]));
    assert_eq!(drawings(&doc)[0].descr.as_deref(), Some("photo"));
}

// Jest: "converts SVG and WebP to Word-compatible PNGs"
#[test]
fn svg_becomes_a_png_of_the_source_pixel_size() {
    let doc = convert(&format!(
        "![Vector](data:image/svg+xml;base64,{SVG_PAYLOAD})"
    ));
    let media = only_media(&doc);
    assert!(media.path.ends_with(".png"), "path {}", media.path);
    assert_eq!(png_size(&media.bytes), (20, 10));
    assert_eq!(doc.drawing_count, 1);
}

// Jest: "converts SVG and WebP to Word-compatible PNGs"
#[test]
fn webp_becomes_a_png_of_the_source_pixel_size() {
    let doc = convert(&format!("![Web](data:image/webp;base64,{WEBP_PAYLOAD})"));
    let media = only_media(&doc);
    assert!(media.path.ends_with(".png"), "path {}", media.path);
    assert_eq!(png_size(&media.bytes), (20, 10));
    assert_eq!(doc.drawing_count, 1);
}

fn paragraphs_one(doc: &Document) -> &Paragraph {
    let paragraphs = doc.paragraphs();
    assert_eq!(paragraphs.len(), 1, "paragraphs\n{}", doc.render());
    paragraphs[0]
}
