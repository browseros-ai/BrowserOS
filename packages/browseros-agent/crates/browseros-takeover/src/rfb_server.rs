//! The server's side of RFB 3.8 as bytes, as far as Neo's own RFB clients (the screen's
//! resizer and watcher, in `screen`) need it from Xvnc. Pure: the connection code reads bytes
//! into a buffer and asks here whether a whole item has arrived, so a TCP read that splits or
//! merges messages never changes what is parsed.
//!
//! ```text
//!  Xvnc ── "RFB 003.008\n" ───────────────▶ Neo ── "RFB 003.008\n" ───▶ Xvnc
//!  Xvnc ── security types [1] (None) ─────▶ Neo ── 1 ──────────────────▶ Xvnc
//!  Xvnc ── u32 SecurityResult (0 is OK) ──▶ Neo ── ClientInit 1 (shared) ▶ Xvnc
//!  Xvnc ── ServerInit: u16 w, u16 h, PIXEL_FORMAT[16], u32 len, name ─▶ Neo
//!  Neo ── SetEncodings [raw, ExtendedDesktopSize], a 1×1 FramebufferUpdateRequest ─▶ Xvnc
//!  Xvnc ── FramebufferUpdate: raw 1×1 pixels and ExtendedDesktopSize layouts ─▶ Neo
//! ```
//!
//! Neo asks only for raw pixels and layouts, so any other encoding or server message is not
//! RFB as Neo spoke it, and the connection closes.

use crate::ScreenSize;
use crate::limits::{MAX_DESKTOP_NAME, MAX_RECT_BYTES};

/// The protocol version, sent and expected both ways.
pub(crate) const VERSION: &[u8; 12] = b"RFB 003.008\n";

/// The security type Neo picks: None. Xvnc runs `-SecurityTypes None -localhost`.
pub(crate) const SECURITY_NONE: u8 = 1;

/// ClientInit's shared flag. Unshared, Xvnc would disconnect every other client, the owner's
/// viewer included.
pub(crate) const SHARED: u8 = 1;

const ENCODING_RAW: i32 = 0;

/// The pseudo-encoding that carries the screen layout: the server's own changes, and its
/// verdict on a client's SetDesktopSize.
const ENCODING_DESKTOP_LAYOUT: i32 = -308;

/// What one parse step found in the bytes received so far.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Parse<T> {
    /// Not a whole item yet: read more.
    Incomplete,
    /// A whole item, and how many bytes it took.
    Done(T, usize),
    /// Bytes that are not RFB 3.8 as Neo speaks it.
    Invalid(&'static str),
}

/// What ServerInit tells a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ServerInit {
    /// The screen's size now.
    pub(crate) size: ScreenSize,
    /// How many bytes each raw pixel takes.
    pub(crate) bytes_per_pixel: usize,
}

/// Who changed the layout, from the layout rectangle's x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutReason {
    /// The server itself (0), or a reason RFB does not define.
    Server,
    /// This client's SetDesktopSize (1): the verdict on it.
    ThisClient,
    /// Another client's SetDesktopSize (2), such as noVNC's `resize=remote`.
    OtherClient,
}

/// The id and flags of one screen in a layout. SetDesktopSize sends them back unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScreenEntry {
    pub(crate) id: u32,
    pub(crate) flags: u32,
}

/// One ExtendedDesktopSize rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DesktopLayout {
    pub(crate) reason: LayoutReason,
    /// The rectangle's y: 0 is success; anything else refused this client's request.
    pub(crate) status: u16,
    /// The framebuffer's size after the change.
    pub(crate) size: ScreenSize,
    /// The first screen listed, if any. Xvnc lists its one screen.
    pub(crate) first_screen: Option<ScreenEntry>,
}

/// What a FramebufferUpdateRequest asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateRequest {
    /// An update now (non-incremental).
    Now,
    /// An update once something changes (incremental). Xvnc holds it until then.
    OnChange,
}

