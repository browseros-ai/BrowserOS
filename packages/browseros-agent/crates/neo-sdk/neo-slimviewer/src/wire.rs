//! Each record starts with the remaining length as a big-endian u32, then a type byte and payload.
//! The page reassembles records across WebSocket chunks so the host can relay the TCP byte stream.

use std::io::{self, Read};

use crate::rfb::{FieldBox, Fields, PasteOutcome};

const HELLO: u8 = 0x01;
const FRAME: u8 = 0x02;
const CLIP: u8 = 0x03;
const PASTE_RESULT: u8 = 0x04;
const FIELDS: u8 = 0x05;
const ACK: u8 = 0x10;
const KEYFRAME: u8 = 0x11;
const POINTER: u8 = 0x20;
const KEY: u8 = 0x21;
const PASTE: u8 = 0x22;
const TYPE: u8 = 0x23;
const CLIPBOARD_WATCH: u8 = 0x24;
const WHEEL: u8 = 0x25;
const PASTE_TX: u8 = 0x26;
const FIELDS_WATCH: u8 = 0x27;
const VERSION: u8 = 1;
const SUPPORTS_TYPE: u8 = 1 << 1;
const SUPPORTS_COPY: u8 = 1 << 2;
const SUPPORTS_WHEEL: u8 = 1 << 3;
const SUPPORTS_PASTE_RESULT: u8 = 1 << 4;
const SUPPORTS_FIELDS: u8 = 1 << 5;
/// Text is capped independently of the enclosing record's transaction id.
pub(crate) const MAX_PASTE_BYTES: usize = 64 * 1024;
const MAX_FROM_PAGE: usize = MAX_PASTE_BYTES + 5;

pub enum FromPage {
    Ack(u32),
    Keyframe,
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { sym: u32, down: bool },
    Paste(String),
    PasteTx { id: u32, text: String },
    Type(String),
    ClipboardWatch,
    FieldsWatch,
    Wheel { x: u16, y: u16, dx: f32, dy: f32 },
}

pub fn paste_result(id: u32, outcome: PasteOutcome) -> Vec<u8> {
    let mut out = 6u32.to_be_bytes().to_vec();
    out.push(PASTE_RESULT);
    out.extend(id.to_be_bytes());
    out.push(outcome as u8);
    out
}

pub fn fields(f: &Fields) -> Vec<u8> {
    let mut out = ((7 + 8 * f.boxes().len()) as u32).to_be_bytes().to_vec();
    out.push(FIELDS);
    out.extend(f.width().to_be_bytes());
    out.extend(f.height().to_be_bytes());
    out.extend([f.focused().unwrap_or(0xff), f.boxes().len() as u8]);
    for FieldBox { x, y, w, h } in f.boxes() {
        for value in [x, y, w, h] {
            out.extend(value.to_be_bytes());
        }
    }
    out
}

pub fn hello(view_only: bool) -> Vec<u8> {
    let mut out = 3u32.to_be_bytes().to_vec();
    // Revocation also uses HELLO; older pages recognize its exact view-only flag.
    out.extend([
        HELLO,
        VERSION,
        if view_only {
            1
        } else {
            SUPPORTS_TYPE | SUPPORTS_COPY | SUPPORTS_WHEEL | SUPPORTS_PASTE_RESULT | SUPPORTS_FIELDS
        },
    ]);
    out
}

/// Copy-out is text only; the owner identity is checked by the engine before delivery.
pub fn clipboard(text: &str) -> io::Result<Vec<u8>> {
    if text.len() > MAX_PASTE_BYTES || text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid clipboard text",
        ));
    }
    let mut out = ((text.len() + 1) as u32).to_be_bytes().to_vec();
    out.push(CLIP);
    out.extend_from_slice(text.as_bytes());
    Ok(out)
}

