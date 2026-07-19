pub fn terminal_blocks(text: &str) -> Option<String> {
    let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Low).ok()?;
    let quiet = 2i32;
    let mut out = String::new();
    let mut row = -quiet;
    while row < qr.size() + quiet {
        out.push_str("  ");
        for col in -quiet..qr.size() + quiet {
            // Out-of-range modules read as light, which draws the quiet zone.
            out.push(
                match (qr.get_module(col, row), qr.get_module(col, row + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                },
            );
        }
        out.push('\n');
        row += 2;
    }
    Some(out)
}

pub fn svg(text: &str) -> Option<String> {
    let qr = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Low).ok()?;
    let border = 4i32;
    let size = qr.size() + 2 * border;
    let mut path = String::new();
    for y in 0..qr.size() {
        for x in 0..qr.size() {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + border, y + border));
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\">\
         <rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\
         <path d=\"{path}\" fill=\"#000\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_renders_terminal_blocks() {
        let q = terminal_blocks("HELLO").unwrap();
        assert!(q.contains('█'), "{q}");
        assert!(q.lines().count() >= 12, "quiet zone + modules");
    }

    #[test]
    fn a_short_text_renders_a_self_contained_svg() {
        let s = svg("http://192.168.1.10:7777/?token=abc").unwrap();
        assert!(s.starts_with("<svg "), "{s}");
        assert!(s.contains("<path "), "{s}");
        assert!(s.contains("viewBox"), "{s}");
    }

    #[test]
    fn an_overlong_text_returns_none_instead_of_panicking() {
        assert!(svg(&"x".repeat(5000)).is_none());
    }
}
