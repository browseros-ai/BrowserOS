//! Neo: byte-compatible MJPEG framing shared by legacy and Neo HTTP routes.

use crate::Frame;
use bytes::{BufMut, Bytes, BytesMut};
use std::fmt::Write as _;

/// Neo: legacy clients use this fixed multipart boundary.
const BOUNDARY: &str = "frame";
/// Neo: the content type served by every MJPEG capture route.
pub const CONTENT_TYPE: &str = "multipart/x-mixed-replace; boundary=frame";

/// One MJPEG part: the boundary, the part headers the spec documents, the JPEG.
pub fn part(frame: &Frame) -> Bytes {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Tab-Id: {}\r\nX-Url: {}\r\nX-Frame-At: {}\r\n\r\n",
        frame.jpeg.len(),
        frame.tab_id,
        header_safe(&frame.url),
        frame.captured_at,
    );
    let mut part = BytesMut::with_capacity(head.len() + frame.jpeg.len() + 2);
    part.put_slice(head.as_bytes());
    part.put_slice(&frame.jpeg);
    part.put_slice(b"\r\n");
    part.freeze()
}

/// A URL as a part-header value: printable ASCII kept, anything else percent-encoded, so a
/// page URL can never end the header early or break a client's parser.
#[must_use]
pub fn header_safe(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_graphic() {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Frame, header_safe, part};

    #[test]
    fn a_part_carries_the_documented_headers_around_the_jpeg() {
        let part = part(&Frame {
            jpeg: b"JPEG"[..].into(),
            tab_id: 42,
            url: "https://example.com/a b\r\nX-Evil: 1".to_string(),
            captured_at: 1_800_000_000_123,
        });

        assert_eq!(
            part.as_ref(),
            &b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\nX-Tab-Id: 42\r\nX-Url: https://example.com/a%20b%0D%0AX-Evil:%201\r\nX-Frame-At: 1800000000123\r\n\r\nJPEG\r\n"[..]
        );
    }

    #[test]
    fn non_ascii_urls_are_percent_encoded() {
        assert_eq!(
            header_safe("https://例.com/é"),
            "https://%E4%BE%8B.com/%C3%A9"
        );
    }
}
