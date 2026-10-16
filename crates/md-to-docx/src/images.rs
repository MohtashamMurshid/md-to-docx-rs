//! Load one Markdown image into bytes Word can embed.
//!
//! Remote HTTPS is off unless [`ImageOptions::remote_enabled`] is set. Data
//! URLs are on unless [`ImageOptions::data_urls_enabled`] is cleared. The
//! failed-attempt budget matches `max_images` and only counts sources that are
//! not data URLs. Unit tests never open a socket.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use image::GenericImageView;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;
use rustls::pki_types::ServerName;
use url::Url;

use crate::error::MarkdownConversionError;

const MAX_IMAGE_ALT_TEXT_LENGTH: usize = 2048;
const MAX_IMAGE_TITLE_LENGTH: usize = 512;
const MAX_RASTER_PIXELS: u64 = 16_000_000;

/// What to do when a Markdown image has blank alt text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingImageAltText {
    /// Keep the image. This is the default.
    #[default]
    Permissive,
    /// Fail conversion before any fetch or decode.
    Throw,
}

/// Bytes a caller-supplied resolver already loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImage {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// Passed to [`ImageOptions::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageResolveContext {
    pub max_bytes: usize,
}

/// Runs before data URLs, local files, and remote fetch.
///
/// `Ok(None)` falls through to the normal path.
pub type ImageResolver = Arc<
    dyn Fn(&str, &ImageResolveContext) -> Result<Option<ResolvedImage>, MarkdownConversionError>
        + Send
        + Sync,
>;

/// Limits and switches for [`load_markdown_image`].
///
/// Successful loads share one embed budget. Failed non-data-URL attempts share
/// a second budget of the same size. Create a new value with [`Default`].
pub struct ImageOptions {
    /// Fetch `https` images. Default: `false`.
    pub remote_enabled: bool,
    /// Exact host allow list. `None` allows any public host. `Some` list,
    /// including an empty one, allows only those hosts.
    pub allowed_hosts: Option<Vec<String>>,
    /// Accept `data:` images. Default: `true`.
    pub data_urls_enabled: bool,
    /// Successful embeds allowed for this options value. Default: 50.
    pub max_images: u32,
    /// Decoded or fetched byte cap. Default: 5 MiB.
    pub max_image_bytes: usize,
    /// Total remote fetch budget in milliseconds, including the body. Default: 10_000.
    pub fetch_timeout_ms: u64,
    /// Redirects to follow. Default: 3.
    pub max_redirects: u32,
    /// Maximum remote URL length. Default: 2048.
    pub max_url_length: usize,
    /// Absolute directory for relative Markdown paths.
    pub base_directory: Option<PathBuf>,
    /// Optional trusted resolver. `Ok(None)` falls through.
    pub resolve: Option<ImageResolver>,
    /// Blank alt text policy. Default: permissive.
    pub missing_image_alt_text: MissingImageAltText,
    embedded_images: AtomicU32,
    failed_remote_attempts: AtomicU32,
    /// Test double. Production leaves this empty, so loads use the pinned HTTPS client.
    transport: Option<RemoteTransport>,
}

impl Clone for ImageOptions {
    /// Clone the configuration with a fresh per-document image budget.
    fn clone(&self) -> Self {
        Self {
            remote_enabled: self.remote_enabled,
            allowed_hosts: self.allowed_hosts.clone(),
            data_urls_enabled: self.data_urls_enabled,
            max_images: self.max_images,
            max_image_bytes: self.max_image_bytes,
            fetch_timeout_ms: self.fetch_timeout_ms,
            max_redirects: self.max_redirects,
            max_url_length: self.max_url_length,
            base_directory: self.base_directory.clone(),
            resolve: self.resolve.clone(),
            missing_image_alt_text: self.missing_image_alt_text,
            transport: self.transport.clone(),
            embedded_images: AtomicU32::new(0),
            failed_remote_attempts: AtomicU32::new(0),
        }
    }
}

impl PartialEq for ImageOptions {
    fn eq(&self, other: &Self) -> bool {
        self.remote_enabled == other.remote_enabled
            && self.allowed_hosts == other.allowed_hosts
            && self.data_urls_enabled == other.data_urls_enabled
            && self.max_images == other.max_images
            && self.max_image_bytes == other.max_image_bytes
            && self.fetch_timeout_ms == other.fetch_timeout_ms
            && self.max_redirects == other.max_redirects
            && self.max_url_length == other.max_url_length
            && self.base_directory == other.base_directory
            && self.missing_image_alt_text == other.missing_image_alt_text
            && match (&self.resolve, &other.resolve) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.transport, &other.transport) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            remote_enabled: false,
            allowed_hosts: None,
            data_urls_enabled: true,
            max_images: 50,
            max_image_bytes: 5 * 1024 * 1024,
            fetch_timeout_ms: 10_000,
            max_redirects: 3,
            max_url_length: 2048,
            base_directory: None,
            resolve: None,
            missing_image_alt_text: MissingImageAltText::Permissive,
            embedded_images: AtomicU32::new(0),
            failed_remote_attempts: AtomicU32::new(0),
            transport: None,
        }
    }
}

impl std::fmt::Debug for ImageOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageOptions")
            .field("remote_enabled", &self.remote_enabled)
            .field("allowed_hosts", &self.allowed_hosts)
            .field("data_urls_enabled", &self.data_urls_enabled)
            .field("max_images", &self.max_images)
            .field("max_image_bytes", &self.max_image_bytes)
            .field("fetch_timeout_ms", &self.fetch_timeout_ms)
            .field("max_redirects", &self.max_redirects)
            .field("max_url_length", &self.max_url_length)
            .field("base_directory", &self.base_directory)
            .field("missing_image_alt_text", &self.missing_image_alt_text)
            .field(
                "embedded_images",
                &self.embedded_images.load(Ordering::SeqCst),
            )
            .field(
                "failed_remote_attempts",
                &self.failed_remote_attempts.load(Ordering::SeqCst),
            )
            .finish()
    }
}

/// Raster bytes ready for a Word drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedImage {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub width_px: u32,
    pub height_px: u32,
}

type RemoteTransport =
    Arc<dyn Fn(&str) -> Result<RemoteReply, MarkdownConversionError> + Send + Sync>;

struct RemoteReply {
    status: u16,
    reason: String,
    location: Option<String>,
    content_type: String,
    content_length: Option<u64>,
    body: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RasterKind {
    Png,
    Jpeg,
    Gif,
}

impl RasterKind {
    fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
        }
    }
}

/// Load `source` using the same rules as the TypeScript image pipeline.
///
/// `source` is the Markdown destination, including an optional `#` fragment.
/// Alt text and title are checked before a resolver, a local read, or a fetch.
pub fn load_markdown_image(
    source: &str,
    alt: &str,
    title: Option<&str>,
    options: &ImageOptions,
) -> Result<LoadedImage, MarkdownConversionError> {
    validate_image_text(alt, title)?;
    assert_alt_policy(alt, options)?;

    let source = strip_fragment(source);
    let remote_attempt = !is_data_url(source);
    if options.embedded_images.load(Ordering::SeqCst) >= options.max_images {
        return Err(conversion_error("Image budget reached (embed limit)"));
    }
    if remote_attempt && options.failed_remote_attempts.load(Ordering::SeqCst) >= options.max_images
    {
        return Err(conversion_error("Image budget reached"));
    }

    match load_image_bytes(source, options) {
        Ok(image) => {
            options.embedded_images.fetch_add(1, Ordering::SeqCst);
            Ok(image)
        }
        Err(error) => {
            if remote_attempt {
                options
                    .failed_remote_attempts
                    .fetch_add(1, Ordering::SeqCst);
            }
            Err(error)
        }
    }
}

fn load_image_bytes(
    source: &str,
    options: &ImageOptions,
) -> Result<LoadedImage, MarkdownConversionError> {
    let mut hint = String::new();
    let bytes = if let Some(resolved) = call_resolve(source, options)? {
        hint = resolved.content_type.unwrap_or_default();
        resolved.bytes
    } else if is_local_candidate(source, options) {
        read_local_asset(source, options)?
    } else if is_data_url(source) {
        let (decoded, content_type) = decode_data_url(source, options)?;
        hint = content_type;
        decoded
    } else {
        let (fetched, content_type) = fetch_remote_image(source, options)?;
        hint = content_type;
        fetched
    };

    if bytes.is_empty() {
        return Err(conversion_error("Invalid image data: data length is 0"));
    }
    let normalized = normalize_image(&bytes, options.max_image_bytes)?;
    finish_image(&normalized, &hint, source)
}

