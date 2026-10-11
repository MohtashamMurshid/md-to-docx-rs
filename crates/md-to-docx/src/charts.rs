//! Built-in chart rasterizer.
//!
//! Draws bar, line, pie, and doughnut charts to PNG bytes. Geometry follows
//! `src/renderers/chartRenderer.ts`. PNG encoding uses `image` 0.25.10
//! [`PngEncoder`](image::codecs::png::PngEncoder), which deflates scanlines
//! in the normal PNG writer. The TypeScript encoder's bit-at-a-time CRC32
//! loop is not used.

use std::collections::BTreeMap;

use image::ImageEncoder;

use crate::error::MarkdownConversionError;

const DEFAULT_MAX_CHART_WIDTH: u32 = 2000;
const DEFAULT_MAX_CHART_HEIGHT: u32 = 2000;
const MAX_JSON_DEPTH: usize = 64;

const DEFAULT_COLORS: [[u8; 3]; 10] = [
    [0x4E, 0x79, 0xA7],
    [0xF2, 0x8E, 0x2B],
    [0xE1, 0x57, 0x59],
    [0x76, 0xB7, 0xB2],
    [0x59, 0xA1, 0x4F],
    [0xED, 0xC9, 0x48],
    [0xB0, 0x7A, 0xA1],
    [0xFF, 0x9D, 0xA7],
    [0x9C, 0x75, 0x5F],
    [0xBA, 0xB0, 0xAC],
];

const AXIS_COLOR: [u8; 3] = [0x4B, 0x55, 0x63];
const GRID_COLOR: [u8; 3] = [0xE5, 0xE7, 0xEB];
const BACKGROUND: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// Rasterize a chart JSON definition to PNG bytes.
///
/// `width_px` and `height_px` are used when the JSON omits that axis. Callers
/// pass 640 and 360 for the built-in defaults. A JSON `width` or `height`
/// replaces the corresponding argument. Either axis must be a positive integer
/// no larger than 2000. Invalid JSON and invalid chart objects return
/// [`MarkdownConversionError`] so the caller can insert a placeholder.
pub fn render_chart(
    definition: &str,
    width_px: u32,
    height_px: u32,
) -> Result<Vec<u8>, MarkdownConversionError> {
    let chart = parse_chart(definition)?;
    let width = resolve_dimension(chart.width, width_px, DEFAULT_MAX_CHART_WIDTH, "width")?;
    let height = resolve_dimension(chart.height, height_px, DEFAULT_MAX_CHART_HEIGHT, "height")?;
    let mut raster = Raster::new(width, height)?;
    raster.clear(BACKGROUND);
    match chart.kind {
        ChartKind::Pie | ChartKind::Doughnut => render_pie(&mut raster, &chart)?,
        ChartKind::Bar => render_bar(&mut raster, &chart),
        ChartKind::Line => render_line(&mut raster, &chart),
    }
    encode_png(raster.width, raster.height, &raster.pixels)
}

#[derive(Clone, Copy)]
enum ChartKind {
    Bar,
    Line,
    Pie,
    Doughnut,
}

#[derive(Clone, Copy)]
enum Dimension {
    Missing,
    Number(f64),
    Invalid,
}

enum ColorField {
    Absent,
    One(String),
    Many(Vec<Option<String>>),
}

struct Dataset {
    data: Vec<f64>,
    background: ColorField,
    border: ColorField,
}

struct ChartDefinition {
    kind: ChartKind,
    datasets: Vec<Dataset>,
    width: Dimension,
    height: Dimension,
}

