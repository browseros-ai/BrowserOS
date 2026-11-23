use crate::snapshot::{Snapshot, Unmarked};

/// `unmarkedHead`: the line a text opens with, after the on-top lines and outside the cap, when
/// the root frame's check for clickable elements did not finish. A cut must not hide why refs
/// may be missing.
pub(super) fn unmarked_head(snapshot: &Snapshot) -> Option<String> {
    snapshot.unmarked.as_ref().map(|Unmarked { reason }| {
        format!(
            "# refs missing: the check for clickable elements did not finish on this page \
             ({reason}). Some clickable elements may have no ref here, and content inside closed \
             shadow roots may be missing. Take the snapshot again; if this line is still here, \
             use a locator."
        )
    })
}
