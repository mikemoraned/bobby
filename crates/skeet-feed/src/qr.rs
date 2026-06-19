use qrcode::QrCode;
use qrcode::render::svg;

#[derive(Debug, thiserror::Error)]
pub enum QrError {
    #[error("failed to encode QR code: {0}")]
    Encode(#[from] qrcode::types::QrError),
}

/// Render `url` as a self-contained inline SVG QR code. The output has no
/// external references, so it can be embedded directly in a page.
pub fn qr_svg(url: &str) -> Result<String, QrError> {
    let code = QrCode::new(url.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(96, 96)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_well_formed_svg() {
        let svg = qr_svg("https://bobby.houseofmoran.io/").expect("should encode");
        assert!(svg.contains("<svg"), "output should be an SVG element");
        assert!(svg.contains("</svg>"), "SVG element should be closed");
        assert!(
            !svg.contains("http://www.w3.org/1999/xlink"),
            "no external refs"
        );
    }

    #[test]
    fn different_urls_produce_different_codes() {
        let a = qr_svg("https://bobby.houseofmoran.io/").expect("encode a");
        let b = qr_svg("https://bobby-staging.houseofmoran.io/").expect("encode b");
        assert_ne!(a, b, "distinct URLs should encode to distinct QR codes");
    }
}