struct Plot {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

struct Raster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Raster {
    fn new(width: u32, height: u32) -> Result<Self, MarkdownConversionError> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| chart_error("chart exceeds maximum 2000"))?;
        Ok(Self {
            width,
            height,
            pixels: vec![0; len],
        })
    }

    fn clear(&mut self, color: [u8; 3]) {
        let mut index = 0;
        while index < self.pixels.len() {
            self.pixels[index] = color[0];
            self.pixels[index + 1] = color[1];
            self.pixels[index + 2] = color[2];
            self.pixels[index + 3] = 255;
            index += 4;
        }
    }

    fn set_pixel(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[index] = color[0];
        self.pixels[index + 1] = color[1];
        self.pixels[index + 2] = color[2];
        self.pixels[index + 3] = 255;
    }

    fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: [u8; 3]) {
        let x0 = clamp_i32(floor_i32(x), 0, self.width as i32);
        let y0 = clamp_i32(floor_i32(y), 0, self.height as i32);
        let x1 = clamp_i32(ceil_i32(x + width), 0, self.width as i32);
        let y1 = clamp_i32(ceil_i32(y + height), 0, self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                self.set_pixel(px, py, color);
            }
        }
    }

    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: [u8; 3]) {
        let mut x = round_i32(x0);
        let mut y = round_i32(y0);
        let end_x = round_i32(x1);
        let end_y = round_i32(y1);
        let dx = (end_x - x).abs();
        let sx = if x < end_x { 1 } else { -1 };
        let dy = -(end_y - y).abs();
        let sy = if y < end_y { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            self.set_pixel(x, y, color);
            self.set_pixel(x + 1, y, color);
            self.set_pixel(x, y + 1, color);
            if x == end_x && y == end_y {
                break;
            }
            let e2 = 2 * error;
            if e2 >= dy {
                error += dy;
                x += sx;
            }
            if e2 <= dx {
                error += dx;
                y += sy;
            }
        }
    }
}

fn render_bar(raster: &mut Raster, chart: &ChartDefinition) {
    let plot = plot_area(raster.width, raster.height);
    let series_length = chart
        .datasets
        .iter()
        .map(|dataset| dataset.data.len())
        .max()
        .unwrap_or(1);
    let (min, max) = value_range(chart);
    draw_axes(raster, &plot);
    let group_width = plot.width / (series_length.max(1) as f64);
    let bar_width = ((group_width * 0.72) / chart.datasets.len() as f64).max(2.0);
    for (dataset_index, dataset) in chart.datasets.iter().enumerate() {
        for (value_index, value) in dataset.data.iter().copied().enumerate() {
            let zero_y = scale_y(0.0, min, max, &plot);
            let value_y = scale_y(value, min, max, &plot);
            let x = plot.left
                + value_index as f64 * group_width
                + group_width * 0.14
                + dataset_index as f64 * bar_width;
            let y = zero_y.min(value_y);
            let height = (zero_y - value_y).abs().max(1.0);
            let color = color_for(&dataset.background, dataset_index, value_index);
            raster.fill_rect(x, y, (bar_width - 2.0).max(1.0), height, color);
        }
    }
}

fn render_line(raster: &mut Raster, chart: &ChartDefinition) {
    let plot = plot_area(raster.width, raster.height);
    let series_length = chart
        .datasets
        .iter()
        .map(|dataset| dataset.data.len())
        .max()
        .unwrap_or(1);
    let (min, max) = value_range(chart);
    draw_axes(raster, &plot);
    for (dataset_index, dataset) in chart.datasets.iter().enumerate() {
        let color = color_for(&dataset.border, dataset_index, 0);
        for value_index in 1..dataset.data.len() {
            let previous = dataset.data[value_index - 1];
            let value = dataset.data[value_index];
            raster.line(
                scale_x(value_index - 1, series_length, &plot),
                scale_y(previous, min, max, &plot),
                scale_x(value_index, series_length, &plot),
                scale_y(value, min, max, &plot),
                color,
            );
        }
    }
}