pub(crate) fn parse_version(buffer: &[u8]) -> Parse<()> {
    match buffer.get(..VERSION.len()) {
        None => Parse::Incomplete,
        Some(version) if version == VERSION => Parse::Done((), VERSION.len()),
        Some(_) => Parse::Invalid("the screen does not speak RFB 3.8"),
    }
}

/// The security types the server offers: a count, then one byte each. A count of 0 is a
/// refusal (a reason string follows, which Neo does not need).
pub(crate) fn parse_security_types(buffer: &[u8]) -> Parse<()> {
    let Some(&count) = buffer.first() else {
        return Parse::Incomplete;
    };
    if count == 0 {
        return Parse::Invalid("the screen refused the connection");
    }
    let end = 1 + usize::from(count);
    match buffer.get(1..end) {
        None => Parse::Incomplete,
        Some(types) if types.contains(&SECURITY_NONE) => Parse::Done((), end),
        Some(_) => Parse::Invalid("the screen asks for a password"),
    }
}

/// The u32 SecurityResult, which RFB 3.8 sends even for None. 0 is OK.
pub(crate) fn parse_security_result(buffer: &[u8]) -> Parse<()> {
    match u32_at(buffer, 0) {
        None => Parse::Incomplete,
        Some(0) => Parse::Done((), 4),
        Some(_) => Parse::Invalid("the screen refused the connection"),
    }
}

/// `u16 width, u16 height, PIXEL_FORMAT[16], u32 name length, name`. The pixel format's first
/// byte is bits per pixel; the rest only matters to a client that draws.
pub(crate) fn parse_server_init(buffer: &[u8]) -> Parse<ServerInit> {
    let (Some(width), Some(height), Some(&bits_per_pixel), Some(name)) = (
        u16_at(buffer, 0),
        u16_at(buffer, 2),
        buffer.get(4),
        u32_at(buffer, 20),
    ) else {
        return Parse::Incomplete;
    };
    if bits_per_pixel == 0 || bits_per_pixel % 8 != 0 {
        return Parse::Invalid("the screen sent no usable pixel format");
    }
    let Some(name) = usize::try_from(name)
        .ok()
        .filter(|name| *name <= MAX_DESKTOP_NAME)
    else {
        return Parse::Invalid("the screen's name is too long");
    };
    let end = 24 + name;
    if buffer.len() < end {
        return Parse::Incomplete;
    }
    Parse::Done(
        ServerInit {
            size: ScreenSize { width, height },
            bytes_per_pixel: usize::from(bits_per_pixel / 8),
        },
        end,
    )
}

/// One server message, reduced to the layout it carried: the last ExtendedDesktopSize
/// rectangle of a FramebufferUpdate. Raw pixels, SetColourMapEntries, Bell and ServerCutText
/// are framed and skipped.
pub(crate) fn parse_server_message(
    buffer: &[u8],
    bytes_per_pixel: usize,
) -> Parse<Option<DesktopLayout>> {
    match buffer.first() {
        None => Parse::Incomplete,
        Some(0) => parse_update(buffer, bytes_per_pixel),
        // SetColourMapEntries: type, padding, u16 first colour, u16 count, 6 bytes per colour.
        Some(1) => match u16_at(buffer, 4) {
            None => Parse::Incomplete,
            Some(count) => skip(buffer, 6 + 6 * usize::from(count)),
        },
        // Bell.
        Some(2) => Parse::Done(None, 1),
        // ServerCutText: type, 3 bytes padding, u32 length, the text. Xvnc runs with
        // `-SendCutText=0`, so this is only framed.
        Some(3) => match u32_at(buffer, 4).and_then(|length| usize::try_from(length).ok()) {
            None => Parse::Incomplete,
            Some(length) if length > MAX_RECT_BYTES => {
                Parse::Invalid("the screen sent clipboard text too long to skip")
            }
            Some(length) => skip(buffer, 8 + length),
        },
        Some(_) => Parse::Invalid("the screen sent a message Neo did not ask for"),
    }
}