fn call_resolve(
    source: &str,
    options: &ImageOptions,
) -> Result<Option<ResolvedImage>, MarkdownConversionError> {
    let Some(resolve) = &options.resolve else {
        return Ok(None);
    };
    let context = ImageResolveContext {
        max_bytes: options.max_image_bytes,
    };
    resolve(source, &context)
}

fn finish_image(
    bytes: &[u8],
    hint: &str,
    source: &str,
) -> Result<LoadedImage, MarkdownConversionError> {
    if bytes.is_empty() {
        return Err(conversion_error("Invalid image data: data length is 0"));
    }
    let kind = detect_image_type(bytes).ok_or_else(|| unsupported_image(hint, source))?;
    // Like `readIntrinsicDimensions`, unknown dimensions are not an error:
    // the writer falls back to a 200 pixel wide drawing. Zero means unknown.
    let (width, height) = read_dimensions(kind, bytes).unwrap_or((0, 0));
    Ok(LoadedImage {
        bytes: bytes.to_vec(),
        content_type: kind.mime().to_string(),
        width_px: width,
        height_px: height,
    })
}

fn unsupported_image(hint: &str, source: &str) -> MarkdownConversionError {
    let label = if hint.is_empty() { source } else { hint };
    conversion_error(format!("Unsupported or invalid image data ({label})"))
}

fn normalize_image(data: &[u8], max_bytes: usize) -> Result<Vec<u8>, MarkdownConversionError> {
    if data.len() > max_bytes {
        return Err(conversion_error("Image exceeds maximum size"));
    }
    if looks_like_svg(data) {
        validate_svg(data)?;
        let png = rasterize_svg(data)?;
        if png.len() > max_bytes {
            return Err(conversion_error("Converted image exceeds maximum size"));
        }
        return Ok(png);
    }
    if looks_like_webp(data) {
        let png = rasterize_webp(data)?;
        if png.len() > max_bytes {
            return Err(conversion_error("Converted image exceeds maximum size"));
        }
        return Ok(png);
    }
    Ok(data.to_vec())
}

fn looks_like_webp(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"
}

fn looks_like_svg(data: &[u8]) -> bool {
    let prefix_len = data.len().min(512);
    let prefix = String::from_utf8_lossy(&data[..prefix_len]);
    let mut rest = trim_start_ws(&prefix);
    if let Some(after_decl) = rest.strip_prefix("<?xml") {
        let Some(end) = after_decl.find('>') else {
            return false;
        };
        rest = trim_start_ws(&after_decl[end + 1..]);
    }
    if let Some(after_comment) = rest.strip_prefix("<!--") {
        let Some(end) = after_comment.find("-->") else {
            return false;
        };
        rest = trim_start_ws(&after_comment[end + 3..]);
    }
    let lower = rest.to_ascii_lowercase();
    lower.starts_with("<svg>")
        || lower.starts_with("<svg ")
        || lower.starts_with("<svg\t")
        || lower.starts_with("<svg\n")
        || lower.starts_with("<svg\r")
}

fn trim_start_ws(value: &str) -> &str {
    value.trim_start_matches(|ch: char| ch.is_whitespace())
}

fn validate_svg(data: &[u8]) -> Result<(), MarkdownConversionError> {
    if contains_ascii_ci(data, b"<!DOCTYPE")
        || contains_ascii_ci(data, b"<!ENTITY")
        || contains_ascii_ci(data, b"<?xml-stylesheet")
    {
        return Err(conversion_error(
            "SVG declarations and external stylesheets are not supported",
        ));
    }
    let text = std::str::from_utf8(data).map_err(|_| conversion_error("Invalid SVG"))?;
    let mut reader = Reader::from_str(text);
    let mut saw_root = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element) | Event::Empty(element)) => {
                check_svg_element(&element, &mut saw_root)?;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(conversion_error("Invalid SVG")),
        }
    }
    if !saw_root {
        return Err(conversion_error("Invalid SVG root"));
    }
    Ok(())
}

fn check_svg_element(
    element: &BytesStart<'_>,
    saw_root: &mut bool,
) -> Result<(), MarkdownConversionError> {
    let local = element.local_name().as_ref().to_string();
    if !*saw_root {
        *saw_root = true;
        if local != "svg" {
            return Err(conversion_error("Invalid SVG root"));
        }
    }
    if is_active_svg_element(&local) {
        return Err(conversion_error("Active SVG content is not supported"));
    }
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| conversion_error("Invalid SVG"))?;
        let name = attribute.key.as_ref().to_string();
        let local_attr = attribute.key.local_name().as_ref().to_string();
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| conversion_error("Invalid SVG"))?;
        if name == "xml:base"
            || value.contains('\\')
            || contains_ascii_ci(value.as_bytes(), b"@import")
            || starts_with_on(&name)
            || starts_with_on(&local_attr)
            || contains_url_call(&value)
            || ((is_href_name(&name) || is_href_name(&local_attr)) && !value.starts_with('#'))
        {
            return Err(conversion_error(
                "External or active SVG references are not supported",
            ));
        }
    }
    Ok(())
}

fn is_active_svg_element(local_name: &str) -> bool {
    matches!(
        local_name.to_ascii_lowercase().as_str(),
        "script"
            | "foreignobject"
            | "style"
            | "animate"
            | "animatetransform"
            | "animatemotion"
            | "set"
    )
}

fn starts_with_on(name: &str) -> bool {
    name.len() >= 2 && name.as_bytes()[..2].eq_ignore_ascii_case(b"on")
}

fn is_href_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "href" || lower.ends_with(":href")
}

fn contains_url_call(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if &bytes[index..index + 3] == b"url" {
            let mut cursor = index + 3;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'(' {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn rasterize_svg(data: &[u8]) -> Result<Vec<u8>, MarkdownConversionError> {
    let options = usvg::Options::default();
    let tree =
        usvg::Tree::from_data(data, &options).map_err(|_| conversion_error("Invalid SVG"))?;
    let width = pixel_length(tree.size().width())?;
    let height = pixel_length(tree.size().height())?;
    ensure_pixel_budget(width, height)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| conversion_error("Image exceeds pixel limit"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|_| conversion_error("Image conversion failed"))
}

fn rasterize_webp(data: &[u8]) -> Result<Vec<u8>, MarkdownConversionError> {
    let (width, height) = webp_dimensions(data)?;
    ensure_pixel_budget(width, height)?;
    let decoded = decode_webp(data)?;
    let (width, height) = decoded.dimensions();
    ensure_pixel_budget(width, height)?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| conversion_error("Image conversion failed"))?;
    Ok(png)
}

fn webp_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_000_000);
    limits.max_image_height = Some(16_000_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits
}

fn webp_reader(data: &[u8]) -> image::ImageReader<std::io::Cursor<&[u8]>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data));
    reader.set_format(image::ImageFormat::WebP);
    reader.limits(webp_limits());
    reader
}

fn webp_dimensions(data: &[u8]) -> Result<(u32, u32), MarkdownConversionError> {
    webp_reader(data)
        .into_dimensions()
        .map_err(|_| conversion_error("Unsupported or invalid image data (image/webp)"))
}

fn decode_webp(data: &[u8]) -> Result<image::DynamicImage, MarkdownConversionError> {
    webp_reader(data)
        .decode()
        .map_err(|_| conversion_error("Unsupported or invalid image data (image/webp)"))
}

fn pixel_length(value: f32) -> Result<u32, MarkdownConversionError> {
    if !value.is_finite() || value <= 0.0 || value.ceil() > u32::MAX as f32 {
        return Err(conversion_error("Image exceeds pixel limit"));
    }
    let px = value.ceil() as u32;
    if px == 0 {
        return Err(conversion_error("Image exceeds pixel limit"));
    }
    Ok(px)
}

fn ensure_pixel_budget(width: u32, height: u32) -> Result<(), MarkdownConversionError> {
    if u64::from(width).saturating_mul(u64::from(height)) > MAX_RASTER_PIXELS {
        return Err(conversion_error("Image exceeds pixel limit"));
    }
    Ok(())
}