fn render_pie(raster: &mut Raster, chart: &ChartDefinition) -> Result<(), MarkdownConversionError> {
    let dataset = chart
        .datasets
        .first()
        .ok_or_else(|| chart_error("chart data.datasets must contain at least one dataset"))?;
    let values: Vec<f64> = dataset
        .data
        .iter()
        .copied()
        .map(|value| if value.is_sign_negative() { 0.0 } else { value })
        .collect();
    let total = values.iter().sum::<f64>();
    if total <= 0.0 {
        return Err(chart_error(
            "pie and doughnut charts require a positive total",
        ));
    }
    let center_x = raster.width as f64 / 2.0;
    let center_y = raster.height as f64 / 2.0;
    let radius = (raster.width.min(raster.height) as f64 * 0.38).max(1.0);
    let inner_radius = if matches!(chart.kind, ChartKind::Doughnut) {
        radius * 0.52
    } else {
        0.0
    };
    let disc = Disc {
        center_x,
        center_y,
        radius,
        inner_radius,
    };
    let mut start = -std::f64::consts::PI / 2.0;
    for (value_index, value) in values.iter().copied().enumerate() {
        let angle = (value / total) * std::f64::consts::PI * 2.0;
        let color = color_for(&dataset.background, value_index, value_index);
        if angle >= std::f64::consts::PI * 2.0 - f64::EPSILON {
            fill_disc(raster, &disc, None, color);
        } else {
            fill_disc(raster, &disc, Some((start, start + angle)), color);
        }
        start += angle;
    }
    Ok(())
}

fn draw_axes(raster: &mut Raster, plot: &Plot) {
    let bottom = plot.top + plot.height;
    raster.line(plot.left, plot.top, plot.left, bottom, AXIS_COLOR);
    raster.line(
        plot.left,
        bottom,
        plot.left + plot.width,
        bottom,
        AXIS_COLOR,
    );
    for index in 1..=4 {
        let y = plot.top + (plot.height / 5.0) * index as f64;
        raster.line(plot.left, y, plot.left + plot.width, y, GRID_COLOR);
    }
}

fn plot_area(width: u32, height: u32) -> Plot {
    let width = width as f64;
    let height = height as f64;
    let left = (width * 0.08).round().max(36.0);
    let right = (width * 0.04).round().max(18.0);
    let top = (height * 0.06).round().max(18.0);
    let bottom = (height * 0.1).round().max(28.0);
    Plot {
        left,
        top,
        width: (width - left - right).max(1.0),
        height: (height - top - bottom).max(1.0),
    }
}

fn value_range(chart: &ChartDefinition) -> (f64, f64) {
    let mut min = 0.0;
    let mut max = 0.0;
    for dataset in &chart.datasets {
        for value in &dataset.data {
            if *value < min {
                min = *value;
            }
            if *value > max {
                max = *value;
            }
        }
    }
    if min == max {
        min -= 1.0;
        max += 1.0;
    }
    (min, max)
}

fn scale_x(index: usize, count: usize, plot: &Plot) -> f64 {
    if count <= 1 {
        return plot.left + plot.width / 2.0;
    }
    plot.left + (index as f64 / (count - 1) as f64) * plot.width
}

fn scale_y(value: f64, min: f64, max: f64, plot: &Plot) -> f64 {
    plot.top + plot.height - ((value - min) / (max - min)) * plot.height
}

struct Disc {
    center_x: f64,
    center_y: f64,
    radius: f64,
    inner_radius: f64,
}

fn fill_disc(raster: &mut Raster, disc: &Disc, wedge: Option<(f64, f64)>, color: [u8; 3]) {
    let min_x = floor_i32(disc.center_x - disc.radius);
    let max_x = ceil_i32(disc.center_x + disc.radius);
    let min_y = floor_i32(disc.center_y - disc.radius);
    let max_y = ceil_i32(disc.center_y + disc.radius);
    let wedge = wedge.map(|(start, end)| (normalize_angle(start), normalize_angle(end)));
    let mut y = min_y;
    while y <= max_y {
        let mut x = min_x;
        while x <= max_x {
            let dx = x as f64 - disc.center_x;
            let dy = y as f64 - disc.center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance <= disc.radius && distance >= disc.inner_radius {
                let inside = match wedge {
                    Some((start, end)) => angle_in_range(normalize_angle(dy.atan2(dx)), start, end),
                    None => true,
                };
                if inside {
                    raster.set_pixel(x, y, color);
                }
            }
            if x == i32::MAX {
                break;
            }
            x += 1;
        }
        if y == i32::MAX {
            break;
        }
        y += 1;
    }
}

