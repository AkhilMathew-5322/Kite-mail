//! QR code reading for "scan to add / copy setup": camera frames and pictures arrive from the
//! web UI as 8-bit grayscale pixels, so the same pure-Rust decoder runs on every platform.

/// Finds and decodes the first readable QR code in a grayscale image (`width` pixels per row).
pub fn decode_gray(pixels: &[u8], width: usize) -> Option<String> {
    if width == 0 || pixels.len() < width {
        return None;
    }
    let height = pixels.len() / width;
    // larger frames are only slower: the UI sends at most ~1000 px per side
    if width > 4000 || height > 4000 {
        return None;
    }
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| pixels[y * width + x]);
    img.detect_grids().into_iter().find_map(|g| g.decode().ok().map(|(_, text)| text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders `text` as a QR code: `scale` px per module, quiet zone, optional noise/offset.
    fn render(text: &str, scale: usize, w: usize, h: usize, ox: usize, oy: usize, bg: u8) -> Vec<u8> {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let n = code.width();
        let colors = code.to_colors();
        let mut px = vec![bg; w * h];
        for y in 0..n * scale {
            for x in 0..n * scale {
                if colors[(y / scale) * n + x / scale] == qrcode::Color::Dark {
                    px[(oy + y) * w + ox + x] = 20;
                }
            }
        }
        px
    }

    #[test]
    fn reads_pairing_code_from_a_frame() {
        let text = "kitemail://pair?h=192.168.1.20:47213&k=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg&t=8f3a2c";
        let px = render(text, 6, 640, 480, 150, 60, 235);
        assert_eq!(decode_gray(&px, 640).as_deref(), Some(text));
    }

    #[test]
    fn no_code_or_bad_input() {
        assert_eq!(decode_gray(&vec![200u8; 320 * 240], 320), None);
        assert_eq!(decode_gray(&[], 0), None);
        assert_eq!(decode_gray(&[1, 2, 3], 10), None);
    }
}
