//! `ImageFormat`, `sniff` and `is_image` from magic bytes.

/// Only this many leading bytes are inspected.
const SNIFF_LEN: usize = 32;

/// The seven image types of Kotlin's `ImageType`. The extension of JPEG is `jpg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Avif,
    Gif,
    Heif,
    Jpeg,
    Jxl,
    Png,
    Webp,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 7] = [
        ImageFormat::Avif,
        ImageFormat::Gif,
        ImageFormat::Heif,
        ImageFormat::Jpeg,
        ImageFormat::Jxl,
        ImageFormat::Png,
        ImageFormat::Webp,
    ];

    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Avif => "image/avif",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Heif => "image/heif",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Jxl => "image/jxl",
            ImageFormat::Png => "image/png",
            ImageFormat::Webp => "image/webp",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Avif => "avif",
            ImageFormat::Gif => "gif",
            ImageFormat::Heif => "heif",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Jxl => "jxl",
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
        }
    }
}

/// The format of an image from its first bytes. Only the first 32 bytes are read.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    let head = &bytes[..bytes.len().min(SNIFF_LEN)];
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(ImageFormat::Gif);
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    if head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\x00\x00\x00\x0CJXL \x0D\x0A\x87\x0A")
    {
        return Some(ImageFormat::Jxl);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"avif" | b"avis" => Some(ImageFormat::Avif),
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                Some(ImageFormat::Heif)
            }
            _ => None,
        };
    }
    None
}

/// Whether a file is an image. A missing name is not an image. Otherwise the
/// extension after the last `.` (case-sensitive) decides, and sniffing the bytes is the fallback.
pub fn is_image(name: Option<&str>, bytes: &[u8]) -> bool {
    let Some(name) = name else {
        return false;
    };
    let extension = name.rsplit('.').next().unwrap_or(name);
    ImageFormat::ALL.iter().any(|f| f.extension() == extension) || sniff(bytes).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0];

    #[test]
    fn table_matches_kotlin_image_type() {
        let rows: Vec<(&str, &str)> = ImageFormat::ALL
            .iter()
            .map(|f| (f.mime(), f.extension()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("image/avif", "avif"),
                ("image/gif", "gif"),
                ("image/heif", "heif"),
                ("image/jpeg", "jpg"),
                ("image/jxl", "jxl"),
                ("image/png", "png"),
                ("image/webp", "webp"),
            ]
        );
    }

    #[test]
    fn sniffs_each_magic_number() {
        assert_eq!(sniff(JPEG), Some(ImageFormat::Jpeg));
        assert_eq!(
            sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(ImageFormat::Png)
        );
        assert_eq!(sniff(b"GIF89a\x01\x00"), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"GIF87a\x01\x00"), Some(ImageFormat::Gif));
        assert_eq!(sniff(b"RIFF\x24\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(sniff(&[0xFF, 0x0A, 0x00]), Some(ImageFormat::Jxl));
        assert_eq!(
            sniff(b"\x00\x00\x00\x0CJXL \x0D\x0A\x87\x0A"),
            Some(ImageFormat::Jxl)
        );
        assert_eq!(sniff(b"\0\0\0\x1Cftypavif\0\0"), Some(ImageFormat::Avif));
        assert_eq!(sniff(b"\0\0\0\x1Cftypavis\0\0"), Some(ImageFormat::Avif));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0"), Some(ImageFormat::Heif));
        assert_eq!(sniff(b"\0\0\0\x18ftypmif1\0\0"), Some(ImageFormat::Heif));
    }

    #[test]
    fn unknown_bytes_sniff_to_none() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"BM\x00\x00"), None);
        assert_eq!(sniff(b"\0\0\0\x1Cftypmp42\0\0"), None);
        assert_eq!(sniff(b"RIFF\x24\0\0\0WAVEfmt "), None);
        assert_eq!(sniff(b"\xFF"), None);
    }

    #[test]
    fn only_the_first_32_bytes_count() {
        let mut bytes = vec![b'x'; 40];
        bytes[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        assert_eq!(sniff(&bytes), Some(ImageFormat::Jpeg));
        let mut late = vec![b'x'; 40];
        late[32..35].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        assert_eq!(sniff(&late), None);
    }

    #[test]
    fn name_extension_is_case_sensitive() {
        assert!(is_image(Some("page.png"), b"not an image"));
        assert!(is_image(Some("page.webp"), b""));
        assert!(!is_image(Some("page.PNG"), b"not an image"));
    }

    #[test]
    fn sniffing_is_the_fallback_for_other_names() {
        assert!(is_image(Some("photo.JPG"), JPEG));
        assert!(is_image(Some("cover.jpeg"), JPEG));
        assert!(!is_image(Some("cover.jpeg"), b"text"));
        assert!(!is_image(Some("notes.txt"), b"text"));
    }

    #[test]
    fn missing_name_is_never_an_image() {
        assert!(!is_image(None, JPEG));
    }

    #[test]
    fn name_without_dot_uses_the_whole_name_as_extension() {
        assert!(is_image(Some("jpg"), b""));
        assert!(is_image(Some("cover"), JPEG));
        assert!(!is_image(Some("cover"), b"text"));
    }
}