fn detect_image_type(bytes: &[u8]) -> Option<RasterKind> {
    if bytes.len() >= 8
        && bytes[0] == 0x89
        && bytes[1] == b'P'
        && bytes[2] == b'N'
        && bytes[3] == b'G'
        && bytes[4] == 0x0d
        && bytes[5] == 0x0a
        && bytes[6] == 0x1a
        && bytes[7] == 0x0a
    {
        return Some(RasterKind::Png);
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Some(RasterKind::Jpeg);
    }
    if bytes.len() >= 6
        && ((bytes[0] == b'G'
            && bytes[1] == b'I'
            && bytes[2] == b'F'
            && bytes[3] == b'8'
            && bytes[4] == b'7'
            && bytes[5] == b'a')
            || (bytes[0] == b'G'
                && bytes[1] == b'I'
                && bytes[2] == b'F'
                && bytes[3] == b'8'
                && bytes[4] == b'9'
                && bytes[5] == b'a'))
    {
        return Some(RasterKind::Gif);
    }
    None
}

fn read_dimensions(kind: RasterKind, bytes: &[u8]) -> Option<(u32, u32)> {
    match kind {
        RasterKind::Png => read_png_dimensions(bytes),
        RasterKind::Jpeg => read_jpeg_dimensions(bytes),
        RasterKind::Gif => read_gif_dimensions(bytes),
    }
}

fn read_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24
        || bytes[0] != 0x89
        || bytes[1] != b'P'
        || bytes[2] != b'N'
        || bytes[3] != b'G'
    {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn read_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2usize;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            break;
        }
        let marker = bytes[offset + 1];
        let length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
        if marker == 0xc0 || marker == 0xc2 {
            let height = u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]));
            return Some((width, height));
        }
        if length < 2 {
            break;
        }
        offset = offset.saturating_add(2).saturating_add(length);
    }
    None
}

fn read_gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 {
        return None;
    }
    let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    Some((width, height))
}

fn is_local_candidate(source: &str, options: &ImageOptions) -> bool {
    options.base_directory.is_some() && !has_scheme(source)
}

fn has_scheme(source: &str) -> bool {
    let bytes = source.as_bytes();
    if bytes.first().is_none_or(|byte| !byte.is_ascii_alphabetic()) {
        return false;
    }
    for byte in &bytes[1..] {
        if *byte == b':' {
            return true;
        }
        if !byte.is_ascii_alphanumeric() && *byte != b'+' && *byte != b'.' && *byte != b'-' {
            return false;
        }
    }
    false
}

fn read_local_asset(
    source: &str,
    options: &ImageOptions,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let Some(base) = options.base_directory.as_deref() else {
        return Err(conversion_error(
            "Local image baseDirectory must be absolute",
        ));
    };
    if !base.is_absolute() {
        return Err(conversion_error(
            "Local image baseDirectory must be absolute",
        ));
    }
    if has_scheme(source)
        || source.starts_with("//")
        || source.contains('\\')
        || source.contains('\0')
    {
        return Err(conversion_error(
            "Local images must use relative file paths",
        ));
    }
    let decoded = decode_uri_component(source)
        .map_err(|_| conversion_error("Local images must use relative file paths"))?;
    if decoded.contains('\0') {
        return Err(conversion_error(
            "Local images must use relative file paths",
        ));
    }
    let root = base
        .canonicalize()
        .map_err(|_| conversion_error("Local image escapes baseDirectory"))?;
    let candidate = root.join(&decoded);
    let file = candidate.canonicalize().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            conversion_error("Local image not found")
        } else {
            conversion_error("Local image escapes baseDirectory")
        }
    })?;
    if !file.starts_with(&root) {
        return Err(conversion_error("Local image escapes baseDirectory"));
    }
    read_bounded_file(&file, options.max_image_bytes)
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, MarkdownConversionError> {
    let metadata = std::fs::metadata(path).map_err(|_| {
        conversion_error("Local image exceeds maximum size or is not a regular file")
    })?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err(conversion_error(
            "Local image exceeds maximum size or is not a regular file",
        ));
    }
    let mut file = File::open(path).map_err(|_| {
        conversion_error("Local image exceeds maximum size or is not a regular file")
    })?;
    let mut data = Vec::new();
    let mut remaining = max_bytes.saturating_add(1);
    let mut buffer = [0u8; 8192];
    while remaining > 0 {
        let chunk = remaining.min(buffer.len());
        let read = file
            .read(&mut buffer[..chunk])
            .map_err(|_| conversion_error("Local image exceeds maximum size"))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
        remaining -= read;
    }
    if data.len() > max_bytes {
        return Err(conversion_error("Local image exceeds maximum size"));
    }
    Ok(data)
}

fn is_data_url(source: &str) -> bool {
    source.len() >= 5 && source.as_bytes()[..5].eq_ignore_ascii_case(b"data:")
}

fn decode_data_url(
    source: &str,
    options: &ImageOptions,
) -> Result<(Vec<u8>, String), MarkdownConversionError> {
    if !options.data_urls_enabled {
        return Err(conversion_error("Data URL images are disabled"));
    }
    let (mime, is_base64, payload) = split_data_url(source)?;
    let estimated = if is_base64 {
        let stripped = payload
            .chars()
            .filter(|ch| !ch.is_ascii_whitespace())
            .count();
        stripped.saturating_mul(3) / 4
    } else {
        decode_uri_component(payload)
            .map(|text| utf16_len(&text))
            .unwrap_or(usize::MAX)
    };
    if estimated > options.max_image_bytes {
        return Err(conversion_error("Data URL image exceeds maximum size"));
    }
    let bytes = (|| {
        let decoded = if is_base64 {
            decode_base64(payload).map_err(|message| {
                conversion_error(format!("Failed to decode data URL: {message}"))
            })?
        } else {
            decode_uri_component(payload)
                .map(|text| text.into_bytes())
                .map_err(|message| {
                    conversion_error(format!("Failed to decode data URL: {message}"))
                })?
        };
        if decoded.is_empty() {
            return Err(conversion_error(
                "Failed to decode data URL: Data URL produced empty image data",
            ));
        }
        if decoded.len() > options.max_image_bytes {
            return Err(conversion_error(
                "Failed to decode data URL: Data URL image exceeds maximum size",
            ));
        }
        Ok(decoded)
    })()?;
    Ok((bytes, mime))
}

fn split_data_url(source: &str) -> Result<(String, bool, &str), MarkdownConversionError> {
    let Some(comma) = source.find(',') else {
        return Err(invalid_data_url(source));
    };
    if comma < 5 || !source.as_bytes()[..5].eq_ignore_ascii_case(b"data:") {
        return Err(invalid_data_url(source));
    }
    let header = &source[5..comma];
    let payload = &source[comma + 1..];
    let base64_suffix = ";base64";
    let is_base64 = header.len() >= base64_suffix.len()
        && header[header.len() - base64_suffix.len()..].eq_ignore_ascii_case(base64_suffix);
    let mime = if is_base64 {
        &header[..header.len() - base64_suffix.len()]
    } else {
        header
    };
    if mime.contains(';') || mime.contains(',') {
        return Err(invalid_data_url(source));
    }
    Ok((mime.to_string(), is_base64, payload))
}

fn invalid_data_url(source: &str) -> MarkdownConversionError {
    let preview: String = source.chars().take(100).collect();
    conversion_error(format!("Invalid data URL for image: {preview}..."))
}

