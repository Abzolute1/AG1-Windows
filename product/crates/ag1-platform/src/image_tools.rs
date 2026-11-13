//! Native image attachment projection. No Node, browser or model process is involved.
use ag1_core::DomainError;
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, imageops::FilterType};
use serde_json::{Value, json};
use std::{io::Cursor, sync::Mutex};
static DECODER: Mutex<()> = Mutex::new(());
mod svg;
/// Read SVG source and attach a safe raster preview on a whole-file read.
/// Ranged reads retain the ordinary text contract for code inspection/editing.
pub(crate) fn read_file_with_control(
    path: &str,
    args: &Value,
    bytes: &[u8],
    check: &mut impl FnMut() -> Result<(), DomainError>,
) -> Result<Option<Value>, DomainError> {
    if std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        let mut result = ag1_core::tools::read::text(args, bytes)?;
        if args.get("offset").is_none() && args.get("limit").is_none() {
            check()?;
            match svg::render(bytes, check) {
                Ok(png) => {
                    result["content"].as_array_mut().unwrap().push(json!({"type":"image","data":STANDARD.encode(png),"mimeType":"image/png"}));
                    let name = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy();
                    result["details"]["imagePreview"] = json!({"name":name,"sourceMimeType":"image/svg+xml","sourceBase64":STANDARD.encode(bytes)});
                }
                Err(error @ (DomainError::Cancelled | DomainError::TimedOut)) => return Err(error),
                Err(error) => result["content"].as_array_mut().unwrap().push(json!({"type":"text","text":format!("SVG source read successfully; inline preview unavailable: {error:?}. Read the specific diagnostic: for an XML parse error, fix the indicated syntax and call ui with this path again. Do not simplify valid artwork or switch tools to fix malformed XML. Only an explicit LimitExceeded reports a resource limit. Do not use website screenshot tools for local images.")})),
            }
        }
        return Ok(Some(result));
    }
    read_with_control(bytes, true, check)
}
const INLINE_BYTES: usize = 9 * 1024 * 1024 / 2;
fn encoded_size(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}
fn mime(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}
fn omitted(mime: &str) -> Value {
    json!({"content":[{"type":"text","text":format!("Read image file [{mime}]\n[Image omitted: could not be resized below the inline image size limit.]")}]})
}
fn attachment(bytes: &[u8], mime: &str, note: Option<String>) -> Value {
    let mut text = format!("Read image file [{mime}]");
    if let Some(note) = note {
        text.push('\n');
        text.push_str(&note);
    }
    json!({"content":[{"type":"text","text":text},{"type":"image","data":STANDARD.encode(bytes),"mimeType":mime}]})
}
pub(crate) fn supported_header(bytes: &[u8]) -> bool {
    image::guess_format(bytes).ok().is_some_and(|format| {
        mime(format).is_some() && (format != ImageFormat::Png || bytes.get(12..16) == Some(b"IHDR"))
    })
}
/// Returns None for non-supported image formats. The caller then uses the text reader.
pub fn read(bytes: &[u8]) -> Result<Option<Value>, DomainError> {
    read_with_resize(bytes, true)
}
/// Host configuration only; a model cannot disable attachment limits through tool arguments.
pub fn read_with_resize(bytes: &[u8], resize_images: bool) -> Result<Option<Value>, DomainError> {
    read_with_control(bytes, resize_images, &mut || Ok(()))
}
/// Cooperative cancellation between bounded codec/resample stages. A codec call
/// itself is synchronous; the native owner must use its bounded blocking worker.
pub fn read_with_control(
    bytes: &[u8],
    resize_images: bool,
    check: &mut impl FnMut() -> Result<(), DomainError>,
) -> Result<Option<Value>, DomainError> {
    check()?;
    let Ok(format) = image::guess_format(bytes) else {
        return Ok(None);
    };
    if format == ImageFormat::Png && bytes.get(12..16) != Some(b"IHDR") {
        return Ok(None);
    }
    let Some(mime) = mime(format) else {
        return Ok(None);
    };
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("encoded image input".into()));
    }
    if !resize_images {
        return Ok(Some(attachment(bytes, mime, None)));
    }
    let _permit = DECODER.try_lock().map_err(|_| DomainError::QueueFull)?;
    Ok(Some(match resize(bytes, format, mime, false, check) {
        Ok(v) => v,
        Err(error @ (DomainError::Cancelled | DomainError::TimedOut)) => return Err(error),
        Err(_) => omitted(mime),
    }))
}
/// Untrusted MCP images are fully decoded and rasterized. Never pass through
/// source metadata, animation or SVG to the renderer. Original custody is the
/// caller's responsibility; this only returns a bounded display projection.
pub(crate) fn mcp_preview(bytes: &[u8], declared: &str) -> Result<Value, DomainError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("MCP image bytes".into()));
    }
    let format = image::guess_format(bytes)
        .map_err(|_| DomainError::InvalidInput("MCP image header".into()))?;
    if mime(format) != Some(declared) {
        return Err(DomainError::InvalidInput(
            "MCP image MIME/header mismatch".into(),
        ));
    }
    let _permit = DECODER.try_lock().map_err(|_| DomainError::QueueFull)?;
    resize(bytes, format, declared, true, &mut || Ok(()))
}