/// FramebufferUpdate: type, padding, u16 rectangle count, then each rectangle as
/// `u16 x, u16 y, u16 w, u16 h, i32 encoding` and its data.
fn parse_update(buffer: &[u8], bytes_per_pixel: usize) -> Parse<Option<DesktopLayout>> {
    let Some(count) = u16_at(buffer, 2) else {
        return Parse::Incomplete;
    };
    let mut at = 4;
    let mut layout = None;
    for _ in 0..count {
        let (Some(x), Some(y), Some(width), Some(height), Some(encoding)) = (
            u16_at(buffer, at),
            u16_at(buffer, at + 2),
            u16_at(buffer, at + 4),
            u16_at(buffer, at + 6),
            i32_at(buffer, at + 8),
        ) else {
            return Parse::Incomplete;
        };
        at += 12;
        match encoding {
            ENCODING_RAW => {
                let Some(pixels) = usize::from(width)
                    .checked_mul(usize::from(height))
                    .and_then(|area| area.checked_mul(bytes_per_pixel))
                    .filter(|bytes| *bytes <= MAX_RECT_BYTES)
                else {
                    return Parse::Invalid("the screen sent more pixels than Neo asked for");
                };
                at += pixels;
            }
            // `u8 screen count`, 3 bytes padding, then 16 bytes per screen:
            // `u32 id, u16 x, u16 y, u16 w, u16 h, u32 flags`.
            ENCODING_DESKTOP_LAYOUT => {
                let Some(&screens) = buffer.get(at) else {
                    return Parse::Incomplete;
                };
                let list = at + 4;
                let end = list + 16 * usize::from(screens);
                if buffer.len() < end {
                    return Parse::Incomplete;
                }
                let first_screen = (screens > 0)
                    .then(|| u32_at(buffer, list).zip(u32_at(buffer, list + 12)))
                    .flatten()
                    .map(|(id, flags)| ScreenEntry { id, flags });
                layout = Some(DesktopLayout {
                    reason: match x {
                        1 => LayoutReason::ThisClient,
                        2 => LayoutReason::OtherClient,
                        _ => LayoutReason::Server,
                    },
                    status: y,
                    size: ScreenSize { width, height },
                    first_screen,
                });
                at = end;
            }
            _ => return Parse::Invalid("the screen sent an encoding Neo did not ask for"),
        }
    }
    if buffer.len() < at {
        return Parse::Incomplete;
    }
    Parse::Done(layout, at)
}

/// A whole message of `length` bytes that carries no layout.
fn skip(buffer: &[u8], length: usize) -> Parse<Option<DesktopLayout>> {
    if buffer.len() < length {
        Parse::Incomplete
    } else {
        Parse::Done(None, length)
    }
}

/// SetEncodings: raw, every server's fallback, and ExtendedDesktopSize, the layout.
pub(crate) fn set_encodings() -> Vec<u8> {
    let mut message = vec![2, 0];
    message.extend_from_slice(&2u16.to_be_bytes());
    message.extend_from_slice(&ENCODING_RAW.to_be_bytes());
    message.extend_from_slice(&ENCODING_DESKTOP_LAYOUT.to_be_bytes());
    message
}

/// FramebufferUpdateRequest for the top-left pixel alone, so every reply stays a few bytes.
pub(crate) fn update_request(request: UpdateRequest) -> Vec<u8> {
    let incremental = match request {
        UpdateRequest::Now => 0,
        UpdateRequest::OnChange => 1,
    };
    let mut message = vec![3, incremental];
    for value in [0u16, 0, 1, 1] {
        message.extend_from_slice(&value.to_be_bytes());
    }
    message
}