/// One decodable frame: header, then the Annex B access unit. `half`: the picture is half the
/// screen's width and height (the page draws it scaled ×2).
pub fn frame(
    seq: u32,
    key: bool,
    sharp: bool,
    half: bool,
    width: u16,
    height: u16,
    au: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + au.len());
    out.extend(((10 + au.len()) as u32).to_be_bytes());
    out.push(FRAME);
    out.extend(seq.to_be_bytes());
    out.push(u8::from(key) | (u8::from(sharp) << 1) | (u8::from(half) << 2));
    out.extend(width.to_be_bytes());
    out.extend(height.to_be_bytes());
    out.extend_from_slice(au);
    out
}

/// Reads one record from the page. Ok(None): a record this connection ignores (unknown, or input
/// on the view-only port). Err ends the connection.
pub fn read(from: &mut impl Read, view_only: bool) -> io::Result<Option<FromPage>> {
    let mut len = [0u8; 4];
    from.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FROM_PAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad record length",
        ));
    }
    let mut buf = vec![0; len];
    from.read_exact(&mut buf)?;
    let (kind, p) = (buf[0], &buf[1..]);
    let be16 = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]);
    let be32 = |i: usize| u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    let msg = match (kind, p.len()) {
        (ACK, 4) => FromPage::Ack(be32(0)),
        (KEYFRAME, 0) => FromPage::Keyframe,
        (POINTER, 5) => FromPage::Pointer {
            x: be16(0),
            y: be16(2),
            buttons: p[4],
        },
        (KEY, 5) if p[4] <= 1 => FromPage::Key {
            sym: be32(0),
            down: p[4] != 0,
        },
        (CLIPBOARD_WATCH, 0) => FromPage::ClipboardWatch,
        (FIELDS_WATCH, 0) => FromPage::FieldsWatch,
        (WHEEL, 12) => {
            let dx = f32::from_bits(be32(4));
            let dy = f32::from_bits(be32(8));
            if ![dx, dy].iter().all(|d| d.is_finite() && d.abs() <= 16384.0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid wheel delta",
                ));
            }
            FromPage::Wheel {
                x: be16(0),
                y: be16(2),
                dx,
                dy,
            }
        }
        (PASTE_TX, 5..) if be32(0) != 0 => FromPage::PasteTx {
            id: be32(0),
            text: text(&p[4..])?,
        },
        (PASTE | TYPE, _) => {
            let text = text(p)?;
            if kind == PASTE {
                FromPage::Paste(text)
            } else {
                FromPage::Type(text)
            }
        }
        (ACK | KEYFRAME | POINTER | KEY | CLIPBOARD_WATCH | WHEEL | PASTE_TX | FIELDS_WATCH, _) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed record",
            ));
        }
        _ => return Ok(None),
    };
    // The spectator port is the trusted filter: whatever the page sends, only acks get through.
    if view_only && !matches!(msg, FromPage::Ack(_) | FromPage::Keyframe) {
        return Ok(None);
    }
    Ok(Some(msg))
}

