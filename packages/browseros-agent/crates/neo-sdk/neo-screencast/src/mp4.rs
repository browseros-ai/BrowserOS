//! Splits Chromium's fragmented MP4 recording into the pieces a live viewer needs.
//!
//! `Page.startScreenRecording` writes one growing MP4 file to a CDP IO stream: an
//! initialization segment (`ftyp` + `moov`), then a `moof` + `mdat` fragment for every
//! keyframe interval. Reads arrive in arbitrary chunks, so boxes can straddle reads. A viewer
//! joining late needs the initialization segment and then whole fragments; because each
//! fragment starts on a keyframe, any fragment is a valid place to start playback.

use bytes::{Bytes, BytesMut};

/// One piece of the recording, ready to hand to viewers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mp4Part {
    /// Everything before the first fragment: `ftyp`, `moov` and anything between them.
    Init(Bytes),
    /// One `moof` + `mdat` pair, with any boxes the muxer wrote just before it.
    Fragment(Bytes),
}

/// Incremental box splitter. Feed it reads in order; it returns the parts each read completes.
#[derive(Default)]
pub(super) struct Mp4Splitter {
    unread: BytesMut,
    /// Complete boxes waiting for the box that closes their part (`moov` or `mdat`).
    pending: BytesMut,
    init_done: bool,
}

impl Mp4Splitter {
    pub(super) fn push(&mut self, data: &[u8]) -> Result<Vec<Mp4Part>, Mp4Error> {
        self.unread.extend_from_slice(data);
        let mut parts = Vec::new();
        while let Some((kind, len)) = next_box(&self.unread)? {
            let mp4_box = self.unread.split_to(len);
            self.pending.extend_from_slice(&mp4_box);
            // The init segment closes at `moov`, each fragment at its `mdat`. Anything else
            // (`ftyp`, `styp`, `sidx`, `moof`, ...) belongs with the part it precedes.
            if !self.init_done && &kind == b"moov" {
                self.init_done = true;
                parts.push(Mp4Part::Init(self.pending.split().freeze()));
            } else if self.init_done && &kind == b"mdat" {
                parts.push(Mp4Part::Fragment(self.pending.split().freeze()));
            }
        }
        Ok(parts)
    }
}

/// The type and full length of the box at the start of `data`, once all of it has arrived.
fn next_box(data: &[u8]) -> Result<Option<([u8; 4], usize)>, Mp4Error> {
    let Some(header) = data.get(..8) else {
        return Ok(None);
    };
    let kind: [u8; 4] = header[4..8]
        .try_into()
        .map_err(|_| Mp4Error::MalformedBox)?;
    let len = match u32::from_be_bytes(header[..4].try_into().map_err(|_| Mp4Error::MalformedBox)?)
    {
        // A 64-bit size follows the type.
        1 => {
            let Some(large) = data.get(8..16) else {
                return Ok(None);
            };
            let len = u64::from_be_bytes(large.try_into().map_err(|_| Mp4Error::MalformedBox)?);
            if len < 16 {
                return Err(Mp4Error::MalformedBox);
            }
            usize::try_from(len).map_err(|_| Mp4Error::MalformedBox)?
        }
        // "Extends to the end of the file": never written into a live fragmented stream.
        0 => return Err(Mp4Error::MalformedBox),
        len if len < 8 => return Err(Mp4Error::MalformedBox),
        len => usize::try_from(len).map_err(|_| Mp4Error::MalformedBox)?,
    };
    Ok((data.len() >= len).then_some((kind, len)))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum Mp4Error {
    #[error("malformed MP4 box header")]
    MalformedBox,
}

#[cfg(test)]
mod tests {
    use super::{Mp4Part, Mp4Splitter};

    fn mp4_box(kind: &[u8; 4], body_len: usize) -> Vec<u8> {
        let mut out = u32::try_from(8 + body_len)
            .unwrap_or(u32::MAX)
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(kind);
        out.extend(std::iter::repeat_n(kind[0], body_len));
        out
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn splits_the_init_segment_and_each_fragment() -> Result<(), super::Mp4Error> {
        let init = concat(&[mp4_box(b"ftyp", 16), mp4_box(b"moov", 40)]);
        let first = concat(&[mp4_box(b"moof", 24), mp4_box(b"mdat", 100)]);
        let second = concat(&[mp4_box(b"moof", 24), mp4_box(b"mdat", 80)]);
        let mut splitter = Mp4Splitter::default();

        let parts = splitter.push(&concat(&[init.clone(), first.clone(), second.clone()]))?;

        assert_eq!(
            parts,
            vec![
                Mp4Part::Init(init.into()),
                Mp4Part::Fragment(first.into()),
                Mp4Part::Fragment(second.into()),
            ]
        );
        Ok(())
    }

    #[test]
    fn reassembles_boxes_split_across_reads() -> Result<(), super::Mp4Error> {
        let init = concat(&[mp4_box(b"ftyp", 16), mp4_box(b"moov", 40)]);
        let fragment = concat(&[mp4_box(b"moof", 24), mp4_box(b"mdat", 300)]);
        let stream = concat(&[init.clone(), fragment.clone()]);
        let mut splitter = Mp4Splitter::default();
        let mut parts = Vec::new();

        // Seven-byte reads split every header and body.
        for chunk in stream.chunks(7) {
            parts.extend(splitter.push(chunk)?);
        }

        assert_eq!(
            parts,
            vec![
                Mp4Part::Init(init.into()),
                Mp4Part::Fragment(fragment.into())
            ]
        );
        Ok(())
    }

    #[test]
    fn keeps_boxes_written_between_fragments_with_the_next_fragment() -> Result<(), super::Mp4Error>
    {
        let init = concat(&[mp4_box(b"ftyp", 8), mp4_box(b"moov", 8)]);
        let fragment = concat(&[
            mp4_box(b"styp", 4),
            mp4_box(b"moof", 8),
            mp4_box(b"mdat", 8),
        ]);
        let mut splitter = Mp4Splitter::default();

        let parts = splitter.push(&concat(&[init, fragment.clone()]))?;

        assert_eq!(parts.last(), Some(&Mp4Part::Fragment(fragment.into())));
        Ok(())
    }

    #[test]
    fn reads_64_bit_box_sizes() -> Result<(), super::Mp4Error> {
        let init = concat(&[mp4_box(b"ftyp", 8), mp4_box(b"moov", 8)]);
        // size == 1 means the real size follows the type as a u64.
        let mut large_mdat = 1u32.to_be_bytes().to_vec();
        large_mdat.extend_from_slice(b"mdat");
        large_mdat.extend_from_slice(&(16u64 + 10).to_be_bytes());
        large_mdat.extend(std::iter::repeat_n(0u8, 10));
        let fragment = concat(&[mp4_box(b"moof", 8), large_mdat]);
        let mut splitter = Mp4Splitter::default();

        let parts = splitter.push(&concat(&[init, fragment.clone()]))?;

        assert_eq!(parts.last(), Some(&Mp4Part::Fragment(fragment.into())));
        Ok(())
    }

    #[test]
    fn rejects_a_box_smaller_than_its_header() {
        let mut splitter = Mp4Splitter::default();
        let mut bogus = 4u32.to_be_bytes().to_vec();
        bogus.extend_from_slice(b"ftyp");

        assert_eq!(splitter.push(&bogus), Err(super::Mp4Error::MalformedBox));
    }
}