/// SetDesktopSize with one screen at the origin: `fb 00 W H 01 00 | id 0 0 W H flags`, with
/// `screen`'s id and flags sent back as the server listed them.
pub(crate) fn set_desktop_size(size: ScreenSize, screen: ScreenEntry) -> Vec<u8> {
    let mut message = vec![251, 0];
    message.extend_from_slice(&size.width.to_be_bytes());
    message.extend_from_slice(&size.height.to_be_bytes());
    message.extend_from_slice(&[1, 0]);
    message.extend_from_slice(&screen.id.to_be_bytes());
    for value in [0, 0, size.width, size.height] {
        message.extend_from_slice(&value.to_be_bytes());
    }
    message.extend_from_slice(&screen.flags.to_be_bytes());
    message
}

fn u16_at(buffer: &[u8], at: usize) -> Option<u16> {
    let bytes = buffer.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes(bytes.try_into().ok()?))
}

fn u32_at(buffer: &[u8], at: usize) -> Option<u32> {
    let bytes = buffer.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

fn i32_at(buffer: &[u8], at: usize) -> Option<i32> {
    u32_at(buffer, at).map(|value| i32::from_be_bytes(value.to_be_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopLayout, LayoutReason, Parse, ScreenEntry, ServerInit, UpdateRequest,
        parse_security_result, parse_security_types, parse_server_init, parse_server_message,
        parse_version, set_desktop_size, set_encodings, update_request,
    };
    use crate::ScreenSize;

    const SIZE_390: ScreenSize = ScreenSize {
        width: 390,
        height: 844,
    };

    /// TigerVNC's ServerInit for a 1600×1000 screen at 32 bits per pixel, named "neo".
    fn server_init() -> Vec<u8> {
        let mut bytes = vec![0x06, 0x40, 0x03, 0xe8];
        bytes.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes.extend_from_slice(b"neo");
        bytes
    }

    /// A FramebufferUpdate with a raw 1×1 pixel, then a layout: reason, status, 390×844, one
    /// screen with id 7 and flags 0x10.
    fn update_with_layout(reason: u16, status: u16) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 2];
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0x00]);
        bytes.extend_from_slice(&reason.to_be_bytes());
        bytes.extend_from_slice(&status.to_be_bytes());
        bytes.extend_from_slice(&[0x01, 0x86, 0x03, 0x4c, 0xff, 0xff, 0xfe, 0xcc]);
        bytes.extend_from_slice(&[1, 0, 0, 0]);
        bytes.extend_from_slice(&[
            0, 0, 0, 7, 0, 0, 0, 0, 0x01, 0x86, 0x03, 0x4c, 0, 0, 0, 0x10,
        ]);
        bytes
    }

    /// Every proper prefix of `bytes` is incomplete, and the whole is one item of its length:
    /// a TCP read can stop anywhere.
    fn parses_only_whole<T: std::fmt::Debug + PartialEq>(
        bytes: &[u8],
        parse: impl Fn(&[u8]) -> Parse<T>,
        item: T,
    ) {
        for end in 0..bytes.len() {
            assert_eq!(parse(&bytes[..end]), Parse::Incomplete, "{end} bytes");
        }
        let mut longer = bytes.to_vec();
        longer.extend_from_slice(&[3, 1, 0, 0]);
        assert_eq!(parse(&longer), Parse::Done(item, bytes.len()));
    }

    #[test]
    fn the_handshake_reads_rfb_3_8_with_no_security() {
        parses_only_whole(b"RFB 003.008\n", parse_version, ());
        assert!(matches!(parse_version(b"RFB 003.003\n"), Parse::Invalid(_)));
        parses_only_whole(&[2, 2, 1], parse_security_types, ());
        assert!(matches!(parse_security_types(&[1, 2]), Parse::Invalid(_)));
        assert!(matches!(
            parse_security_types(&[0, 0, 0, 0, 4]),
            Parse::Invalid(_)
        ));
        parses_only_whole(&[0, 0, 0, 0], parse_security_result, ());
        assert!(matches!(
            parse_security_result(&[0, 0, 0, 1]),
            Parse::Invalid(_)
        ));
        parses_only_whole(
            &server_init(),
            parse_server_init,
            ServerInit {
                size: ScreenSize {
                    width: 1600,
                    height: 1000,
                },
                bytes_per_pixel: 4,
            },
        );
        let mut no_format = server_init();
        no_format[4] = 0;
        assert!(matches!(parse_server_init(&no_format), Parse::Invalid(_)));
    }

    #[test]
    fn an_update_yields_its_layout_and_skips_its_pixels() {
        let reasons = [
            (0, LayoutReason::Server),
            (1, LayoutReason::ThisClient),
            (2, LayoutReason::OtherClient),
        ];
        for (x, reason) in reasons {
            parses_only_whole(
                &update_with_layout(x, 0),
                |bytes| parse_server_message(bytes, 4),
                Some(DesktopLayout {
                    reason,
                    status: 0,
                    size: SIZE_390,
                    first_screen: Some(ScreenEntry { id: 7, flags: 0x10 }),
                }),
            );
        }
        let refused = parse_server_message(&update_with_layout(1, 3), 4);
        assert!(
            matches!(
                refused,
                Parse::Done(Some(DesktopLayout { status: 3, .. }), _)
            ),
            "a refusal parses; the resizer decides it is an error: {refused:?}"
        );
        let pixel_only = [0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4];
        parses_only_whole(&pixel_only, |bytes| parse_server_message(bytes, 4), None);
    }

    #[test]
    fn other_server_messages_are_framed_and_unknown_ones_are_invalid() {
        parses_only_whole(&[2], |bytes| parse_server_message(bytes, 4), None);
        let colours = [1, 0, 0, 0, 0, 1, 0, 1, 0, 2, 0, 3];
        parses_only_whole(&colours, |bytes| parse_server_message(bytes, 4), None);
        let cut_text = [3, 0, 0, 0, 0, 0, 0, 2, b'h', b'i'];
        parses_only_whole(&cut_text, |bytes| parse_server_message(bytes, 4), None);
        for unknown in [&[150, 0][..], &[248, 0, 0, 0][..], &[7][..]] {
            assert!(
                matches!(parse_server_message(unknown, 4), Parse::Invalid(_)),
                "{unknown:?}"
            );
        }
        let tight = [0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 7];
        assert!(
            matches!(parse_server_message(&tight, 4), Parse::Invalid(_)),
            "an encoding Neo did not declare"
        );
        let huge = [0, 0, 0, 1, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0];
        assert!(
            matches!(parse_server_message(&huge, 4), Parse::Invalid(_)),
            "a rectangle far beyond the 1×1 Neo asked for"
        );
    }

    #[test]
    fn the_client_messages_have_rfbs_bytes() {
        assert_eq!(
            set_encodings(),
            [2, 0, 0, 2, 0, 0, 0, 0, 0xff, 0xff, 0xfe, 0xcc]
        );
        assert_eq!(
            update_request(UpdateRequest::Now),
            [3, 0, 0, 0, 0, 0, 0, 1, 0, 1]
        );
        assert_eq!(
            update_request(UpdateRequest::OnChange),
            [3, 1, 0, 0, 0, 0, 0, 1, 0, 1]
        );
        assert_eq!(
            set_desktop_size(
                SIZE_390,
                ScreenEntry {
                    id: 0x0102_0304,
                    flags: 0x10
                }
            ),
            [
                0xfb, 0, 0x01, 0x86, 0x03, 0x4c, 1, 0, 0x01, 0x02, 0x03, 0x04, 0, 0, 0, 0, 0x01,
                0x86, 0x03, 0x4c, 0, 0, 0, 0x10,
            ]
        );
    }
}
