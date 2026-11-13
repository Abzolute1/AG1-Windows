//! Bounded, non-browser SVG previews. Never resolve files, URLs or embedded images.
//! Unsupported constructs leave the source readable instead of rendering a misleading partial image.
use ag1_core::DomainError;
use resvg::{tiny_skia, usvg};

pub(super) fn render(
    bytes: &[u8],
    check: &mut impl FnMut() -> Result<(), DomainError>,
) -> Result<Vec<u8>, DomainError> {
    render_bounded(bytes, check, false)
}
pub(super) fn render_icon(bytes: &[u8]) -> Result<Vec<u8>, DomainError> {
    render_bounded(bytes, &mut || Ok(()), true)
}
fn render_bounded(
    bytes: &[u8],
    check: &mut impl FnMut() -> Result<(), DomainError>,
    icon: bool,
) -> Result<Vec<u8>, DomainError> {
    check()?;
    if bytes.len() > 256 * 1024 {
        return Err(DomainError::LimitExceeded(
            "SVG preview input (256 KiB)".into(),
        ));
    }
    let _permit = if icon {
        super::DECODER.lock().map_err(|_| DomainError::Conflict)?
    } else {
        super::DECODER
            .try_lock()
            .map_err(|_| DomainError::QueueFull)?
    };
    let text =
        std::str::from_utf8(bytes).map_err(|_| DomainError::InvalidInput("SVG UTF-8".into()))?;
    let doc = usvg::roxmltree::Document::parse_with_options(
        text,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 2048,
        },
    )
    .map_err(|error| match error {
        usvg::roxmltree::Error::NodesLimitReached => DomainError::LimitExceeded(
            "SVG XML nodes (2048; includes elements, comments and text nodes)".into(),
        ),
        usvg::roxmltree::Error::AttributesLimitReached =>
            DomainError::LimitExceeded("SVG XML attributes".into()),
        usvg::roxmltree::Error::NamespacesLimitReached =>
            DomainError::LimitExceeded("SVG XML namespaces".into()),
        error => DomainError::InvalidInput(format!(
            "SVG XML parse error at line {}, column {}: {}. Fix the XML syntax at this location and preview again; this is not a complexity-limit error",
            error.pos().row, error.pos().col, error
        )),
    })?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return Err(DomainError::InvalidInput("SVG namespace/root".into()));
    }
    // resvg clips filter regions to its bounded canvas. Also cap primitive
    // chains so a small SVG cannot create an unbounded sequence of buffers.
    if doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name().starts_with("fe"))
        .count()
        > 16
    {
        return Err(DomainError::LimitExceeded(
            "SVG filter primitives (16)".into(),
        ));
    }
    for node in doc.descendants() {
        check()?;
        if node.ancestors().take(34).count() > 32 {
            return Err(DomainError::LimitExceeded("SVG nesting".into()));
        }
        if node.is_pi() {
            return Err(DomainError::InvalidInput(
                "SVG processing instructions".into(),
            ));
        }
        if !node.is_element() {
            continue;
        }
        if node.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
            || !matches!(
                node.tag_name().name(),
                "svg"
                    | "g"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "defs"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "clipPath"
                    | "filter"
                    | "feDropShadow"
                    | "feGaussianBlur"
                    | "feOffset"
                    | "feFlood"
                    | "feBlend"
                    | "feComposite"
                    | "feColorMatrix"
                    | "feMerge"
                    | "feMergeNode"
                    | "title"
                    | "desc"
            )
        {
            return Err(DomainError::InvalidInput(format!(
                "SVG preview does not support <{}>; use vector paths/shapes",
                node.tag_name().name()
            )));
        }
        for attr in node.attributes() {
            let value = attr.value().to_ascii_lowercase();
            if attr.name().to_ascii_lowercase().starts_with("on")
                || matches!(attr.name(), "href" | "base")
                || value.contains('\\')
                || value.contains("@import")
                || (value.contains("url(")
                    && !value
                        .split("url(")
                        .skip(1)
                        .all(|v| v.trim_start().starts_with('#')))
            {
                return Err(DomainError::InvalidInput(
                    "SVG active content or external reference".into(),
                ));
            }
        }
    }
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    check()?;
    let tree = usvg::Tree::from_xmltree(&doc, &options)
        .map_err(|_| DomainError::InvalidInput("SVG geometry".into()))?;
    let size = tree.size();
    let scale = (if icon { 64.0 } else { 1024.0 } / size.width().max(size.height())).min(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    if width > 1024 || height > 1024 {
        return Err(DomainError::LimitExceeded("SVG preview dimensions".into()));
    }
    let mut pixels = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| DomainError::LimitExceeded("SVG pixels".into()))?;
    check()?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixels.as_mut(),
    );
    check()?;
    pixels
        .encode_png()
        .map_err(|_| DomainError::InvalidInput("SVG preview encoding".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn svg(body: &str) -> Vec<u8> {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="160">{body}</svg>"#)
            .into_bytes()
    }
    #[test]
    fn svg_preview_is_png_with_real_pixels_and_preserved_source() {
        let _serial = TEST_SERIAL.lock().unwrap();
        let bytes = svg(r#"<rect width="320" height="160" fill="red"/>"#);
        let png = render(&bytes, &mut || Ok(())).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (320, 160));
        assert_eq!(image.get_pixel(40, 40).0, [255, 0, 0, 255]);
        let args = serde_json::json!({"path":"ball.svg"});
        let output =
            super::super::read_file_with_control("ball.svg", &args, &bytes, &mut || Ok(()))
                .unwrap()
                .unwrap();
        assert_eq!(output["content"][1]["mimeType"], "image/png");
        assert_eq!(
            output["details"]["imagePreview"]["sourceBase64"],
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        );
        assert!(
            output["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("<svg")
        );
        let output = super::super::read_file_with_control(
            "ball.svg",
            &serde_json::json!({"path":"ball.svg","offset":1}),
            &bytes,
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(output["content"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn svg_shadow_and_blur_are_rendered_as_visible_pixels() {
        let _serial = TEST_SERIAL.lock().unwrap();
        let bytes = svg(
            r##"<defs><filter id="shadow"><feDropShadow dx="12" dy="8" stdDeviation="3" flood-color="blue"/></filter><filter id="blur"><feGaussianBlur stdDeviation="2"/></filter></defs><rect x="40" y="40" width="60" height="40" fill="red" filter="url(#shadow)"/><circle cx="200" cy="80" r="15" fill="green" filter="url(#blur)"/>"##,
        );
        let png = render(&bytes, &mut || Ok(())).unwrap();
        let pixels = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(pixels.get_pixel(60, 60).0, [255, 0, 0, 255]);
        let shadow = pixels.get_pixel(105, 75).0;
        assert!(
            shadow[2] > 100 && shadow[3] > 0,
            "missing blue shadow: {shadow:?}"
        );
        assert!(pixels.get_pixel(200, 80).0[1] > 0);
    }
    #[test]
    fn svg_preview_rejects_active_external_and_expanding_content() {
        let _serial = TEST_SERIAL.lock().unwrap();
        for body in [
            r#"<script>alert(1)</script>"#,
            r#"<image href="file:///etc/passwd"/>"#,
            r#"<image href="https://example.com/a.png"/>"#,
            r#"<image href="data:image/svg+xml;base64,eA=="/>"#,
            r##"<use href="#a"/>"##,
            r#"<foreignObject/>"#,
            r#"<filter><feImage href="https://example.com/image.png"/></filter>"#,
            r#"<text>Not silently omitted</text>"#,
            r#"<rect onclick="alert(1)"/>"#,
            r#"<rect style="fill:url(file:///tmp/a)"/>"#,
        ] {
            assert!(render(&svg(body), &mut || Ok(())).is_err(), "{body}");
        }
        assert!(render(b"<!DOCTYPE svg [<!ENTITY x 'secret'>]><svg xmlns='http://www.w3.org/2000/svg'>&x;</svg>", &mut || Ok(())).is_err());
        let nested = format!("{}{}", "<g>".repeat(35), "</g>".repeat(35));
        assert!(matches!(
            render(&svg(&nested), &mut || Ok(())),
            Err(DomainError::LimitExceeded(_))
        ));
        assert!(render(&svg(&"<circle r='1'/>".repeat(2050)), &mut || Ok(())).is_err());
        assert!(render(&vec![b' '; 256 * 1024 + 1], &mut || Ok(())).is_err());
        assert!(matches!(
            render(&svg(""), &mut || Err(DomainError::Cancelled)),
            Err(DomainError::Cancelled)
        ));
    }
    #[test]
    fn svg_syntax_errors_report_location_and_do_not_claim_complexity() {
        let _serial = TEST_SERIAL.lock().unwrap();
        let malformed = b"<svg xmlns='http://www.w3.org/2000/svg'>\n<g/>\n</g>\n</svg>";
        let error = render(malformed, &mut || Ok(())).unwrap_err();
        let DomainError::InvalidInput(message) = error else {
            panic!("{error:?}")
        };
        assert!(message.contains("line 3, column 1"), "{message}");
        assert!(message.contains("expected 'svg' tag"), "{message}");
        assert!(message.contains("not a complexity-limit error"));
        let error = render(&svg(&"<circle r='1'/>".repeat(2050)), &mut || Ok(())).unwrap_err();
        assert!(
            matches!(error, DomainError::LimitExceeded(ref message) if message.contains("XML nodes (2048"))
        );
    }
    #[test]
    fn svg_preview_bounds_size_and_reports_unsupported_without_failing_source_read() {
        let _serial = TEST_SERIAL.lock().unwrap();
        let png = render(b"<svg xmlns='http://www.w3.org/2000/svg' width='100000' height='50000'><circle r='20'/></svg>", &mut || Ok(())).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 1024);
        let result = super::super::read_file_with_control(
            "a.svg",
            &serde_json::json!({"path":"a.svg"}),
            &svg("<script/>"),
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
        assert!(
            result["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["type"] == "text")
        );
        assert!(
            result["content"][1]["text"]
                .as_str()
                .unwrap()
                .contains("source read successfully")
        );
    }
}