fn fetch_remote_image(
    initial: &str,
    options: &ImageOptions,
) -> Result<(Vec<u8>, String), MarkdownConversionError> {
    let deadline = Instant::now() + Duration::from_millis(options.fetch_timeout_ms.max(1));
    let mut current = Url::parse(initial).map_err(|_| conversion_error("Invalid URL"))?;
    let mut redirects = 0u32;
    loop {
        if Instant::now() >= deadline {
            return Err(conversion_error("Remote image fetch timed out"));
        }
        enforce_remote_https_policy(&current, options)?;
        reject_blocked_literal(&current)?;
        let reply = if let Some(transport) = &options.transport {
            transport(current.as_str())?
        } else {
            let ip = resolve_pinned_ip(&current, deadline)?;
            https_get(&current, ip, options.max_image_bytes, deadline)?
        };
        if (300..400).contains(&reply.status) {
            if let Some(location) = reply.location.as_deref() {
                if redirects >= options.max_redirects {
                    return Err(conversion_error("Remote image exceeded redirect limit"));
                }
                current = current
                    .join(location)
                    .map_err(|_| conversion_error("Invalid URL"))?;
                redirects += 1;
                continue;
            }
        }
        if !(200..300).contains(&reply.status) {
            return Err(conversion_error(format!(
                "Failed to fetch image: {} {}",
                reply.status, reply.reason
            )));
        }
        if let Some(length) = reply.content_length {
            if length > options.max_image_bytes as u64 {
                return Err(conversion_error("Remote image exceeds maximum size"));
            }
        }
        if reply.body.len() > options.max_image_bytes {
            return Err(conversion_error("Remote image exceeds maximum size"));
        }
        return Ok((reply.body, reply.content_type));
    }
}

fn enforce_remote_https_policy(
    url: &Url,
    options: &ImageOptions,
) -> Result<(), MarkdownConversionError> {
    if !options.remote_enabled {
        return Err(conversion_error("Remote image fetching is disabled"));
    }
    if url.scheme() != "https" {
        return Err(conversion_error("Remote image URLs must use https"));
    }
    if url.as_str().len() > options.max_url_length {
        return Err(conversion_error("Remote image URL exceeds maximum length"));
    }
    if let Some(allowed_hosts) = &options.allowed_hosts {
        let host = url_hostname(url).to_ascii_lowercase();
        let allowed = allowed_hosts
            .iter()
            .any(|entry| entry.trim().eq_ignore_ascii_case(&host));
        if !allowed {
            return Err(conversion_error("Remote image host is not allowed"));
        }
    }
    Ok(())
}

fn reject_blocked_literal(url: &Url) -> Result<(), MarkdownConversionError> {
    let host = url_hostname(url);
    if host.is_empty() {
        return Err(conversion_error("Remote image host did not resolve"));
    }
    if is_ip_literal(host) && is_blocked_ip_address(host) {
        return Err(conversion_error(
            "Remote image host resolves to a blocked network address",
        ));
    }
    Ok(())
}

fn resolve_pinned_ip(url: &Url, deadline: Instant) -> Result<IpAddr, MarkdownConversionError> {
    let host = url_hostname(url);
    if host.is_empty() {
        return Err(conversion_error("Remote image host did not resolve"));
    }
    if is_ip_literal(host) {
        return host.parse::<IpAddr>().map_err(|_| {
            conversion_error("Remote image host resolves to a blocked network address")
        });
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let remain = deadline.saturating_duration_since(Instant::now());
    if remain.is_zero() {
        return Err(conversion_error("Remote image fetch timed out"));
    }
    let remain_ms = remain.as_millis().max(1);
    let dns_ms = remain_ms.min(50u128.max(remain_ms / 4));
    let addresses = lookup_with_timeout(host, port, Duration::from_millis(dns_ms as u64))?;
    if addresses.is_empty() {
        return Err(conversion_error("Remote image host did not resolve"));
    }
    if addresses
        .iter()
        .any(|address| is_blocked_ip_address(&address.ip().to_string()))
    {
        return Err(conversion_error(
            "Remote image host resolves to a blocked network address",
        ));
    }
    Ok(addresses[0].ip())
}

fn lookup_with_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<Vec<SocketAddr>, MarkdownConversionError> {
    let (sender, receiver) = mpsc::channel();
    let host = host.to_string();
    thread::spawn(move || {
        let resolved = (host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect::<Vec<_>>());
        let _ = sender.send(resolved);
    });
    match receiver.recv_timeout(timeout) {
        Ok(Ok(addresses)) => Ok(addresses),
        Ok(Err(_)) => Err(conversion_error("Remote image host did not resolve")),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(conversion_error("DNS lookup timed out")),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(conversion_error("Remote image host did not resolve"))
        }
    }
}

/// Refresh the timeout before every socket operation, including TLS handshakes.
/// A server sending occasional bytes must not extend the total request budget.
struct DeadlineSocket {
    inner: TcpStream,
    deadline: Instant,
}
impl DeadlineSocket {
    fn remaining(&self) -> std::io::Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Remote image fetch timed out",
            ))
        } else {
            Ok(remaining)
        }
    }
}
impl Read for DeadlineSocket {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.set_read_timeout(Some(self.remaining()?))?;
        self.inner.read(buffer)
    }
}
impl Write for DeadlineSocket {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.set_write_timeout(Some(self.remaining()?))?;
        self.inner.write(buffer)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.remaining()?;
        self.inner.flush()
    }
}

fn https_get(
    url: &Url,
    ip: IpAddr,
    max_bytes: usize,
    deadline: Instant,
) -> Result<RemoteReply, MarkdownConversionError> {
    let remain = deadline.saturating_duration_since(Instant::now());
    if remain.is_zero() {
        return Err(conversion_error("Remote image fetch timed out"));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let address = SocketAddr::new(ip, port);
    let tcp = TcpStream::connect_timeout(&address, remain).map_err(map_io_error)?;
    tcp.set_read_timeout(Some(remain)).map_err(map_io_error)?;
    tcp.set_write_timeout(Some(remain)).map_err(map_io_error)?;
    let host = url_hostname(url);
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|_| conversion_error("Failed to fetch image: invalid server name"))?;
    let mut connection = rustls::ClientConnection::new(tls_config()?, server_name)
        .map_err(|error| conversion_error(format!("Failed to fetch image: {error}")))?;
    let mut socket = DeadlineSocket {
        inner: tcp,
        deadline,
    };
    let mut tls = rustls::Stream::new(&mut connection, &mut socket);
    let request = http_request(url);
    tls.write_all(request.as_bytes()).map_err(map_io_error)?;
    read_http_response(&mut tls, max_bytes)
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, MarkdownConversionError> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    if let Some(config) = CONFIG.get() {
        return Ok(Arc::clone(config));
    }
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| conversion_error(format!("Failed to fetch image: {error}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::clone(CONFIG.get_or_init(|| Arc::new(config))))
}

fn http_request(url: &Url) -> String {
    let host = url_hostname(url);
    let port = url.port_or_known_default().unwrap_or(443);
    let host_header = if host.contains(':') {
        if port == 443 {
            format!("[{host}]")
        } else {
            format!("[{host}]:{port}")
        }
    } else if port == 443 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let query = url
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    format!(
        "GET {path}{query} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: md-to-docx\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    )
}

fn read_http_response(
    stream: &mut dyn Read,
    max_bytes: usize,
) -> Result<RemoteReply, MarkdownConversionError> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if buffer.len() > 64 * 1024 {
            return Err(conversion_error(
                "Failed to fetch image: response headers too large",
            ));
        }
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            return Err(conversion_error("Failed to fetch image: connection closed"));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position;
        }
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let already = buffer[header_end + 4..].to_vec();
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| conversion_error("Failed to fetch image: missing status line"))?;
    let (status, reason) = parse_status_line(status_line)?;
    let mut content_type = String::new();
    let mut content_length = None;
    let mut location = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-type") && content_type.is_empty() {
            content_type = value.to_string();
        } else if name.eq_ignore_ascii_case("content-length") && content_length.is_none() {
            content_length = parse_content_length(value);
        } else if name.eq_ignore_ascii_case("location") && location.is_none() {
            location = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    if (300..400).contains(&status) && location.is_some() {
        return Ok(RemoteReply {
            status,
            reason,
            location,
            content_type,
            content_length,
            body: Vec::new(),
        });
    }
    if !(200..300).contains(&status) {
        return Ok(RemoteReply {
            status,
            reason,
            location,
            content_type,
            content_length,
            body: Vec::new(),
        });
    }
    if let Some(length) = content_length {
        if length > max_bytes as u64 {
            return Err(conversion_error("Remote image exceeds maximum size"));
        }
    }
    let body = if chunked {
        read_chunked(&already, stream, max_bytes)?
    } else if let Some(length) = content_length {
        read_exact_body(&already, stream, length as usize, max_bytes)?
    } else {
        read_capped_body(&already, stream, max_bytes)?
    };
    Ok(RemoteReply {
        status,
        reason,
        location,
        content_type,
        content_length,
        body,
    })
}

