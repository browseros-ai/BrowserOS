//! `Last-Event-ID` semantics, as one pure decision over the ring's bounds. A client resumes
//! only when Neo can show it missed nothing; anything else starts with `stream.reset`, and the
//! client re-reads the state endpoints. Silently skipping events is the outcome this rules out.

/// The ring's extent: the oldest seq it still holds, and the newest it ever held (0 before
/// the first event). An empty ring has `oldest == newest + 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub(crate) oldest: u64,
    pub(crate) newest: u64,
}

/// Where a stream starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resume {
    /// Send every entry after this seq: the client missed nothing.
    After(u64),
    /// Send `stream.reset`, then the entries after `newest`.
    Reset { newest: u64 },
}

/// No `Last-Event-ID` means live events only. An id `<boot>-<seq>` resumes when `boot` is this
/// process's and every entry after `seq` is still in the ring. Another boot's id, one older
/// than the ring, one from the future, or text that is no id at all, resets.
pub(crate) fn resume(last_event_id: Option<&str>, boot: &str, bounds: Bounds) -> Resume {
    let Some(text) = last_event_id else {
        return Resume::After(bounds.newest);
    };
    let seq = text
        .rsplit_once('-')
        .filter(|(id_boot, _)| *id_boot == boot)
        .and_then(|(_, seq)| seq.parse::<u64>().ok())
        .filter(|seq| *seq <= bounds.newest && !missed(*seq, bounds));
    match seq {
        Some(seq) => Resume::After(seq),
        None => Resume::Reset {
            newest: bounds.newest,
        },
    }
}

/// Whether a reader that has read up to `seq` lost entries the ring no longer holds.
pub(crate) fn missed(seq: u64, bounds: Bounds) -> bool {
    seq.saturating_add(1) < bounds.oldest
}

#[cfg(test)]
mod tests {
    use super::{Bounds, Resume, missed, resume};

    const BOOT: &str = "01JBOOT";

    #[test]
    fn last_event_ids_resume_only_when_nothing_was_missed() {
        // The ring holds 78..=1101: 1101 events were published into 1024 slots.
        let ring = Bounds {
            oldest: 78,
            newest: 1101,
        };
        let reset = Resume::Reset { newest: 1101 };
        let cases = [
            (None, Resume::After(1101), "no id: live events only"),
            (Some("01JBOOT-1101"), Resume::After(1101), "up to date"),
            (Some("01JBOOT-500"), Resume::After(500), "inside the ring"),
            (
                Some("01JBOOT-77"),
                Resume::After(77),
                "78 is the first it lacks, and it is kept",
            ),
            (Some("01JBOOT-76"), reset, "77 was evicted"),
            (Some("01JBOOT-1102"), reset, "from the future"),
            (Some("01JOTHER-500"), reset, "another process"),
            (Some("500"), reset, "no boot"),
            (Some("01JBOOT-"), reset, "no seq"),
            (Some("01JBOOT-x"), reset, "not a number"),
            (Some(""), reset, "empty"),
        ];
        for (last, expected, case) in cases {
            assert_eq!(resume(last, BOOT, ring), expected, "{case}");
        }
    }

    #[test]
    fn an_empty_ring_resumes_from_zero() {
        let empty = Bounds {
            oldest: 1,
            newest: 0,
        };
        assert_eq!(resume(Some("01JBOOT-0"), BOOT, empty), Resume::After(0));
        assert_eq!(resume(None, BOOT, empty), Resume::After(0));
        assert!(!missed(0, empty));
    }
}