fn angle_in_range(angle: f64, start: f64, end: f64) -> bool {
    if end < start {
        angle >= start || angle <= end
    } else {
        angle >= start && angle <= end
    }
}

fn normalize_angle(angle: f64) -> f64 {
    let full = std::f64::consts::PI * 2.0;
    ((angle % full) + full) % full
}

fn color_for(colors: &ColorField, dataset_index: usize, value_index: usize) -> [u8; 3] {
    let (candidate, fallback_index) = match colors {
        ColorField::Many(items) => (
            items.get(value_index).and_then(|item| item.as_deref()),
            value_index,
        ),
        ColorField::One(text) => (Some(text.as_str()), dataset_index),
        ColorField::Absent => (None, dataset_index),
    };
    candidate
        .and_then(normalize_color)
        .unwrap_or(DEFAULT_COLORS[fallback_index % DEFAULT_COLORS.len()])
}

fn normalize_color(color: &str) -> Option<[u8; 3]> {
    let trimmed = color.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([red, green, blue])
}

fn resolve_dimension(
    block_value: Dimension,
    default_value: u32,
    max_value: u32,
    name: &str,
) -> Result<u32, MarkdownConversionError> {
    let value = match block_value {
        Dimension::Missing => f64::from(default_value),
        Dimension::Number(value) => value,
        Dimension::Invalid => {
            return Err(chart_error(format!(
                "chart {name} must be a positive integer"
            )));
        }
    };
    if !value.is_finite() || value <= 0.0 || value.fract() != 0.0 {
        return Err(chart_error(format!(
            "chart {name} must be a positive integer"
        )));
    }
    if value > f64::from(max_value) {
        return Err(chart_error(format!(
            "chart {name} exceeds maximum {max_value}"
        )));
    }
    Ok(value as u32)
}

fn floor_i32(value: f64) -> i32 {
    saturating_i32(value.floor())
}

fn ceil_i32(value: f64) -> i32 {
    saturating_i32(value.ceil())
}

/// JavaScript `Math.round` rounds halfway cases toward +infinity.
fn round_i32(value: f64) -> i32 {
    saturating_i32((value + 0.5).floor())
}

fn saturating_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    if value >= f64::from(i32::MAX) {
        i32::MAX
    } else if value <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

fn clamp_i32(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

fn chart_error(message: impl Into<String>) -> MarkdownConversionError {
    MarkdownConversionError::new(message)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, MarkdownConversionError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| chart_error("chart exceeds maximum 2000"))?;
    if rgba.len() != expected {
        return Err(chart_error("chart PNG encoding failed"));
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|_| chart_error("chart PNG encoding failed"))?;
    Ok(png)
}

