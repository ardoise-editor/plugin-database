//! A binary value (`bytea`, `BLOB`) as the drivers write it, seen as bytes:
//! a hex dump, what kind of file it is, and a picture when it is one.

/// The bytes of a value written as `\x…` (PostgreSQL), `x'…'` (SQLite) or
/// `0x…` (MySQL).
pub fn bytes_of(value: &str) -> Option<Vec<u8>> {
    let value = value.trim();
    let hex = value
        .strip_prefix("\\x")
        .or_else(|| value.strip_prefix("0x"))
        .or_else(|| {
            value
                .strip_prefix("x'")
                .or_else(|| value.strip_prefix("X'"))
                .and_then(|v| v.strip_suffix('\''))
        })?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// What the first bytes say the file is: its name and its media type.
pub fn kind_of(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(("PNG", "image/png"))
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Some(("JPEG", "image/jpeg"))
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(("GIF", "image/gif"))
    } else if bytes.len() > 12 && starts(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(("WebP", "image/webp"))
    } else if starts(b"%PDF-") {
        Some(("PDF", "application/pdf"))
    } else if starts(b"PK\x03\x04") {
        Some(("ZIP", "application/zip"))
    } else if starts(&[0x1f, 0x8b]) {
        Some(("gzip", "application/gzip"))
    } else {
        None
    }
}

/// The most bytes shown in the dump.
pub const DUMP_BYTES: usize = 4096;

/// `offset  hex bytes  |text|`, sixteen bytes a line.
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (line, chunk) in bytes
        .iter()
        .take(DUMP_BYTES)
        .collect::<Vec<_>>()
        .chunks(16)
        .enumerate()
    {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let text: String = chunk
            .iter()
            .map(|b| {
                if (0x20..0x7f).contains(*b) {
                    **b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!(
            "{:08x}  {:<47}  |{text}|\n",
            line * 16,
            hex.join(" ")
        ));
    }
    if bytes.len() > DUMP_BYTES {
        out.push_str(&format!("… {} more bytes\n", bytes.len() - DUMP_BYTES));
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(CHARS[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A picture of the bytes when they are an image: the SVG that holds it.
pub fn image_svg(bytes: &[u8]) -> Option<String> {
    let (_, media) = kind_of(bytes).filter(|(_, m)| m.starts_with("image/"))?;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"320\" height=\"240\" viewBox=\"0 0 320 240\"><image width=\"320\" height=\"240\" \
         preserveAspectRatio=\"xMidYMid meet\" xlink:href=\"data:{media};base64,{}\"/></svg>",
        base64(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_of_each_system_is_read_as_bytes() {
        assert_eq!(bytes_of("\\x89504e47"), Some(vec![0x89, 0x50, 0x4e, 0x47]));
        assert_eq!(bytes_of("x'00ff'"), Some(vec![0, 255]));
        assert_eq!(bytes_of("0xCAFE"), Some(vec![0xca, 0xfe]));
        // Not hex, odd, or empty: it is a text.
        assert_eq!(bytes_of("0xZZ"), None);
        assert_eq!(bytes_of("\\xabc"), None);
        assert_eq!(bytes_of("hello"), None);
        assert_eq!(bytes_of("0x"), None);
    }

    #[test]
    fn the_kind_and_the_dump_are_told() {
        let png = bytes_of("\\x89504e470d0a1a0a0000").unwrap();
        assert_eq!(kind_of(&png), Some(("PNG", "image/png")));
        assert!(image_svg(&png)
            .unwrap()
            .contains("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!(kind_of(b"%PDF-1.7"), Some(("PDF", "application/pdf")));
        assert!(image_svg(b"%PDF-1.7").is_none());
        let dump = hex_dump(b"Hello, world! 0123456789");
        assert!(dump.starts_with(
            "00000000  48 65 6c 6c 6f 2c 20 77 6f 72 6c 64 21 20 30 31  |Hello, world! 01|\n"
        ));
        assert!(dump.contains("00000010  32 33 34 35 36 37 38 39"));
    }

    #[test]
    fn base64_is_padded() {
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
    }
}