fn parse_status_line(line: &str) -> Result<(u16, String), MarkdownConversionError> {
    let mut parts = line.splitn(3, ' ');
    let _version = parts
        .next()
        .ok_or_else(|| conversion_error("Failed to fetch image: missing status line"))?;
    let code = parts
        .next()
        .ok_or_else(|| conversion_error("Failed to fetch image: missing status line"))?;
    let status = code
        .parse::<u16>()
        .map_err(|_| conversion_error("Failed to fetch image: invalid status line"))?;
    let reason = parts.next().unwrap_or("").to_string();
    Ok((status, reason))
}

fn parse_content_length(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn read_exact_body(
    initial: &[u8],
    stream: &mut dyn Read,
    exact: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, MarkdownConversionError> {
    if exact > max_bytes {
        return Err(conversion_error("Remote image exceeds maximum size"));
    }
    let mut body = Vec::new();
    if initial.len() >= exact {
        body.extend_from_slice(&initial[..exact]);
        return Ok(body);
    }
    body.extend_from_slice(initial);
    let mut chunk = [0u8; 8192];
    while body.len() < exact {
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            break;
        }
        let need = exact - body.len();
        body.extend_from_slice(&chunk[..read.min(need)]);
    }
    Ok(body)
}

fn read_capped_body(
    initial: &[u8],
    stream: &mut dyn Read,
    max_bytes: usize,
) -> Result<Vec<u8>, MarkdownConversionError> {
    if initial.len() > max_bytes {
        return Err(conversion_error("Remote image exceeds maximum size"));
    }
    let mut body = initial.to_vec();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            return Ok(body);
        }
        if body.len().saturating_add(read) > max_bytes {
            return Err(conversion_error("Remote image exceeds maximum size"));
        }
        body.extend_from_slice(&chunk[..read]);
    }
}

fn read_chunked(
    initial: &[u8],
    stream: &mut dyn Read,
    max_bytes: usize,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut pending = initial.to_vec();
    let mut body = Vec::new();
    loop {
        let line = read_line(&mut pending, stream)?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| conversion_error("Failed to fetch image: invalid chunk size"))?;
        if size == 0 {
            return Ok(body);
        }
        if body.len().saturating_add(size) > max_bytes {
            return Err(conversion_error("Remote image exceeds maximum size"));
        }
        let chunk = read_exact_pending(&mut pending, stream, size)?;
        body.extend_from_slice(&chunk);
        let trailer = read_exact_pending(&mut pending, stream, 2)?;
        if trailer.as_slice() != b"\r\n" {
            return Err(conversion_error("Failed to fetch image: invalid chunk"));
        }
    }
}

fn read_line(
    pending: &mut Vec<u8>,
    stream: &mut dyn Read,
) -> Result<String, MarkdownConversionError> {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(position) = find_subsequence(pending, b"\n") {
            let line: Vec<u8> = pending.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line);
            return Ok(text.trim_end_matches(['\r', '\n']).to_string());
        }
        if pending.len() > 8192 {
            return Err(conversion_error(
                "Failed to fetch image: invalid chunk size",
            ));
        }
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            return Err(conversion_error("Failed to fetch image: connection closed"));
        }
        pending.extend_from_slice(&chunk[..read]);
    }
}

fn read_exact_pending(
    pending: &mut Vec<u8>,
    stream: &mut dyn Read,
    count: usize,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let mut chunk = [0u8; 8192];
    while pending.len() < count {
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            return Err(conversion_error("Failed to fetch image: connection closed"));
        }
        pending.extend_from_slice(&chunk[..read]);
    }
    Ok(pending.drain(..count).collect())
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn map_io_error(error: std::io::Error) -> MarkdownConversionError {
    if error.kind() == std::io::ErrorKind::TimedOut
        || error.kind() == std::io::ErrorKind::WouldBlock
    {
        conversion_error("Remote image fetch timed out")
    } else {
        conversion_error(format!("Failed to fetch image: {error}"))
    }
}

fn is_ip_literal(host: &str) -> bool {
    let host = bare_host(host);
    host.contains(':') || ipv4_parts(host).is_some() || is_numeric_dotted_quad(host)
}

fn is_numeric_dotted_quad(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn is_blocked_ip_address(address: &str) -> bool {
    let address = bare_host(address).to_ascii_lowercase();
    if let Some(parts) = ipv4_parts(&address) {
        return is_private_ipv4(parts);
    }
    if address.contains(':') {
        return is_private_ipv6(&address);
    }
    true
}

/// Hostname the way `URL.hostname` reports it: no brackets, no zone id.
fn url_hostname(url: &Url) -> &str {
    bare_host(url.host_str().unwrap_or(""))
}

fn bare_host(host: &str) -> &str {
    let host = host.trim();
    let inner = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host
    };
    inner.split('%').next().unwrap_or(inner)
}

fn ipv4_parts(address: &str) -> Option<[u32; 4]> {
    let mut parts = [0u32; 4];
    let mut count = 0usize;
    for part in address.split('.') {
        if count >= 4
            || part.is_empty()
            || part.len() > 3
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let value: u32 = part.parse().ok()?;
        if value > 255 {
            return None;
        }
        parts[count] = value;
        count += 1;
    }
    if count == 4 {
        Some(parts)
    } else {
        None
    }
}

fn is_private_ipv4(parts: [u32; 4]) -> bool {
    let [first, second, _, _] = parts;
    first == 0
        || first == 10
        || first == 127
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 168)
        || (first == 198 && (second == 18 || second == 19))
        || first >= 224
}

fn is_private_ipv6(address: &str) -> bool {
    let normalized = bare_host(address).to_ascii_lowercase();
    if let Some(mapped) = ipv4_embedded_in_mapped(&normalized) {
        return match ipv4_parts(&mapped) {
            Some(parts) => is_private_ipv4(parts),
            None => true,
        };
    }
    normalized == "::"
        || normalized == "::1"
        || normalized.starts_with("fe80:")
        || normalized.starts_with("fc")
        || normalized.starts_with("fd")
        || normalized.starts_with("ff")
}

fn ipv4_embedded_in_mapped(address: &str) -> Option<String> {
    if let Some(rest) = address.strip_prefix("::ffff:") {
        if ipv4_parts(rest).is_some() {
            return Some(rest.to_string());
        }
        let mut groups = rest.split(':');
        let high = groups.next()?;
        let low = groups.next()?;
        if groups.next().is_some() {
            return None;
        }
        if is_hex_group(high) && is_hex_group(low) {
            return hextets_to_ipv4(high, low);
        }
        return None;
    }
    let parts: Vec<&str> = address.split(':').collect();
    if parts.len() == 8
        && parts[0] == "0"
        && parts[1] == "0"
        && parts[2] == "0"
        && parts[3] == "0"
        && parts[4] == "0"
        && parts[5] == "ffff"
        && is_hex_group(parts[6])
        && is_hex_group(parts[7])
    {
        return hextets_to_ipv4(parts[6], parts[7]);
    }
    None
}

fn is_hex_group(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hextets_to_ipv4(high_hex: &str, low_hex: &str) -> Option<String> {
    let high = u32::from_str_radix(high_hex, 16).ok()?;
    let low = u32::from_str_radix(low_hex, 16).ok()?;
    if high > 65535 || low > 65535 {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        (high >> 8) & 255,
        high & 255,
        (low >> 8) & 255,
        low & 255
    ))
}

fn validate_image_text(alt: &str, title: Option<&str>) -> Result<(), MarkdownConversionError> {
    validate_ooxml_text(alt, "Markdown image alt text", MAX_IMAGE_ALT_TEXT_LENGTH)?;
    if let Some(title) = title {
        validate_ooxml_text(title, "Markdown image title", MAX_IMAGE_TITLE_LENGTH)?;
    }
    Ok(())
}

fn validate_ooxml_text(
    value: &str,
    context: &str,
    max_length: usize,
) -> Result<(), MarkdownConversionError> {
    let length = utf16_len(value);
    if length > max_length {
        return Err(with_context(
            format!("{context} exceeds the maximum length of {max_length} characters"),
            &[
                ("context", context),
                ("length", &length.to_string()),
                ("maxLength", &max_length.to_string()),
            ],
        ));
    }
    if has_illegal_ooxml_char(value) {
        return Err(with_context(
            format!("{context} contains characters that are not valid in OOXML"),
            &[("context", context)],
        ));
    }
    Ok(())
}

