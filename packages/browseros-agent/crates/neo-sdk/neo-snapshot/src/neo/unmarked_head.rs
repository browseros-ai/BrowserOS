use crate::snapshot::{Snapshot, Unmarked};

/// The words that open the line, which [`is_unmarked_line`] recognises it by.
const PREFIX: &str = "# refs missing: ";

/// `unmarkedHead`: the line a text opens with, after the on-top lines and outside the cap, when
/// the root frame's check for clickable elements did not finish. A cut must not hide why refs
/// may be missing.
pub(crate) fn unmarked_head(snapshot: &Snapshot) -> Option<String> {
    snapshot.unmarked.as_ref().map(|Unmarked { reason }| {
        format!(
            "{PREFIX}the check for clickable elements did not finish on this page ({reason}). \
             Some clickable elements may have no ref here, and content inside closed shadow \
             roots may be missing. Take the snapshot again; if this line is still here, use a \
             locator."
        )
    })
}

/// `isUnmarkedLine`: the `# refs missing:` line. It explains this read, not the page, so a delta
/// leaves it out of both texts it compares and prints the current read's own once, after its
/// first line.
pub(crate) fn is_unmarked_line(line: &str) -> bool {
    line.starts_with(PREFIX)
}
