//! Media storage layout and sizing rules.
//!
//! Media is stored once by content hash under `MEDIA_ROOT`. Originals keep their bytes untouched at
//! `originals/<sha[0..2]>/<sha[2..4]>/<sha>.<ext>`; web-size copies for phones and kitchen displays live at
//! `web/<max_side>/<sha[0..2]>/<sha>.<ext>`. Paths use `/` on every platform so they can be stored in the
//! database and served as URLs.

/// Longest-side sizes of the web copies made for every stored image.
pub const WEB_SIZES: &[u32] = &[400, 1200];

/// Where an original with this hash and extension is stored, relative to `MEDIA_ROOT`.
pub fn original_path(sha256: &str, ext: &str) -> String {
    format!(
        "originals/{}/{}/{sha256}.{ext}",
        &sha256[0..2],
        &sha256[2..4]
    )
}

/// Where the web copy of an image at this size is stored, relative to `MEDIA_ROOT`.
pub fn derivative_path(sha256: &str, max_side: u32, ext: &str) -> String {
    format!("web/{max_side}/{}/{sha256}.{ext}", &sha256[0..2])
}

/// Dimensions that fit `width` x `height` within `max_side` on the longest side, keeping the aspect ratio.
/// Never upscales: an image already within the limit keeps its size.
pub fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_side || longest == 0 {
        return (width, height);
    }
    let scale = |side: u32| {
        ((side as u64 * max_side as u64 + longest as u64 / 2) / longest as u64).max(1) as u32
    };
    if width >= height {
        (max_side, scale(height))
    } else {
        (scale(width), max_side)
    }
}

/// An image format mise stores, detected from a file's first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// JPEG, stored as ".jpg".
    Jpeg,
    /// PNG.
    Png,
    /// WebP.
    Webp,
    /// GIF.
    Gif,
}

impl ImageKind {
    /// Detect the format from a file's leading bytes (magic numbers). Extensions are not trusted.
    pub fn sniff(head: &[u8]) -> Option<Self> {
        match head {
            [0xFF, 0xD8, 0xFF, ..] => Some(Self::Jpeg),
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some(Self::Png),
            [
                b'R',
                b'I',
                b'F',
                b'F',
                _,
                _,
                _,
                _,
                b'W',
                b'E',
                b'B',
                b'P',
                ..,
            ] => Some(Self::Webp),
            [b'G', b'I', b'F', b'8', ..] => Some(Self::Gif),
            _ => None,
        }
    }

    /// The content type it is served with: "image/jpeg".
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
        }
    }

    /// The extension mise stores this format under (one spelling per format).
    pub fn ext(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Gif => "gif",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ab12cd34ef567890ab12cd34ef567890ab12cd34ef567890ab12cd34ef567890";

    #[test]
    fn lays_out_paths_by_hash() {
        assert_eq!(
            original_path(SHA, "jpg"),
            format!("originals/ab/12/{SHA}.jpg")
        );
        assert_eq!(
            derivative_path(SHA, 400, "jpg"),
            format!("web/400/ab/{SHA}.jpg")
        );
        assert_eq!(
            derivative_path(SHA, 1200, "png"),
            format!("web/1200/ab/{SHA}.png")
        );
    }

    #[test]
    fn fits_longest_side_without_upscaling() {
        assert_eq!(fit_within(4000, 3000, 1200), (1200, 900));
        assert_eq!(fit_within(3000, 4000, 400), (300, 400));
        assert_eq!(fit_within(1000, 1000, 400), (400, 400));
        assert_eq!(fit_within(800, 600, 1200), (800, 600));
        assert_eq!(fit_within(1200, 10, 1200), (1200, 10));
        // Rounds to the nearest pixel and never collapses a side to zero.
        assert_eq!(fit_within(1001, 500, 400), (400, 200));
        assert_eq!(fit_within(10000, 3, 400), (400, 1));
        assert_eq!(fit_within(0, 0, 400), (0, 0));
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(
            ImageKind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            Some(ImageKind::Jpeg)
        );
        assert_eq!(
            ImageKind::sniff(b"\x89PNG\r\n\x1a\n...."),
            Some(ImageKind::Png)
        );
        assert_eq!(
            ImageKind::sniff(b"RIFF\x10\0\0\0WEBPVP8 "),
            Some(ImageKind::Webp)
        );
        assert_eq!(ImageKind::sniff(b"GIF89a"), Some(ImageKind::Gif));
        // A saved S3 error page named like an image.
        assert_eq!(ImageKind::sniff(b"<?xml version=\"1.0\"?><Error>"), None);
        assert_eq!(ImageKind::sniff(&[0xFF, 0xD8]), None);
        assert_eq!(ImageKind::Jpeg.ext(), "jpg");
    }
}
