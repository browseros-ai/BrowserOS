//! The client's side of RFB 3.8, as Neo's display reads what a viewer sends to Xvnc: the bytes
//! cut into whole messages, and the one table of what each display mode lets through (design
//! D12, decision 23). Pure: the proxy pushes whatever a WebSocket frame carried and takes whole
//! messages out, so frames that split or merge messages never change what is judged.
//!
//! ```text
//!  viewer ── "RFB 003.008\n" ── security type 1 (None) ── ClientInit (sent on as 1, shared)
//!         ── then messages: type byte, fixed or self-described length ── verdict(mode, message)
//! ```
//!
//! **The filter fails closed.** Xvnc has no auth, and anything forwarded reaches the browser as
//! the owner's own hands. So bytes this module does not know close the display instead of being
//! skipped: an unknown type, a QEMU subtype other than the key event, a length past its cap, a
//! handshake other than RFB 3.8 with no security. A gap in this parser then disconnects a
//! viewer; it never lets input through a view display (design R6). Skipping an unknown message
//! would need its length, which only knowing the message gives.

use crate::limits::{MAX_DESKTOP_SCREENS, MAX_PASTE};
use crate::rfb_server::{Parse, SECURITY_NONE, SHARED, VERSION, i32_at, u16_at};
use bytes::{Bytes, BytesMut};

/// What a display lets its viewer do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayMode {
    /// A spectator's: the screen only. Keys, clicks, pastes and resizes are dropped.
    View,
    /// The owner's: everything goes to Xvnc, and input first waits for the taint.
    Control,
}

/// One whole message a viewer sent, by what it is. Only messages the filter knows exist here,
/// so every value has a verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientMessage {
    /// `"RFB 003.008\n"`.
    Version,
    /// The security type the viewer picked: 1 (None), the only one Xvnc offers.
    Security,
    /// One byte, sent on as 1 (shared) whatever the viewer asked, so one viewer can never
    /// disconnect the others, the owner included.
    ClientInit,
    /// Type 0.
    SetPixelFormat,
    /// Type 2.
    SetEncodings,
    /// Type 3.
    UpdateRequest,
    /// Type 4: a key, by keysym.
    Key,
    /// Type 5, with its button mask. A mask of 0 is a hover move.
    Pointer {
        /// The buttons held down.
        buttons: u8,
    },
    /// Type 6: a paste, plain or extended.
    CutText,
    /// Type 150.
    ContinuousUpdates,
    /// Type 248.
    Fence,
    /// Type 251: the viewer asks for a screen size, as noVNC's `resize=remote` does.
    SetDesktopSize,
    /// Type 255, subtype 0: QEMU's extended key event, a key with its scancode. noVNC 1.3.0
    /// sends every key that has a scancode this way, never as type 4.
    QemuKey,
}

/// What a display does with one message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Send it on to Xvnc.
    Pass,
    /// Swallow it.
    Drop,
    /// Send it on, as the owner's input: before the first one, every open tab goes private.
    Input,
}

/// The decided table (D12, decision 23), and the only list of what each mode allows. A match on
/// every pair, so a message added to [`ClientMessage`] cannot compile without a verdict here.
pub(crate) const fn verdict(mode: DisplayMode, message: ClientMessage) -> Verdict {
    use ClientMessage::{
        ClientInit, ContinuousUpdates, CutText, Fence, Key, Pointer, QemuKey, Security,
        SetDesktopSize, SetEncodings, SetPixelFormat, UpdateRequest, Version,
    };
    match (mode, message) {
        (
            _,
            Version | Security | ClientInit | SetPixelFormat | SetEncodings | UpdateRequest
            | ContinuousUpdates | Fence,
        ) => Verdict::Pass,
        (DisplayMode::View, Key | Pointer { .. } | CutText | SetDesktopSize | QemuKey) => {
            Verdict::Drop
        }
        // A hover move reveals nothing, and noVNC sends one on every mouse move. A resize fits
        // the screen to the owner's viewer; it types nothing.
        (DisplayMode::Control, Pointer { buttons: 0 } | SetDesktopSize) => Verdict::Pass,
        (DisplayMode::Control, Key | Pointer { .. } | CutText | QemuKey) => Verdict::Input,
    }
}