fn resize(
    bytes: &[u8],
    format: ImageFormat,
    mime: &str,
    raster_only: bool,
    check: &mut impl FnMut() -> Result<(), DomainError>,
) -> Result<Value, DomainError> {
    check()?;
    let invalid = |_| DomainError::InvalidInput("image decoding or encoding".into());
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(invalid)?;
    let (w, h) = decoder.dimensions();
    if w == 0
        || h == 0
        || u64::from(w) * u64::from(h) > 16 * 1024 * 1024
        || decoder.total_bytes() > 64 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("decoded image pixels".into()));
    }
    // from_decoder alone does not reserve its final allocation: account for it explicitly.
    limits.reserve(decoder.total_bytes()).map_err(invalid)?;
    decoder.set_limits(limits).map_err(invalid)?;
    let orientation = decoder.orientation().map_err(invalid)?;
    check()?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(invalid)?;
    check()?;
    decoded.apply_orientation(orientation);
    let (original_width, original_height) = (decoded.width(), decoded.height());
    if !raster_only
        && original_width <= 2000
        && original_height <= 2000
        && encoded_size(bytes.len()) < INLINE_BYTES
    {
        return Ok(attachment(bytes, mime, None));
    }
    // Photon uses RGBA pixels, Lanczos3 and PNG followed by these ordered JPEG qualities.
    let decoded = DynamicImage::ImageRgba8(decoded.into_rgba8());
    let (mut width, mut height) = (original_width, original_height);
    if width > 2000 {
        height = (f64::from(height) * 2000.0 / f64::from(width)).round() as u32;
        width = 2000;
    }
    if height > 2000 {
        width = (f64::from(width) * 2000.0 / f64::from(height)).round() as u32;
        height = 2000;
    }
    width = width.max(1);
    height = height.max(1);
    loop {
        check()?;
        let resized = decoded.resize_exact(width, height, FilterType::Lanczos3);
        check()?;
        let mut png = Cursor::new(Vec::new());
        resized
            .write_to(&mut png, ImageFormat::Png)
            .map_err(invalid)?;
        check()?;
        let mut chosen = if encoded_size(png.get_ref().len()) < INLINE_BYTES {
            Some((png.into_inner(), "image/png"))
        } else {
            None
        };
        if chosen.is_none() {
            let rgb = resized.to_rgb8();
            for quality in [80, 85, 70, 55, 40] {
                check()?;
                let mut jpeg = vec![];
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
                    .encode_image(&rgb)
                    .map_err(invalid)?;
                check()?;
                if encoded_size(jpeg.len()) < INLINE_BYTES {
                    chosen = Some((jpeg, "image/jpeg"));
                    break;
                }
            }
        }
        if let Some((data, mime)) = chosen {
            let note = format!(
                "[Image: original {original_width}x{original_height}, displayed at {width}x{height}. Multiply coordinates by {:.2} to map to original image.]",
                f64::from(original_width) / f64::from(width)
            );
            return Ok(attachment(&data, mime, Some(note)));
        }
        if width == 1 && height == 1 {
            return Ok(omitted(mime));
        }
        width = (width * 3 / 4).max(1);
        height = (height * 3 / 4).max(1);
    }
}

/// Registry avatars are decoded once and reduced to metadata-free 64px PNGs.
pub(crate) fn registry_icon(bytes: &[u8], declared: &str) -> Result<String, DomainError> {
    if bytes.len() > 128 * 1024 {
        return Err(DomainError::LimitExceeded("registry icon bytes".into()));
    }
    let format = image::guess_format(bytes)
        .map_err(|_| DomainError::InvalidInput("registry icon header".into()))?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) || mime(format) != Some(declared) {
        return Err(DomainError::InvalidInput("registry icon MIME".into()));
    }
    small_icon(bytes, format)
}

pub(crate) fn website_icon(bytes: &[u8]) -> Result<String, DomainError> {
    if bytes.len() > 128 * 1024 {
        return Err(DomainError::LimitExceeded("website icon bytes".into()));
    }
    // Render supported static SVG through the existing bounded, non-network renderer.
    // The WebView still receives only a sanitized PNG; scripts/external resources fail.
    if image::guess_format(bytes).is_err() {
        let png = svg::render_icon(bytes)?;
        return small_icon(&png, ImageFormat::Png);
    }
    let format = image::guess_format(bytes)
        .map_err(|_| DomainError::InvalidInput("website icon header".into()))?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Ico
    ) {
        return Err(DomainError::InvalidInput("website icon format".into()));
    }
    small_icon(bytes, format)
}
fn small_icon(bytes: &[u8], format: ImageFormat) -> Result<String, DomainError> {
    let _permit = DECODER.lock().map_err(|_| DomainError::Conflict)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| DomainError::InvalidInput("registry icon decode".into()))?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || w > 1024 || h > 1024 || decoder.total_bytes() > 4 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("registry icon pixels".into()));
    }
    limits
        .reserve(decoder.total_bytes())
        .map_err(|_| DomainError::LimitExceeded("registry icon allocation".into()))?;
    decoder
        .set_limits(limits)
        .map_err(|_| DomainError::LimitExceeded("registry icon decoder".into()))?;
    let decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| DomainError::InvalidInput("registry icon pixels".into()))?
        .thumbnail(64, 64);
    let mut output = Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| DomainError::InvalidInput("registry icon encode".into()))?;
    if output.get_ref().len() > 24 * 1024 {
        return Err(DomainError::LimitExceeded("registry icon output".into()));
    }
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}