fn parse_chart(definition: &str) -> Result<ChartDefinition, MarkdownConversionError> {
    let value = parse_json(definition)
        .map_err(|detail| chart_error(format!("expected chart JSON ({detail})")))?;
    let Json::Object(root) = value else {
        return Err(chart_error("chart definition must be a JSON object"));
    };
    let kind = match root.get("type").and_then(Json::as_str) {
        Some("bar") => ChartKind::Bar,
        Some("line") => ChartKind::Line,
        Some("pie") => ChartKind::Pie,
        Some("doughnut") => ChartKind::Doughnut,
        _ => {
            return Err(chart_error(
                "chart type must be one of bar, line, pie, doughnut",
            ));
        }
    };
    let data = match root.get("data") {
        Some(Json::Object(data)) => data,
        _ => {
            return Err(chart_error(
                "chart data.datasets must contain at least one dataset",
            ));
        }
    };
    if let Some(labels) = data.get("labels") {
        let Json::Array(labels) = labels else {
            return Err(chart_error("chart data.labels must be an array of strings"));
        };
        if labels.iter().any(|label| label.as_str().is_none()) {
            return Err(chart_error("chart data.labels must be an array of strings"));
        }
    }
    let Some(Json::Array(raw_datasets)) = data.get("datasets") else {
        return Err(chart_error(
            "chart data.datasets must contain at least one dataset",
        ));
    };
    if raw_datasets.is_empty() {
        return Err(chart_error(
            "chart data.datasets must contain at least one dataset",
        ));
    }
    let mut datasets = Vec::with_capacity(raw_datasets.len());
    for (index, dataset) in raw_datasets.iter().enumerate() {
        let Json::Object(dataset) = dataset else {
            return Err(chart_error(format!(
                "chart data.datasets[{index}].data must be an array"
            )));
        };
        let Some(Json::Array(raw_values)) = dataset.get("data") else {
            return Err(chart_error(format!(
                "chart data.datasets[{index}].data must be an array"
            )));
        };
        let mut data = Vec::with_capacity(raw_values.len());
        for value in raw_values {
            let Json::Number(number) = value else {
                return Err(chart_error(format!(
                    "chart data.datasets[{index}].data must contain finite numbers"
                )));
            };
            if !number.is_finite() {
                return Err(chart_error(format!(
                    "chart data.datasets[{index}].data must contain finite numbers"
                )));
            }
            data.push(*number);
        }
        if data.is_empty() {
            return Err(chart_error(format!(
                "chart data.datasets[{index}].data must contain finite numbers"
            )));
        }
        datasets.push(Dataset {
            data,
            background: read_colors(dataset.get("backgroundColor")),
            border: read_colors(dataset.get("borderColor")),
        });
    }
    Ok(ChartDefinition {
        kind,
        datasets,
        width: read_dimension(&root, "width"),
        height: read_dimension(&root, "height"),
    })
}

fn read_dimension(object: &BTreeMap<String, Json>, key: &str) -> Dimension {
    match object.get(key) {
        None | Some(Json::Null) => Dimension::Missing,
        Some(Json::Number(value)) => Dimension::Number(*value),
        Some(_) => Dimension::Invalid,
    }
}

fn read_colors(value: Option<&Json>) -> ColorField {
    match value {
        None => ColorField::Absent,
        Some(Json::String(text)) => ColorField::One(text.clone()),
        Some(Json::Array(items)) => ColorField::Many(
            items
                .iter()
                .map(|item| item.as_str().map(str::to_string))
                .collect(),
        ),
        Some(_) => ColorField::Absent,
    }
}

enum Json {
    Null,
    Bool,
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }
}