/// Why a viewer's bytes are refused. The display closes the connection with it.
pub(crate) type Refused = &'static str;

/// Where the viewer is in the protocol. Client bytes arrive in order, so the client's side alone
/// says what comes next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Version,
    Security,
    ClientInit,
    Messages,
}

/// One viewer's bytes so far, cut into whole messages in the order they were sent. It holds at
/// most one unfinished message, which the caps keep under about 1 MiB, besides what a control
/// display's viewer sends while its first input waits (`MAX_HELD_INPUT`).
#[derive(Default)]
pub(crate) struct ClientFramer {
    stage: Stage,
    buffer: BytesMut,
}

impl ClientFramer {
    /// Takes what one WebSocket frame carried.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// How many bytes it holds that no message has taken yet.
    pub(crate) fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// The next whole message and the bytes to send on for it, or `None` until more arrive.
    ///
    /// # Errors
    /// Why the bytes are not RFB as a display takes it. The framer is done with this viewer.
    pub(crate) fn next_message(&mut self) -> Result<Option<(ClientMessage, Bytes)>, Refused> {
        let parsed = match self.stage {
            Stage::Version => version(&self.buffer),
            Stage::Security => security(&self.buffer),
            Stage::ClientInit => whole(&self.buffer, ClientMessage::ClientInit, 1),
            Stage::Messages => message(&self.buffer),
        };
        let (message, length) = match parsed {
            Parse::Incomplete => return Ok(None),
            Parse::Invalid(why) => return Err(why),
            Parse::Done(message, length) => (message, length),
        };
        let mut bytes = self.buffer.split_to(length);
        if message == ClientMessage::ClientInit
            && let Some(shared) = bytes.first_mut()
        {
            *shared = SHARED;
        }
        self.stage = match self.stage {
            Stage::Version => Stage::Security,
            Stage::Security => Stage::ClientInit,
            Stage::ClientInit | Stage::Messages => Stage::Messages,
        };
        Ok(Some((message, bytes.freeze())))
    }
}

fn version(buffer: &[u8]) -> Parse<ClientMessage> {
    match buffer.get(..VERSION.len()) {
        None => Parse::Incomplete,
        Some(version) if version == VERSION => Parse::Done(ClientMessage::Version, VERSION.len()),
        Some(_) => Parse::Invalid("the viewer does not speak RFB 3.8"),
    }
}

fn security(buffer: &[u8]) -> Parse<ClientMessage> {
    match buffer.first() {
        None => Parse::Incomplete,
        Some(&SECURITY_NONE) => Parse::Done(ClientMessage::Security, 1),
        Some(_) => Parse::Invalid("the viewer picked a security type the screen does not offer"),
    }
}

/// One message after the handshake, framed by its type: a fixed length, or a length field that
/// follows the type at a known offset (D12's table).
fn message(buffer: &[u8]) -> Parse<ClientMessage> {
    let Some(&kind) = buffer.first() else {
        return Parse::Incomplete;
    };
    match kind {
        0 => whole(buffer, ClientMessage::SetPixelFormat, 20),
        // Type, padding, a u16 count, then 4 bytes per encoding.
        2 => match u16_at(buffer, 2) {
            None => Parse::Incomplete,
            Some(count) => whole(
                buffer,
                ClientMessage::SetEncodings,
                4 + 4 * usize::from(count),
            ),
        },
        3 => whole(buffer, ClientMessage::UpdateRequest, 10),
        4 => whole(buffer, ClientMessage::Key, 8),
        5 => match buffer.get(1) {
            None => Parse::Incomplete,
            Some(&buttons) => whole(buffer, ClientMessage::Pointer { buttons }, 6),
        },
        6 => cut_text(buffer),
        150 => whole(buffer, ClientMessage::ContinuousUpdates, 10),
        // Type, padding, u32 flags, a u8 length, then the payload.
        248 => match buffer.get(8) {
            None => Parse::Incomplete,
            Some(&length) => whole(buffer, ClientMessage::Fence, 9 + usize::from(length)),
        },
        // Type, padding, u16 width, u16 height, a u8 screen count, padding, 16 bytes per screen.
        251 => match buffer.get(6) {
            None => Parse::Incomplete,
            Some(&screens) if screens > MAX_DESKTOP_SCREENS => {
                Parse::Invalid("a resize that lists more than 16 screens")
            }
            Some(&screens) => whole(
                buffer,
                ClientMessage::SetDesktopSize,
                8 + 16 * usize::from(screens),
            ),
        },
        255 => match buffer.get(1) {
            None => Parse::Incomplete,
            Some(0) => whole(buffer, ClientMessage::QemuKey, 12),
            Some(_) => Parse::Invalid("a QEMU message other than the extended key event"),
        },
        _ => Parse::Invalid("a message type the display does not know"),
    }
}

