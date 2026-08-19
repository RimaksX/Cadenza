//! What counts as a picture this application can show.

/// True when the bytes begin the way one of the formats we can draw begins.
///
/// Sniffed rather than trusted to a file extension, because both places these
/// bytes come from are untrustworthy in the same way: a tag inside an audio
/// file carries whatever the tagger put there, and a file the listener chose
/// carries whatever it was named. What the window can actually draw is decided
/// by content or not at all.
///
/// A rule rather than an adapter detail, which is why it lives here: the same
/// answer has to hold for the reader that pulls a cover out of a tag and for
/// the service that accepts one from a chooser.
#[must_use]
pub fn looks_like_an_image(bytes: &[u8]) -> bool {
    image_extension(bytes).is_some()
}

/// The extension bytes like these should be stored under.
///
/// Sniffed from the content and then written into the *name*, because the thing
/// that finally draws the picture decides what it is looking at by extension —
/// a cover stored under a name of our own invention is a cover nothing can
/// open, however good the bytes are.
#[must_use]
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    const JPEG: [u8; 3] = [0xFF, 0xD8, 0xFF];
    const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const GIF: [u8; 3] = [b'G', b'I', b'F'];
    const BMP: [u8; 2] = [b'B', b'M'];

    if bytes.starts_with(&JPEG) {
        Some("jpg")
    } else if bytes.starts_with(&PNG) {
        Some("png")
    } else if bytes.starts_with(&GIF) {
        Some("gif")
    } else if bytes.starts_with(&BMP) {
        Some("bmp")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Every extension a stored cover can have, for finding one again.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "gif", "bmp", "webp"];

#[cfg(test)]
mod tests {
    use super::looks_like_an_image;

    #[test]
    fn a_format_is_named_so_that_what_draws_it_can_tell() {
        assert_eq!(super::image_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(super::image_extension(b"BMxx"), Some("bmp"));
        assert_eq!(super::image_extension(b"RIFF____WEBP...."), Some("webp"));
        assert_eq!(super::image_extension(b"nothing"), None);

        // Every name it can produce is one the finder knows to look for.
        for bytes in [&b"GIF89a"[..], &b"BM"[..], &b"RIFF____WEBP"[..]] {
            let extension = super::image_extension(bytes).expect("a format");
            assert!(super::IMAGE_EXTENSIONS.contains(&extension));
        }
    }

    #[test]
    fn the_formats_we_can_draw_are_recognised_by_their_first_bytes() {
        assert!(looks_like_an_image(b"GIF89a and then anything"));
        assert!(looks_like_an_image(b"BM and then anything"));
        assert!(looks_like_an_image(b"RIFF____WEBPand then anything"));
        assert!(!looks_like_an_image(b"not a picture at all"));
        assert!(!looks_like_an_image(b""));
        // Truncated to less than the header it claims.
        assert!(!looks_like_an_image(b"RIFF__"));
    }
}