fn assert_alt_policy(alt: &str, options: &ImageOptions) -> Result<(), MarkdownConversionError> {
    if options.missing_image_alt_text == MissingImageAltText::Throw && alt.trim().is_empty() {
        return Err(with_context(
            "Missing image alt text in Markdown image; strict accessibility mode requires a non-empty description".to_string(),
            &[
                ("context", "Markdown image"),
                ("missingImageAltText", "throw"),
            ],
        ));
    }
    Ok(())
}

fn has_illegal_ooxml_char(value: &str) -> bool {
    value.chars().any(|ch| {
        let code = ch as u32;
        code <= 0x08
            || code == 0x0b
            || code == 0x0c
            || (0x0e..=0x1f).contains(&code)
            || code == 0xfffe
            || code == 0xffff
    })
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn strip_fragment(source: &str) -> &str {
    match source.find('#') {
        Some(index) => &source[..index],
        None => source,
    }
}

fn decode_uri_component(input: &str) -> Result<String, String> {
    let bytes = percent_decode_bytes(input)?;
    String::from_utf8(bytes).map_err(|_| "invalid UTF-8".to_string())
}

fn percent_decode_bytes(input: &str) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("invalid percent encoding".to_string());
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Ok(output)
}

fn hex_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid percent encoding".to_string()),
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut cleaned = String::new();
    for ch in input.chars() {
        if !ch.is_ascii_whitespace() {
            cleaned.push(ch);
        }
    }
    let remainder = cleaned.len() % 4;
    if remainder == 1 {
        return Err("invalid base64 length".to_string());
    }
    if remainder != 0 {
        cleaned.extend(std::iter::repeat_n('=', 4 - remainder));
    }
    let bytes = cleaned.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let values = [
            base64_value(chunk[0])?,
            base64_value(chunk[1])?,
            base64_value(chunk[2])?,
            base64_value(chunk[3])?,
        ];
        output.push((values[0] << 2) | (values[1] >> 4));
        if chunk[2] != b'=' {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if chunk[3] != b'=' {
            output.push((values[2] << 6) | values[3]);
        }
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        b'=' => Ok(0),
        _ => Err("invalid base64 character".to_string()),
    }
}

fn contains_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn conversion_error(message: impl Into<String>) -> MarkdownConversionError {
    MarkdownConversionError::new(message)
}