/// Type, 3 bytes padding, an i32 length, then the body. A negative length is the extended
/// clipboard's: the body is −L bytes and starts with 4 bytes of flags. The cap applies as soon
/// as the length arrives, before any of the body is buffered.
fn cut_text(buffer: &[u8]) -> Parse<ClientMessage> {
    let Some(length) = i32_at(buffer, 4) else {
        return Parse::Incomplete;
    };
    let body = length.unsigned_abs();
    if length < 0 && body < 4 {
        return Parse::Invalid("an extended paste shorter than its flags");
    }
    match usize::try_from(body) {
        Ok(body) if body <= MAX_PASTE => whole(buffer, ClientMessage::CutText, 8 + body),
        _ => Parse::Invalid("a paste over 1 MiB"),
    }
}

/// `message`, once `length` bytes have arrived.
fn whole(buffer: &[u8], message: ClientMessage, length: usize) -> Parse<ClientMessage> {
    if buffer.len() < length {
        Parse::Incomplete
    } else {
        Parse::Done(message, length)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientFramer, ClientMessage, DisplayMode, Refused, Verdict, verdict};

    /// What a viewer sends before its first message: the version, security type 1 and a
    /// ClientInit that asks for exclusive access (0).
    const HANDSHAKE: &[u8] = b"RFB 003.008\n\x01\x00";

    /// The handshake as Xvnc must get it: the ClientInit sent on as 1, shared.
    fn handshake_sent() -> Vec<(ClientMessage, Vec<u8>)> {
        vec![
            (ClientMessage::Version, b"RFB 003.008\n".to_vec()),
            (ClientMessage::Security, vec![1]),
            (ClientMessage::ClientInit, vec![1]),
        ]
    }

    /// One of each message the filter knows, as noVNC and the RFB specification write them.
    fn every_message() -> Vec<(ClientMessage, Vec<u8>)> {
        let mut set_desktop_size = vec![251, 0, 0x01, 0x86, 0x03, 0x4c, 1, 0];
        set_desktop_size.extend_from_slice(&[0, 0, 0, 7, 0, 0, 0, 0, 0x01, 0x86, 0x03, 0x4c]);
        set_desktop_size.extend_from_slice(&[0, 0, 0, 0]);
        let mut extended_paste = vec![6, 0, 0, 0, 0xff, 0xff, 0xff, 0xf8];
        extended_paste.extend_from_slice(&[0x10, 0, 0, 0x01, b'x', b'y', b'z', b'w']);
        vec![
            (
                ClientMessage::SetPixelFormat,
                vec![
                    0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
                ],
            ),
            (
                ClientMessage::SetEncodings,
                vec![
                    2, 0, 0, 3, 0, 0, 0, 7, 0xff, 0xff, 0xfe, 0xcc, 0xff, 0xff, 0xfe, 0xfe,
                ],
            ),
            (
                ClientMessage::UpdateRequest,
                vec![3, 1, 0, 0, 0, 0, 0x06, 0x40, 0x03, 0xe8],
            ),
            (ClientMessage::Key, vec![4, 1, 0, 0, 0, 0, 0, 0x61]),
            (
                ClientMessage::Pointer { buttons: 0 },
                vec![5, 0, 0, 10, 0, 20],
            ),
            (
                ClientMessage::Pointer { buttons: 1 },
                vec![5, 1, 0, 10, 0, 20],
            ),
            (
                ClientMessage::CutText,
                vec![6, 0, 0, 0, 0, 0, 0, 3, b'p', b'w', b'd'],
            ),
            (ClientMessage::CutText, extended_paste),
            (
                ClientMessage::ContinuousUpdates,
                vec![150, 1, 0, 0, 0, 0, 0x06, 0x40, 0x03, 0xe8],
            ),
            (
                ClientMessage::Fence,
                vec![248, 0, 0, 0, 0x80, 0, 0, 0x03, 2, 0xaa, 0xbb],
            ),
            (ClientMessage::SetDesktopSize, set_desktop_size),
            (
                ClientMessage::QemuKey,
                vec![255, 0, 0, 1, 0, 0, 0, 0x61, 0, 0, 0, 0x1e],
            ),
        ]
    }

    /// Everything the framer gives back for `chunks` pushed in turn, or the first refusal.
    fn frame(chunks: &[&[u8]]) -> Result<Vec<(ClientMessage, Vec<u8>)>, Refused> {
        let mut framer = ClientFramer::default();
        let mut framed = Vec::new();
        for chunk in chunks {
            framer.push(chunk);
            while let Some((message, bytes)) = framer.next_message()? {
                framed.push((message, bytes.to_vec()));
            }
        }
        Ok(framed)
    }

    /// The handshake, then `messages`, as Xvnc must receive them.
    fn expected(messages: &[(ClientMessage, Vec<u8>)]) -> Vec<(ClientMessage, Vec<u8>)> {
        let mut all = handshake_sent();
        all.extend_from_slice(messages);
        all
    }

    fn after_handshake(bytes: &[u8]) -> Vec<u8> {
        let mut all = HANDSHAKE.to_vec();
        all.extend_from_slice(bytes);
        all
    }

    #[test]
    fn every_message_split_at_every_byte_frames_whole() {
        for (message, bytes) in every_message() {
            let stream = after_handshake(&bytes);
            for split in 1..stream.len() {
                let (head, tail) = stream.split_at(split);
                assert_eq!(
                    frame(&[head, tail]),
                    Ok(expected(&[(message, bytes.clone())])),
                    "{message:?} split after byte {split}"
                );
            }
            let drip = stream.chunks(1).collect::<Vec<_>>();
            assert_eq!(
                frame(&drip),
                Ok(expected(&[(message, bytes.clone())])),
                "{message:?} one byte at a time"
            );
        }
    }

    #[test]
    fn every_message_merged_in_one_write_frames_in_order() {
        let messages = every_message();
        let stream = after_handshake(
            &messages
                .iter()
                .flat_map(|(_, bytes)| bytes.clone())
                .collect::<Vec<_>>(),
        );
        assert_eq!(frame(&[&stream]), Ok(expected(&messages)));
    }

    #[test]
    fn a_viewer_asking_for_exclusive_access_is_shared_anyway() {
        for asked in [0, 1, 7] {
            let stream = [b"RFB 003.008\n".as_slice(), &[1, asked]].concat();
            let framed = frame(&[&stream]);
            assert_eq!(framed, Ok(handshake_sent()), "ClientInit {asked}");
        }
    }

    #[test]
    fn a_handshake_other_than_rfb_3_8_with_no_security_closes() {
        assert!(frame(&[b"RFB 003.003\n"]).is_err());
        assert!(frame(&[b"GET / HTTP/1.1\r\n"]).is_err());
        assert!(
            frame(&[b"RFB 003.008\n\x02"]).is_err(),
            "VNC authentication"
        );
        assert!(frame(&[b"RFB 003.008\n\x00"]).is_err(), "an invalid type");
        assert_eq!(
            frame(&[b"RFB 003.00"]),
            Ok(Vec::new()),
            "a version still arriving"
        );
    }

    #[test]
    fn a_type_the_display_does_not_know_closes_at_its_first_byte() {
        let known = [0, 2, 3, 4, 5, 6, 150, 248, 251, 255];
        for kind in (0..=u8::MAX).filter(|kind| !known.contains(kind)) {
            assert!(frame(&[&after_handshake(&[kind])]).is_err(), "type {kind}");
        }
    }

    #[test]
    fn a_qemu_message_other_than_the_key_event_closes() {
        assert_eq!(
            frame(&[&after_handshake(&[255])]),
            Ok(handshake_sent()),
            "its subtype has not arrived"
        );
        for subtype in 1..=u8::MAX {
            assert!(
                frame(&[&after_handshake(&[255, subtype])]).is_err(),
                "subtype {subtype}"
            );
        }
    }

    #[test]
    fn a_paste_past_1_mib_closes_as_soon_as_its_length_arrives() {
        const LIMIT: i32 = 1 << 20;
        let length = |length: i32| {
            after_handshake(&[[6, 0, 0, 0].as_slice(), &length.to_be_bytes()].concat())
        };
        assert_eq!(
            frame(&[&length(LIMIT)]),
            Ok(handshake_sent()),
            "a 1 MiB paste waits for its body"
        );
        assert_eq!(frame(&[&length(-LIMIT)]), Ok(handshake_sent()));
        for refused in [LIMIT + 1, -(LIMIT + 1), i32::MAX, i32::MIN] {
            assert!(frame(&[&length(refused)]).is_err(), "length {refused}");
        }
        for short in [-1, -2, -3] {
            assert!(
                frame(&[&length(short)]).is_err(),
                "an extended paste of {short} has no room for its flags"
            );
        }
        let flags_only = [length(-4), vec![0, 0, 0, 1]].concat();
        assert_eq!(
            frame(&[&flags_only]).map(|framed| framed.len()),
            Ok(4),
            "an extended paste of only its flags"
        );
    }

    #[test]
    fn a_resize_listing_more_than_16_screens_closes() {
        let resize = |screens: u8| after_handshake(&[251, 0, 0x01, 0x86, 0x03, 0x4c, screens]);
        assert_eq!(frame(&[&resize(16)]), Ok(handshake_sent()));
        assert!(frame(&[&resize(17)]).is_err());
        assert!(frame(&[&resize(u8::MAX)]).is_err());
    }

    #[test]
    fn a_refusal_comes_after_the_whole_messages_before_it() {
        let mut framer = ClientFramer::default();
        framer.push(&after_handshake(&[3, 0, 0, 0, 0, 0, 0, 1, 0, 1, 9]));
        let mut framed = Vec::new();
        let refused = loop {
            match framer.next_message() {
                Ok(Some((message, _))) => framed.push(message),
                Ok(None) => break None,
                Err(why) => break Some(why),
            }
        };
        assert_eq!(framed.last(), Some(&ClientMessage::UpdateRequest));
        assert!(refused.is_some(), "type 9 after a whole update request");
    }

    #[test]
    fn view_passes_the_screen_and_drops_every_input_paste_and_resize() {
        for (message, _) in every_message() {
            let pass = matches!(
                message,
                ClientMessage::SetPixelFormat
                    | ClientMessage::SetEncodings
                    | ClientMessage::UpdateRequest
                    | ClientMessage::ContinuousUpdates
                    | ClientMessage::Fence
            );
            let expected = if pass { Verdict::Pass } else { Verdict::Drop };
            assert_eq!(verdict(DisplayMode::View, message), expected, "{message:?}");
        }
        for (message, _) in handshake_sent() {
            assert_eq!(verdict(DisplayMode::View, message), Verdict::Pass);
        }
    }

    #[test]
    fn control_passes_everything_and_marks_keys_pastes_and_clicks_as_input() {
        for (message, _) in every_message() {
            let input = matches!(
                message,
                ClientMessage::Key
                    | ClientMessage::QemuKey
                    | ClientMessage::CutText
                    | ClientMessage::Pointer { buttons: 1.. }
            );
            let expected = if input { Verdict::Input } else { Verdict::Pass };
            assert_eq!(
                verdict(DisplayMode::Control, message),
                expected,
                "{message:?}"
            );
        }
        assert_eq!(
            verdict(DisplayMode::Control, ClientMessage::Pointer { buttons: 0 }),
            Verdict::Pass,
            "a hover move is not input"
        );
        assert_eq!(
            verdict(DisplayMode::Control, ClientMessage::SetDesktopSize),
            Verdict::Pass,
            "a resize is not input"
        );
    }
}