fn parse_json(input: &str) -> Result<Json, String> {
    let mut parser = Parser {
        input: input.as_bytes(),
        index: 0,
    };
    parser.skip_ws();
    if parser.peek().is_none() {
        return Err("Unexpected end of JSON input".to_string());
    }
    let value = parser.parse_value(0)?;
    parser.skip_ws();
    if parser.peek().is_some() {
        return Err(parser.error("Unexpected token in JSON"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.index).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }

    fn error(&self, message: &str) -> String {
        format!("{message} at position {}", self.index)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.error("Unexpected token in JSON"));
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(b't') => {
                self.consume_literal(b"true")?;
                Ok(Json::Bool)
            }
            Some(b'f') => {
                self.consume_literal(b"false")?;
                Ok(Json::Bool)
            }
            Some(b'n') => {
                self.consume_literal(b"null")?;
                Ok(Json::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => Err(self.error("Unexpected token in JSON")),
            None => Err("Unexpected end of JSON input".to_string()),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Json, String> {
        self.index += 1;
        let mut object = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.index += 1;
            return Ok(Json::Object(object));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(match self.peek() {
                    None => "Unexpected end of JSON input".to_string(),
                    Some(_) => self.error("Unexpected token in JSON"),
                });
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(match self.peek() {
                    None => "Unexpected end of JSON input".to_string(),
                    Some(_) => self.error("Unexpected token in JSON"),
                });
            }
            self.index += 1;
            let value = self.parse_value(depth + 1)?;
            object.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b'}') => {
                    self.index += 1;
                    return Ok(Json::Object(object));
                }
                Some(_) => return Err(self.error("Unexpected token in JSON")),
                None => return Err("Unexpected end of JSON input".to_string()),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Json, String> {
        self.index += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.index += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.parse_value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b']') => {
                    self.index += 1;
                    return Ok(Json::Array(items));
                }
                Some(_) => return Err(self.error("Unexpected token in JSON")),
                None => return Err("Unexpected end of JSON input".to_string()),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(self.error("Unexpected token in JSON"));
        }
        self.index += 1;
        let mut output = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err("Unexpected end of JSON input".to_string());
            };
            if byte == b'"' {
                self.index += 1;
                return Ok(output);
            }
            if byte == b'\\' {
                self.index += 1;
                self.push_escape(&mut output)?;
                continue;
            }
            if byte < 0x20 {
                return Err(self.error("Unexpected token in JSON"));
            }
            let rest = std::str::from_utf8(&self.input[self.index..])
                .map_err(|_| self.error("Unexpected token in JSON"))?;
            let Some(ch) = rest.chars().next() else {
                return Err("Unexpected end of JSON input".to_string());
            };
            output.push(ch);
            self.index += ch.len_utf8();
        }
    }

    fn push_escape(&mut self, output: &mut String) -> Result<(), String> {
        let Some(byte) = self.peek() else {
            return Err("Unexpected end of JSON input".to_string());
        };
        self.index += 1;
        match byte {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => self.push_unicode_escape(output)?,
            _ => return Err(self.error("Unexpected token in JSON")),
        }
        Ok(())
    }

    fn push_unicode_escape(&mut self, output: &mut String) -> Result<(), String> {
        let unit = self.read_hex4()?;
        if (0xD800..=0xDBFF).contains(&unit) && self.starts_with(br"\u") {
            let saved = self.index;
            self.index += 2;
            let low = self.read_hex4()?;
            if (0xDC00..=0xDFFF).contains(&low) {
                let code =
                    0x10000 + (((u32::from(unit) - 0xD800) << 10) | (u32::from(low) - 0xDC00));
                output.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                return Ok(());
            }
            self.index = saved;
        }
        output.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
        Ok(())
    }

    fn read_hex4(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err("Unexpected end of JSON input".to_string());
            };
            let Some(digit) = hex_value(byte) else {
                return Err(self.error("Unexpected token in JSON"));
            };
            self.index += 1;
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn starts_with(&self, needle: &[u8]) -> bool {
        self.input[self.index..].starts_with(needle)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.index += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.error("Unexpected token in JSON"));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.index += 1;
                }
            }
            _ => return Err(self.error("Unexpected token in JSON")),
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            let fraction = self.index;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == fraction {
                return Err(self.error("Unexpected token in JSON"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            let exponent = self.index;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == exponent {
                return Err(self.error("Unexpected token in JSON"));
            }
        }
        let text = std::str::from_utf8(&self.input[start..self.index])
            .map_err(|_| self.error("Unexpected token in JSON"))?;
        let value = text
            .parse::<f64>()
            .map_err(|_| self.error("Unexpected token in JSON"))?;
        Ok(Json::Number(value))
    }

    fn consume_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.index + literal.len() <= self.input.len()
            && &self.input[self.index..self.index + literal.len()] == literal
        {
            let after = self.index + literal.len();
            if matches!(
                self.input.get(after),
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
            ) {
                return Err(self.error("Unexpected token in JSON"));
            }
            self.index = after;
            return Ok(());
        }
        if self.peek().is_none() {
            Err("Unexpected end of JSON input".to_string())
        } else {
            Err(self.error("Unexpected token in JSON"))
        }
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::render_chart;

    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn chart(kind: &str, width: Option<u32>, height: Option<u32>) -> String {
        let size = match (width, height) {
            (Some(width), Some(height)) => format!(r#""width":{width},"height":{height},"#,),
            _ => String::new(),
        };
        format!(
            r#"{{{size}"type":"{kind}","data":{{"labels":["Q1","Q2","Q3"],"datasets":[{{"data":[12,18,9]}}]}}}}"#
        )
    }

    fn assert_png_size(bytes: &[u8], width: u32, height: u32) {
        assert!(
            bytes.starts_with(PNG_SIGNATURE),
            "missing png signature: {bytes:?}"
        );
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(bytes[8..12].try_into().unwrap()), 13);
        let actual_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let actual_height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!((actual_width, actual_height), (width, height));
        let decoded = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
            .into_dimensions()
            .expect("png dimensions");
        assert_eq!(decoded, (width, height));
    }

    fn decode(bytes: &[u8]) -> image::RgbaImage {
        image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
            .decode()
            .expect("png")
            .into_rgba8()
    }

    #[test]
    fn png_header_and_pixel_size_match() {
        for kind in ["bar", "line", "pie", "doughnut"] {
            let png = render_chart(&chart(kind, None, None), 160, 90).unwrap();
            assert_png_size(&png, 160, 90);
        }
        let defaults = render_chart(&chart("bar", None, None), 640, 360).unwrap();
        assert_png_size(&defaults, 640, 360);
    }

    #[test]
    fn json_dimensions_override_arguments() {
        let png = render_chart(&chart("line", Some(321), Some(123)), 640, 360).unwrap();
        assert_png_size(&png, 321, 123);
    }

    #[test]
    fn invalid_json_returns_an_error() {
        let error = render_chart("{", 640, 360).unwrap_err();
        assert!(error.message().starts_with("expected chart JSON ("));
        let error = render_chart("[]", 640, 360).unwrap_err();
        assert_eq!(error.message(), "chart definition must be a JSON object");
        let error = render_chart(r#"{"type":"scatter"}"# , 640, 360).unwrap_err();
        assert_eq!(
            error.message(),
            "chart type must be one of bar, line, pie, doughnut"
        );
        let error = render_chart(
            r#"{"type":"bar","data":{"datasets":[{"data":["bad"]}]}}"#,
            640,
            360,
        )
        .unwrap_err();
        assert!(error.message().contains("finite numbers"));
    }

    #[test]
    fn dimensions_respect_the_2000_pixel_maximum() {
        let error = render_chart(&chart("bar", Some(2001), Some(10)), 640, 360).unwrap_err();
        assert_eq!(error.message(), "chart width exceeds maximum 2000");
        let error = render_chart(&chart("bar", None, None), 0, 360).unwrap_err();
        assert_eq!(error.message(), "chart width must be a positive integer");
        let png = render_chart(&chart("bar", Some(2000), Some(8)), 640, 360).unwrap();
        assert_png_size(&png, 2000, 8);
    }

    #[test]
    fn pie_and_doughnut_fill_matches_the_geometry() {
        let pie = render_chart(
            r##"{"type":"pie","data":{"datasets":[{"data":[1],"backgroundColor":["#E15759"]}]},"width":96,"height":96}"##,
            640,
            360,
        )
        .unwrap();
        assert_png_size(&pie, 96, 96);
        let pie_pixels = decode(&pie);
        assert_eq!(pie_pixels.get_pixel(48, 48).0, [0xE1, 0x57, 0x59, 255]);
        let colored = pie_pixels
            .pixels()
            .filter(|pixel| pixel.0 != [255, 255, 255, 255])
            .count();
        assert!(colored > 100, "single-slice pie colored {colored} pixels");

        let doughnut = render_chart(
            r##"{"type":"doughnut","data":{"labels":["All","None"],"datasets":[{"data":[5,0],"backgroundColor":["#E15759","#4E79A7"]}]},"width":96,"height":96}"##,
            640,
            360,
        )
        .unwrap();
        assert_png_size(&doughnut, 96, 96);
        let doughnut_pixels = decode(&doughnut);
        assert_eq!(doughnut_pixels.get_pixel(48, 48).0, [255, 255, 255, 255]);
        let colored = doughnut_pixels
            .pixels()
            .filter(|pixel| pixel.0 != [255, 255, 255, 255])
            .count();
        assert!(colored > 100, "doughnut colored {colored} pixels");

        let error = render_chart(
            r#"{"type":"pie","data":{"datasets":[{"data":[0,-2]}]}}"#,
            32,
            32,
        )
        .unwrap_err();
        assert_eq!(
            error.message(),
            "pie and doughnut charts require a positive total"
        );
    }
}