fn text(bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_PASTE_BYTES || bytes.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid text"));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "text is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 1) as u32).to_be_bytes().to_vec();
        out.push(kind);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn tests_that_checked_paste_and_field_watch_are_owner_only() {
        let mut payload = 0x01020304u32.to_be_bytes().to_vec();
        payload.extend_from_slice("café 👍🏽".as_bytes());
        for record in [rec(0x26, &payload), rec(0x27, &[])] {
            assert!(read(&mut &record[..], false).unwrap().is_some());
            assert!(read(&mut &record[..], true).unwrap().is_none());
        }
    }

    #[test]
    fn tests_that_checked_paste_requires_an_id_and_nonempty_valid_text() {
        for payload in [
            vec![],
            vec![0; 3],
            vec![0, 0, 0, 1],
            vec![0, 0, 0, 0, b'a'],
            vec![0, 0, 0, 1, 0xff],
            vec![0, 0, 0, 1, b'a', 0],
        ] {
            assert!(read(&mut &rec(0x26, &payload)[..], false).is_err());
        }
        assert!(read(&mut &rec(0x27, &[0])[..], false).is_err());
    }

    #[test]
    fn tests_that_checked_paste_fits_64_kib_without_widening_legacy_text() {
        let mut payload = 1u32.to_be_bytes().to_vec();
        payload.extend(vec![b'a'; MAX_PASTE_BYTES]);
        assert!(
            read(&mut &rec(0x26, &payload)[..], false)
                .unwrap()
                .is_some()
        );
        payload.push(b'a');
        assert!(read(&mut &rec(0x26, &payload)[..], false).is_err());
        for kind in [PASTE, TYPE] {
            assert!(read(&mut &rec(kind, &vec![b'a'; MAX_PASTE_BYTES])[..], false).is_ok());
            for extra in 1..=4 {
                assert!(
                    read(
                        &mut &rec(kind, &vec![b'a'; MAX_PASTE_BYTES + extra])[..],
                        false
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn tests_that_transaction_ids_and_utf8_survive_back_to_back_records() {
        let mut payload = 0xf1234567u32.to_be_bytes().to_vec();
        payload.extend_from_slice("नमस्ते 👋🏽".as_bytes());
        let mut bytes = rec(0x26, &payload);
        bytes.extend(rec(0x27, &[]));
        bytes.extend(rec(ACK, &42u32.to_be_bytes()));
        let mut reader = &bytes[..];
        assert!(
            matches!(read(&mut reader, false).unwrap(), Some(FromPage::PasteTx { id: 0xf1234567, text }) if text == "नमस्ते 👋🏽")
        );
        assert!(matches!(
            read(&mut reader, false).unwrap(),
            Some(FromPage::FieldsWatch)
        ));
        assert!(matches!(
            read(&mut reader, false).unwrap(),
            Some(FromPage::Ack(42))
        ));
    }

    #[test]
    fn tests_that_paste_outcomes_keep_their_frozen_wire_bytes() {
        use crate::rfb::PasteOutcome;
        for (outcome, byte) in [
            (PasteOutcome::Pasted, 0),
            (PasteOutcome::NotLanded, 1),
            (PasteOutcome::NoField, 2),
            (PasteOutcome::Refused, 3),
            (PasteOutcome::Unconfirmed, 4),
        ] {
            assert_eq!(
                paste_result(0x01020304, outcome),
                [0, 0, 0, 6, 0x04, 1, 2, 3, 4, byte]
            );
            assert_eq!(outcome as u8, byte);
        }
        assert_eq!(SUPPORTS_PASTE_RESULT, 16);
        assert_eq!(SUPPORTS_FIELDS, 32);
        assert_eq!(
            hello(false)[6] & (SUPPORTS_PASTE_RESULT | SUPPORTS_FIELDS),
            SUPPORTS_PASTE_RESULT | SUPPORTS_FIELDS
        );
        assert_eq!(hello(true)[6], 1);
    }

    #[test]
    fn tests_that_field_records_encode_dimensions_focus_and_boxes_exactly() {
        use crate::rfb::{FieldBox, Fields};
        let b = FieldBox {
            x: 258,
            y: 515,
            w: 30,
            h: 40,
        };
        let f = Fields::new(780, 1688, Some(0), vec![b]).unwrap();
        assert_eq!(
            fields(&f),
            [
                0, 0, 0, 15, 0x05, 3, 12, 6, 152, 0, 1, 1, 2, 2, 3, 0, 30, 0, 40
            ]
        );
        assert_eq!(
            fields(&Fields::new(780, 1688, None, vec![]).unwrap()),
            [0, 0, 0, 7, 0x05, 3, 12, 6, 152, 255, 0]
        );
        let bytes = fields(&Fields::new(780, 1688, Some(127), vec![b; 128]).unwrap());
        assert_eq!(
            u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize,
            7 + 128 * 8
        );
        assert_eq!(&bytes[9..11], &[127, 128]);
        for box_bytes in bytes[11..].as_chunks::<8>().0 {
            assert_eq!(*box_bytes, [1, 2, 2, 3, 0, 30, 0, 40]);
        }
    }

    #[test]
    fn desktop_records_are_bounded_and_spectators_cannot_request_them() {
        let mut wheel = [0, 10, 0, 20].to_vec();
        wheel.extend(1.25f32.to_be_bytes());
        wheel.extend((-2.5f32).to_be_bytes());
        assert!(matches!(
            read(&mut &rec(WHEEL, &wheel)[..], false).unwrap(),
            Some(FromPage::Wheel {
                x: 10,
                y: 20,
                dx: 1.25,
                dy: -2.5
            })
        ));
        for record in [rec(WHEEL, &wheel), rec(CLIPBOARD_WATCH, &[])] {
            assert!(read(&mut &record[..], true).unwrap().is_none());
        }
        assert!(matches!(
            read(&mut &rec(CLIPBOARD_WATCH, &[])[..], false).unwrap(),
            Some(FromPage::ClipboardWatch)
        ));
        assert!(read(&mut &rec(CLIPBOARD_WATCH, &[0])[..], false).is_err());
        for end in 0..12 {
            assert!(read(&mut &rec(WHEEL, &wheel[..end])[..], false).is_err());
        }
        for value in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            16384.1,
            -16384.1,
        ] {
            wheel[4..8].copy_from_slice(&value.to_be_bytes());
            assert!(read(&mut &rec(WHEEL, &wheel)[..], false).is_err());
        }
    }

    #[test]
    fn clipboard_text_keeps_unicode_and_enforces_the_byte_limit() {
        assert_eq!(
            clipboard("日本 👍").unwrap(),
            rec(CLIP, "日本 👍".as_bytes())
        );
        assert_eq!(clipboard("").unwrap(), rec(CLIP, b""));
        assert!(clipboard(&"a".repeat(MAX_PASTE_BYTES)).is_ok());
        assert!(clipboard(&"a".repeat(MAX_PASTE_BYTES + 1)).is_err());
        assert!(clipboard("secret\0suffix").is_err());
    }

    #[test]
    fn committed_text_is_exact_bounded_and_owner_only() {
        for text in ["नमस्ते 👋🏽", "", &"a".repeat(MAX_PASTE_BYTES)] {
            let bytes = rec(TYPE, text.as_bytes());
            assert!(
                matches!(read(&mut &bytes[..], false).unwrap(), Some(FromPage::Type(s)) if s == text)
            );
            assert!(read(&mut &bytes[..], true).unwrap().is_none());
            assert!(
                read(&mut &rec(PASTE, text.as_bytes())[..], true)
                    .unwrap()
                    .is_none()
            );
        }
        for kind in [TYPE, PASTE] {
            for invalid in [&[0xff][..], b"a\0b", &vec![b'a'; MAX_PASTE_BYTES + 1]] {
                assert!(read(&mut &rec(kind, invalid)[..], false).is_err());
            }
        }
    }

    #[test]
    fn tests_that_malformed_known_records_are_rejected() {
        for (kind, payload) in [
            (ACK, vec![0; 3]),
            (KEYFRAME, vec![0]),
            (KEY, vec![0; 4]),
            (POINTER, vec![0; 6]),
        ] {
            let bytes = rec(kind, &payload);
            assert!(read(&mut &bytes[..], false).is_err(), "kind {kind}");
        }
    }

    #[test]
    fn tests_that_invalid_utf8_and_key_states_are_rejected() {
        for bytes in [rec(PASTE, &[0xff]), rec(KEY, &[0, 0, 0, 97, 2])] {
            assert!(read(&mut &bytes[..], false).is_err());
        }
    }

    #[test]
    fn tests_that_record_limits_apply_before_allocation() {
        for len in [0, MAX_FROM_PAGE as u32 + 1, u32::MAX] {
            assert_eq!(
                read(&mut &len.to_be_bytes()[..], false)
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
        let bytes = rec(PASTE, &vec![b'a'; MAX_PASTE_BYTES]);
        assert!(
            matches!(read(&mut &bytes[..], false).unwrap(), Some(FromPage::Paste(s)) if s.len() == MAX_PASTE_BYTES)
        );
    }

    #[test]
    fn tests_that_all_truncations_fail_without_panicking() {
        for bytes in [
            rec(ACK, &[0, 0, 0, 7]),
            rec(KEYFRAME, &[]),
            rec(POINTER, &[0; 5]),
            rec(KEY, &[0; 5]),
            rec(PASTE, b"hello"),
        ] {
            for end in 0..bytes.len() {
                assert!(read(&mut &bytes[..end], false).is_err());
            }
        }
    }

    #[test]
    fn tests_that_unknown_records_preserve_framing() {
        for kind in 0..=u8::MAX {
            if [
                ACK,
                KEYFRAME,
                POINTER,
                KEY,
                PASTE,
                TYPE,
                CLIPBOARD_WATCH,
                WHEEL,
                0x26,
                0x27,
            ]
            .contains(&kind)
            {
                continue;
            }
            let mut bytes = rec(kind, &[0xff; 17]);
            bytes.extend(rec(ACK, &42u32.to_be_bytes()));
            let mut reader = &bytes[..];
            assert!(read(&mut reader, false).unwrap().is_none());
            assert!(matches!(
                read(&mut reader, false).unwrap(),
                Some(FromPage::Ack(42))
            ));
        }
    }

    #[test]
    fn tests_that_hello_advertises_owner_type_support_without_changing_roles_or_version() {
        for view_only in [false, true] {
            let bytes = hello(view_only);
            assert_eq!(bytes.len(), 7);
            assert_eq!(bytes[5], 1);
            assert_eq!(bytes[6] & 1, u8::from(view_only));
            if !view_only {
                assert_ne!(bytes[6] & 2, 0, "TYPE capability must be advertised");
            }
        }
    }

    #[test]
    fn tests_that_revocation_flags_remain_compatible_with_older_pages() {
        // Older pages recognize this repeated HELLO by its exact flags byte.
        assert_eq!(hello(true)[6], 1);
    }

    #[test]
    fn tests_that_hello_and_all_frame_flags_match_wire_v1() {
        assert_eq!(hello(false), [0, 0, 0, 3, HELLO, VERSION, 62]);
        assert_eq!(hello(true), [0, 0, 0, 3, HELLO, VERSION, 1]);
        let bytes = frame(u32::MAX, true, true, true, 640, 480, &[1, 2, 3]);
        assert_eq!(&bytes[..4], &13u32.to_be_bytes());
        assert_eq!(bytes[9], 7);
        assert_eq!(&bytes[14..], &[1, 2, 3]);
    }

    #[test]
    fn view_only_port_keeps_acks_and_drops_input() {
        let mut stream: Vec<u8> = rec(POINTER, &[0, 10, 0, 20, 1]);
        stream.extend(rec(KEY, &[0, 0, 0, 0x61, 1]));
        stream.extend(rec(ACK, &[0, 0, 0, 7]));
        let mut r = &stream[..];
        assert!(read(&mut r, true).unwrap().is_none());
        assert!(read(&mut r, true).unwrap().is_none());
        assert!(matches!(
            read(&mut r, true).unwrap(),
            Some(FromPage::Ack(7))
        ));
        let mut r = &stream[..];
        assert!(matches!(
            read(&mut r, false).unwrap(),
            Some(FromPage::Pointer {
                x: 10,
                y: 20,
                buttons: 1
            })
        ));
    }

    #[test]
    fn frame_header_matches_the_table() {
        let f = frame(5, true, false, false, 780, 1688, &[0, 0, 0, 1, 0x67]);
        assert_eq!(&f[..4], &15u32.to_be_bytes());
        assert_eq!(f[4], FRAME);
        assert_eq!(&f[5..9], &5u32.to_be_bytes());
        assert_eq!(f[9], 1);
        assert_eq!(&f[10..14], &[3, 12, 6, 152]);
    }
}