fn with_context(message: String, pairs: &[(&str, &str)]) -> MarkdownConversionError {
    let mut context = BTreeMap::new();
    for (key, value) in pairs {
        context.insert((*key).to_string(), (*value).to_string());
    }
    MarkdownConversionError::with_context(message, context)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    const ONE_PX_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAGgwJ/vk9yBgAAAABJRU5ErkJggg==";
    const ONE_PX_GIF_B64: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    const TINY_JPEG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        0xFF, 0xD9,
    ];

    fn one_px_png() -> Vec<u8> {
        decode_base64(ONE_PX_PNG_B64).unwrap()
    }

    fn data_png() -> String {
        format!("data:image/png;base64,{ONE_PX_PNG_B64}")
    }

    fn message(error: &MarkdownConversionError) -> &str {
        error.message()
    }

    fn remote_options() -> ImageOptions {
        ImageOptions {
            remote_enabled: true,
            ..Default::default()
        }
    }

    fn panic_transport() -> RemoteTransport {
        Arc::new(|_| panic!("unit test fetched a remote image"))
    }

    // Jest: "rejects remote images by default"
    #[test]
    fn remote_images_are_off_by_default() {
        let options = ImageOptions {
            transport: Some(panic_transport()),
            ..Default::default()
        };
        let error = load_markdown_image("https://example.com/image.png", "remote", None, &options)
            .unwrap_err();
        assert!(message(&error).contains("Remote image fetching is disabled"));
        assert_eq!(options.failed_remote_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(options.embedded_images.load(Ordering::SeqCst), 0);
    }

    // Jest: "rejects http URLs even when remote image fetching is enabled"
    #[test]
    fn http_urls_are_rejected_before_fetch() {
        let mut options = remote_options();
        options.transport = Some(panic_transport());
        let error = load_markdown_image("http://93.184.216.34/image.png", "remote", None, &options)
            .unwrap_err();
        assert!(message(&error).contains("Remote image URLs must use https"));
    }

    // Jest: "rejects loopback and private-network addresses before fetch"
    #[test]
    fn loopback_and_private_literals_are_blocked() {
        let mut options = remote_options();
        options.transport = Some(panic_transport());
        for source in [
            "https://127.0.0.1/image.png",
            "https://10.1.2.3/image.png",
            "https://0.0.0.0/image.png",
            "https://100.64.0.1/image.png",
            "https://169.254.1.1/image.png",
            "https://172.16.0.1/image.png",
            "https://192.168.1.1/image.png",
            "https://198.18.0.1/image.png",
            "https://198.19.0.1/image.png",
            "https://224.0.0.1/image.png",
            "https://[::1]/image.png",
            "https://[::]/image.png",
            "https://[fe80::1]/image.png",
            "https://[fc00::1]/image.png",
            "https://[fd00::1]/image.png",
            "https://[ff02::1]/image.png",
        ] {
            let error = load_markdown_image(source, "remote", None, &options).unwrap_err();
            assert!(
                message(&error).contains("blocked network address"),
                "{source} -> {}",
                message(&error)
            );
        }
    }

    // Jest: "rejects IPv4-mapped IPv6 private addresses before fetch"
    // Jest: "rejects hex IPv4-mapped IPv6 hostnames before fetch"
    // Jest: "rejects uncompressed IPv6-mapped private addresses before fetch"
    #[test]
    fn ipv4_mapped_private_addresses_are_blocked() {
        let cases = [
            "https://[::ffff:192.168.1.10]/image.png",
            "https://[::ffff:c0a8:010a]/image.png",
            "https://[0:0:0:0:0:ffff:c0a8:010a]/image.png",
        ];
        for source in cases {
            let url = Url::parse(source).unwrap();
            let host = url.host_str().unwrap();
            assert!(
                is_blocked_ip_address(host),
                "url host {host} from {source} was not blocked"
            );
            let mut options = remote_options();
            options.transport = Some(panic_transport());
            let error = load_markdown_image(source, "remote", None, &options).unwrap_err();
            assert!(
                message(&error).contains("blocked network address"),
                "{source} host {host} -> {}",
                message(&error)
            );
        }
    }

    #[test]
    fn public_addresses_match_the_typescript_allow_list() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",
            "100.63.0.1",
            "11.0.0.1",
            "192.0.2.1",
            "::ffff:8.8.8.8",
            "2001:db8::1",
        ] {
            assert!(
                !is_blocked_ip_address(address),
                "{address} should be public"
            );
        }
    }

    // Jest: "rejects oversized data URLs before embedding"
    #[test]
    fn oversized_data_urls_are_rejected() {
        let options = ImageOptions {
            max_image_bytes: 16,
            ..Default::default()
        };
        let source = format!("data:image/png;base64,{}", "A".repeat(128));
        let error = load_markdown_image(&source, "too large", None, &options).unwrap_err();
        assert!(message(&error).contains("Data URL image exceeds maximum size"));
        assert_eq!(options.embedded_images.load(Ordering::SeqCst), 0);
        assert_eq!(options.failed_remote_attempts.load(Ordering::SeqCst), 0);
    }

    // Jest: "continues to embed valid data URL images by default"
    #[test]
    fn data_url_png_keeps_original_bytes() {
        let loaded = load_markdown_image(
            &data_png(),
            "one px",
            Some("figure"),
            &ImageOptions::default(),
        )
        .unwrap();
        assert_eq!(loaded.bytes, one_px_png());
        assert_eq!(loaded.content_type, "image/png");
        assert_eq!((loaded.width_px, loaded.height_px), (1, 1));
    }

    #[test]
    fn data_urls_can_be_disabled() {
        let options = ImageOptions {
            data_urls_enabled: false,
            ..Default::default()
        };
        let error = load_markdown_image(&data_png(), "one px", None, &options).unwrap_err();
        assert!(message(&error).contains("Data URL images are disabled"));
    }

    #[test]
    fn gif_and_jpeg_are_not_reencoded() {
        let gif = decode_base64(ONE_PX_GIF_B64).unwrap();
        let source = format!("data:image/gif;base64,{ONE_PX_GIF_B64}");
        let loaded = load_markdown_image(&source, "gif", None, &ImageOptions::default()).unwrap();
        assert_eq!(loaded.bytes, gif);
        assert_eq!(loaded.content_type, "image/gif");
        assert_eq!((loaded.width_px, loaded.height_px), (1, 1));

        let jpeg = TINY_JPEG.to_vec();
        let options = ImageOptions {
            resolve: Some(Arc::new(move |_, _| {
                Ok(Some(ResolvedImage {
                    bytes: jpeg.clone(),
                    content_type: Some("image/jpeg".to_string()),
                }))
            })),
            ..Default::default()
        };
        let loaded = load_markdown_image("picture.jpg", "jpeg", None, &options).unwrap();
        assert_eq!(loaded.bytes, TINY_JPEG);
        assert_eq!(loaded.content_type, "image/jpeg");
        assert_eq!((loaded.width_px, loaded.height_px), (1, 1));
    }

    #[test]
    fn fragment_does_not_change_intrinsic_png_size() {
        let source = format!("{ONE}#100x50", ONE = data_png());
        let loaded =
            load_markdown_image(&source, "one px", None, &ImageOptions::default()).unwrap();
        assert_eq!((loaded.width_px, loaded.height_px), (1, 1));
        assert_eq!(loaded.bytes, one_px_png());
    }

    #[test]
    fn resolve_none_falls_through_to_data_url() {
        let options = ImageOptions {
            resolve: Some(Arc::new(|_, _| Ok(None))),
            ..Default::default()
        };
        let loaded = load_markdown_image(&data_png(), "one px", None, &options).unwrap();
        assert_eq!(loaded.bytes, one_px_png());
    }

    #[test]
    fn resolve_bytes_win_over_remote() {
        let png = one_px_png();
        let mut options = remote_options();
        options.transport = Some(panic_transport());
        options.resolve = Some(Arc::new(move |_, _| {
            Ok(Some(ResolvedImage {
                bytes: png.clone(),
                content_type: Some("image/png".to_string()),
            }))
        }));
        let loaded =
            load_markdown_image("https://example.com/image.png", "alt", None, &options).unwrap();
        assert_eq!(loaded.bytes, one_px_png());
    }

    #[test]
    fn webp_becomes_png_with_same_pixel_size() {
        let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([9, 8, 7, 255]));
        let mut webp = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut webp),
                image::ImageFormat::WebP,
            )
            .unwrap();
        assert!(looks_like_webp(&webp));
        let options = ImageOptions {
            resolve: Some(Arc::new(move |_, _| {
                Ok(Some(ResolvedImage {
                    bytes: webp.clone(),
                    content_type: Some("image/webp".to_string()),
                }))
            })),
            ..Default::default()
        };
        let loaded = load_markdown_image("photo.webp", "webp", None, &options).unwrap();
        assert_eq!(loaded.content_type, "image/png");
        assert!(loaded.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!((loaded.width_px, loaded.height_px), (2, 3));
    }

    #[test]
    fn svg_becomes_png_and_rejects_active_content() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"><rect width="2" height="3" fill="black"/></svg>"#;
        let loaded = load_markdown_image(
            &format!("data:image/svg+xml,{svg}"),
            "svg",
            None,
            &ImageOptions::default(),
        )
        .unwrap();
        assert_eq!(loaded.content_type, "image/png");
        assert!(loaded.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!((loaded.width_px, loaded.height_px), (2, 3));

        let cases = [
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><script>alert(1)</script></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><foreignObject></foreignObject></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style></style></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><animate attributeName="x"/></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><animateTransform attributeName="transform"/></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><animateMotion/></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><set attributeName="x" to="1"/></svg>"#,
                "Active SVG content is not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" onload="alert(1)"></svg>"#,
                "External or active SVG references are not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><a href="https://example.com"><rect width="1" height="1"/></a></svg>"#,
                "External or active SVG references are not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="url(%23grad)"/></svg>"#,
                "External or active SVG references are not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><!DOCTYPE svg></svg>"#,
                "SVG declarations and external stylesheets are not supported",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><!ENTITY x "y"></svg>"#,
                "SVG declarations and external stylesheets are not supported",
            ),
            (
                r#"<?xml-stylesheet type="text/css" href="x"?><svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
                "SVG declarations and external stylesheets are not supported",
            ),
            (
                r#"<SVG xmlns="http://www.w3.org/2000/svg" width="1" height="1"></SVG>"#,
                "Invalid SVG root",
            ),
            (
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="5000" height="5000"></svg>"#,
                "Image exceeds pixel limit",
            ),
        ];
        for (svg, expected) in cases {
            let error = load_markdown_image(
                &format!("data:image/svg+xml,{svg}"),
                "svg",
                None,
                &ImageOptions::default(),
            )
            .unwrap_err();
            assert!(
                message(&error).contains(expected),
                "svg {svg} -> {} (wanted {expected})",
                message(&error)
            );
        }
    }

    #[test]
    fn fragment_href_inside_svg_is_allowed() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect id="a" width="1" height="1" fill="black"/><use href="%23a"/></svg>"##;
        let loaded = load_markdown_image(
            &format!("data:image/svg+xml,{svg}"),
            "svg",
            None,
            &ImageOptions::default(),
        )
        .unwrap();
        assert_eq!(loaded.content_type, "image/png");
        assert_eq!((loaded.width_px, loaded.height_px), (1, 1));
    }

    // Jest: "enforces the maximum image count"
    // Jest: "does not count failed images toward maxImages"
    #[test]
    fn image_budget_counts_successes_and_failed_remote_attempts_separately() {
        let mut options = remote_options();
        options.max_images = 1;
        options.transport = Some(panic_transport());
        let blocked =
            load_markdown_image("https://127.0.0.1/a.png", "blocked", None, &options).unwrap_err();
        assert!(message(&blocked).contains("blocked network address"));
        let loaded = load_markdown_image(&data_png(), "ok", None, &options).unwrap();
        assert_eq!(loaded.width_px, 1);
        assert_eq!(options.embedded_images.load(Ordering::SeqCst), 1);
        assert_eq!(options.failed_remote_attempts.load(Ordering::SeqCst), 1);

        let second = load_markdown_image(&data_png(), "two", None, &options).unwrap_err();
        assert!(message(&second).contains("Image budget reached"));
        assert_eq!(options.embedded_images.load(Ordering::SeqCst), 1);
    }

    // Jest: "stops attempting remote fetches once the failed-image budget is spent"
    #[test]
    fn failed_remote_budget_stops_later_attempts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut options = remote_options();
        options.max_images = 2;
        options.transport = Some(panic_transport());
        for index in 0..2 {
            let error = load_markdown_image(
                &format!("https://10.0.0.{index}/missing.png"),
                "broken",
                None,
                &options,
            )
            .unwrap_err();
            assert!(message(&error).contains("blocked network address"));
        }
        let seen = Arc::clone(&calls);
        options.transport = Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Err(conversion_error("connection refused"))
        }));
        let error = load_markdown_image(
            "https://93.184.216.34/missing.png",
            "broken",
            None,
            &options,
        )
        .unwrap_err();
        assert!(message(&error).contains("Image budget reached"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    // Jest: "denies every remote host when an empty allowedHosts list is provided"
    #[test]
    fn empty_allowed_hosts_denies_every_host() {
        let called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&called);
        let mut options = remote_options();
        options.allowed_hosts = Some(Vec::new());
        options.transport = Some(Arc::new(move |_| {
            flag.store(true, Ordering::SeqCst);
            Err(conversion_error("unexpected fetch"))
        }));
        let error =
            load_markdown_image("https://93.184.216.34/image.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("Remote image host is not allowed"));
        assert!(!called.load(Ordering::SeqCst));
    }

    // Jest: "still fetches hosts present in the allowedHosts list"
    #[test]
    fn allowed_host_uses_transport_without_a_socket() {
        let png = one_px_png();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let body = png.clone();
        let mut options = remote_options();
        options.allowed_hosts = Some(vec!["93.184.216.34".to_string()]);
        options.transport = Some(Arc::new(move |url| {
            seen.fetch_add(1, Ordering::SeqCst);
            assert_eq!(url, "https://93.184.216.34/image.png");
            Ok(RemoteReply {
                status: 200,
                reason: "OK".to_string(),
                location: None,
                content_type: "image/png".to_string(),
                content_length: Some(body.len() as u64),
                body: body.clone(),
            })
        }));
        let loaded =
            load_markdown_image("https://93.184.216.34/image.png", "remote", None, &options)
                .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(loaded.bytes, png);
        assert_eq!(loaded.content_type, "image/png");
    }

    #[test]
    fn allow_list_match_is_case_insensitive() {
        let png = one_px_png();
        let body = png.clone();
        let mut options = remote_options();
        options.allowed_hosts = Some(vec![" Example.COM ".to_string()]);
        options.transport = Some(Arc::new(move |_| {
            Ok(RemoteReply {
                status: 200,
                reason: "OK".to_string(),
                location: None,
                content_type: "image/png".to_string(),
                content_length: Some(body.len() as u64),
                body: body.clone(),
            })
        }));
        let loaded =
            load_markdown_image("https://example.com/a.png", "remote", None, &options).unwrap();
        assert_eq!(loaded.bytes, png);
    }

    // Jest: "re-checks redirected remote image targets"
    #[test]
    fn redirect_to_loopback_is_rejected_without_a_second_fetch() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        let mut options = remote_options();
        options.transport = Some(Arc::new(move |url| {
            seen.lock().unwrap().push(url.to_string());
            Ok(RemoteReply {
                status: 302,
                reason: "Found".to_string(),
                location: Some("https://127.0.0.1/private.png".to_string()),
                content_type: String::new(),
                content_length: None,
                body: Vec::new(),
            })
        }));
        let error =
            load_markdown_image("https://93.184.216.34/image.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("blocked network address"));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["https://93.184.216.34/image.png"]
        );
    }

    #[test]
    fn redirect_to_http_is_rejected() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let mut options = remote_options();
        options.transport = Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(RemoteReply {
                status: 302,
                reason: "Found".to_string(),
                location: Some("http://93.184.216.34/next.png".to_string()),
                content_type: String::new(),
                content_length: None,
                body: Vec::new(),
            })
        }));
        let error =
            load_markdown_image("https://93.184.216.34/start.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("Remote image URLs must use https"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    // Jest: "stops following redirects after maxRedirects"
    #[test]
    fn redirect_limit_matches_three_followed_hops() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let mut options = remote_options();
        options.max_redirects = 3;
        options.transport = Some(Arc::new(move |_| {
            let next = seen.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(RemoteReply {
                status: 302,
                reason: "Found".to_string(),
                location: Some(format!("https://93.184.216.34/step-{next}.png")),
                content_type: String::new(),
                content_length: None,
                body: Vec::new(),
            })
        }));
        let error =
            load_markdown_image("https://93.184.216.34/start.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("Remote image exceeded redirect limit"));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    // Jest: "rejects remote images with oversized content-length"
    #[test]
    fn content_length_over_the_cap_is_rejected() {
        let mut options = remote_options();
        options.max_image_bytes = 16;
        options.transport = Some(Arc::new(|_| {
            Ok(RemoteReply {
                status: 200,
                reason: "OK".to_string(),
                location: None,
                content_type: "image/png".to_string(),
                content_length: Some(128),
                body: Vec::new(),
            })
        }));
        let error =
            load_markdown_image("https://93.184.216.34/image.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("Remote image exceeds maximum size"));
    }

    // Jest: "rejects remote images that exceed the streamed byte cap"
    #[test]
    fn streamed_body_over_the_cap_is_rejected() {
        let mut options = remote_options();
        options.max_image_bytes = 16;
        options.transport = Some(Arc::new(|_| {
            Ok(RemoteReply {
                status: 200,
                reason: "OK".to_string(),
                location: None,
                content_type: "image/png".to_string(),
                content_length: None,
                body: vec![0; 24],
            })
        }));
        let error =
            load_markdown_image("https://93.184.216.34/image.png", "remote", None, &options)
                .unwrap_err();
        assert!(message(&error).contains("Remote image exceeds maximum size"));
    }

    #[test]
    fn long_remote_urls_are_rejected_before_fetch() {
        let mut options = remote_options();
        options.transport = Some(panic_transport());
        let source = format!("https://93.184.216.34/{}", "a".repeat(3000));
        let error = load_markdown_image(&source, "remote", None, &options).unwrap_err();
        assert!(message(&error).contains("Remote image URL exceeds maximum length"));
    }

    #[test]
    fn alt_text_policy_runs_before_fetch() {
        let mut options = remote_options();
        options.missing_image_alt_text = MissingImageAltText::Throw;
        options.transport = Some(panic_transport());
        let error = load_markdown_image("https://example.com/image.png", "   ", None, &options)
            .unwrap_err();
        assert!(message(&error).contains(
            "Missing image alt text in Markdown image; strict accessibility mode requires a non-empty description"
        ));
        assert_eq!(options.failed_remote_attempts.load(Ordering::SeqCst), 0);

        let permissive = remote_options();
        let error = load_markdown_image(&data_png(), "", None, &permissive).unwrap();
        assert_eq!(error.width_px, 1);
    }

    #[test]
    fn alt_and_title_limits_reject_ooxml_illegal_text() {
        let too_long = "a".repeat(2049);
        let error = load_markdown_image(&data_png(), &too_long, None, &ImageOptions::default())
            .unwrap_err();
        assert!(
            message(&error).contains("Markdown image alt text exceeds the maximum length of 2048")
        );

        let emoji = "😀".repeat(1025);
        let error =
            load_markdown_image(&data_png(), &emoji, None, &ImageOptions::default()).unwrap_err();
        assert!(message(&error).contains("maximum length of 2048"));

        let error = load_markdown_image(
            &data_png(),
            "ok",
            Some(&"t".repeat(513)),
            &ImageOptions::default(),
        )
        .unwrap_err();
        assert!(message(&error).contains("Markdown image title exceeds the maximum length of 512"));

        let error = load_markdown_image(&data_png(), "bad\u{0007}", None, &ImageOptions::default())
            .unwrap_err();
        assert!(message(&error).contains("not valid in OOXML"));
        assert_eq!(
            error.context().unwrap().get("context").map(String::as_str),
            Some("Markdown image alt text")
        );
    }

    #[test]
    fn local_files_stay_inside_an_absolute_base_directory() {
        let root = std::env::temp_dir().join(format!(
            "md-to-docx-images-{}-{}",
            std::process::id(),
            unique_id()
        ));
        let base = root.join("base");
        let outside = root.join("outside");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let png = one_px_png();
        std::fs::write(base.join("ok.png"), &png).unwrap();
        std::fs::write(base.join("sub").join("nested.png"), &png).unwrap();
        std::fs::write(outside.join("secret.png"), &png).unwrap();
        std::fs::write(base.join("big.png"), vec![0u8; 64]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.png"), base.join("link.png")).unwrap();

        let mut options = ImageOptions {
            base_directory: Some(base.clone()),
            ..Default::default()
        };
        let loaded = load_markdown_image("ok.png", "local", None, &options).unwrap();
        assert_eq!(loaded.bytes, png);
        let nested = load_markdown_image("sub/../ok.png", "local", None, &options).unwrap();
        assert_eq!(nested.bytes, png);

        let escaped =
            load_markdown_image("../outside/secret.png", "local", None, &options).unwrap_err();
        assert!(message(&escaped).contains("Local image escapes baseDirectory"));

        #[cfg(unix)]
        {
            let link = load_markdown_image("link.png", "local", None, &options).unwrap_err();
            assert!(message(&link).contains("Local image escapes baseDirectory"));
        }

        options.max_image_bytes = 16;
        let big = load_markdown_image("big.png", "local", None, &options).unwrap_err();
        assert!(message(&big).contains("Local image exceeds maximum size"));

        options.max_image_bytes = 5 * 1024 * 1024;
        let directory = load_markdown_image("sub", "local", None, &options).unwrap_err();
        assert!(message(&directory).contains("not a regular file"));

        options.base_directory = Some(PathBuf::from("relative"));
        let relative = load_markdown_image("ok.png", "local", None, &options).unwrap_err();
        assert!(message(&relative).contains("baseDirectory must be absolute"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn unique_id() -> u64 {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        NEXT.fetch_add(1, Ordering::SeqCst) as u64
    }
}
