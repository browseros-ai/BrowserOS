//! What Neo does when its trace store had to be replaced: it treats every open tab as private.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use browseros_core::TabId;
use browseros_trace::{EpochMillis, Privacy, PrivateReason};
use tokio::fs;

/// What the marker says to an operator who finds it beside the store.
const MARKER_TEXT: &str = "Neo replaced trace.db, and the private flags it held were lost. \
    Until every tab open at the browser's first connect is private again, Neo captures no tab. \
    Neo deletes this file once those flags are on disk.\n";

/// Whether Neo is still making up for private flags it lost.
///
/// A restart keeps every flag (sdk-traces D6), because `Privacy` starts from the ones the trace
/// store holds. When the store does not open and a fresh one replaces it (`app::open_trace`),
/// those flags are gone, and with them every record of which tabs were private. browseros-trace
/// cannot read a file that `TraceStore::open` refused, so Neo fails closed:
///
/// ```text
///  start ─ trace.db does not open ─ the marker written beside it ─ a fresh store replaces it
///  every start that finds the marker ─ owed: PrivacyHooks refuses every capture
///  the browser link's first connect ─ HostApi::serve ─ recover(every tab open now): each one
///        private, durably ─ the marker deleted ─ captures allowed again ─ Service::start
/// ```
///
/// The marker carries the debt across a stop. The fresh store opens at the next start like any
/// other, so without it a Neo stopped before it recovered would leave the next one owing
/// nothing. It is deleted only once every recovered flag is on disk, so a flag whose commit
/// failed is owed again at the next start.
///
/// A tab that opens later never had a flag to lose. Each tab open at the first connect stays
/// private until it closes, which over-protects the ones that were not private before.
#[derive(Debug)]
pub(crate) struct FlagRecovery {
    /// The marker beside the store while flags are owed; `None` when nothing is.
    marker: Option<PathBuf>,
    pending: AtomicBool,
}

impl FlagRecovery {
    /// The store opened with its flags, and no earlier start left any owed.
    #[must_use]
    pub(crate) fn none() -> Arc<Self> {
        Arc::new(Self {
            marker: None,
            pending: AtomicBool::new(false),
        })
    }

    /// After `store` opened: owed when an earlier start replaced it and stopped before every
    /// open tab was private again. A marker Neo cannot look for counts as found, because failing
    /// closed costs captures only until the browser link's first connect.
    pub(crate) async fn after_opening(store: &Path) -> Arc<Self> {
        let marker = marker_of(store);
        match fs::try_exists(&marker).await {
            Ok(false) => Self::none(),
            Ok(true) => {
                tracing::warn!(marker = %marker.display(), "an earlier start replaced the trace store and stopped before every open tab was private again; every tab open now is made private");
                Self::owed(marker)
            }
            Err(error) => {
                tracing::warn!(%error, marker = %marker.display(), "cannot tell whether an earlier start owes private flags; every tab open now is made private");
                Self::owed(marker)
            }
        }
    }

    /// Before `store` is replaced: writes the marker, so a stop at any point from here on leaves
    /// the next start owing the flags too. A process crash keeps the marker ahead of the move,
    /// which is the failure the store itself is tuned for (`synchronous = NORMAL`).
    ///
    /// # Errors
    /// When the marker cannot be written. The store must not be replaced then: a fresh store
    /// with no marker would let the next start capture tabs that were private.
    pub(crate) async fn before_replacing(store: &Path) -> io::Result<Arc<Self>> {
        let marker = marker_of(store);
        fs::write(&marker, MARKER_TEXT).await?;
        Ok(Self::owed(marker))
    }

    fn owed(marker: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            marker: Some(marker),
            pending: AtomicBool::new(true),
        })
    }

    /// Whether no tab may be captured yet: the flags were lost, and the open tabs are not
    /// private yet.
    #[must_use]
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Makes every tab in `open` private, then lets captures through, and deletes the marker
    /// once every flag is on disk. The reason on record is `host`, the trace's nearest: Neo
    /// decided it for the host. A flag that cannot be committed still holds in memory, so its
    /// tab stays private in this process, and the kept marker makes the next start recover
    /// again.
    pub(crate) async fn recover(&self, privacy: &Privacy, open: &[TabId]) {
        let at = EpochMillis::now();
        let mut on_disk = true;
        for &tab in open {
            if let Err(error) = privacy.make_private(tab, PrivateReason::Host, at).await {
                on_disk = false;
                tracing::warn!(%error, tab = tab.0, "a tab made private after the trace store was replaced is private, but not on disk");
            }
        }
        self.pending.store(false, Ordering::Release);
        tracing::warn!(
            tabs = open.len(),
            "every tab open since the trace store was replaced is private until it closes"
        );
        let Some(marker) = &self.marker else {
            return;
        };
        if !on_disk {
            tracing::warn!(marker = %marker.display(), "a recovered flag is not on disk, so the next start makes every open tab private again");
            return;
        }
        if let Err(error) = fs::remove_file(marker).await {
            tracing::warn!(%error, marker = %marker.display(), "the recovered flags are on disk, but the marker stays, so the next start makes every open tab private again");
        }
    }
}

/// `trace.db.flags-lost`, beside the store it is about.
fn marker_of(store: &Path) -> PathBuf {
    let mut marker = store.as_os_str().to_owned();
    marker.push(".flags-lost");
    PathBuf::from(marker)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::TabId;
    use browseros_trace::{
        Privacy, PrivacyRecord, PrivacyStore, SecretSet, SecretSetConfig, TraceError,
    };
    use futures_util::future::{self, BoxFuture};

    use super::FlagRecovery;

    /// Where private flags go: a disk that commits each one, or one that fails every commit.
    struct Disk {
        fails: bool,
    }

    impl PrivacyStore for Disk {
        fn persist_private(
            &self,
            _tab: TabId,
            _record: PrivacyRecord,
        ) -> BoxFuture<'_, Result<(), TraceError>> {
            let committed = if self.fails {
                Err(TraceError::Closed)
            } else {
                Ok(())
            };
            Box::pin(future::ready(committed))
        }

        fn forget_private(&self, _tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
            Box::pin(future::ready(Ok(())))
        }
    }

    fn privacy_on(disk: Disk) -> Arc<Privacy> {
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        Privacy::new(secrets, Arc::new(disk), HashMap::new())
    }

    #[tokio::test]
    async fn lost_flags_are_owed_at_every_start_until_each_one_is_on_disk()
    -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = dir.path().join("trace.db");
        assert!(
            !FlagRecovery::after_opening(&store).await.is_pending(),
            "a store that opened owes nothing"
        );

        let replaced = FlagRecovery::before_replacing(&store).await?;
        assert!(
            replaced.is_pending(),
            "no capture until the tabs are private"
        );
        assert!(
            FlagRecovery::after_opening(&store).await.is_pending(),
            "a restart before the recovery still owes the flags"
        );

        replaced
            .recover(&privacy_on(Disk { fails: true }), &[TabId(1)])
            .await;
        assert!(
            !replaced.is_pending(),
            "the flag holds in memory, so this process may capture other tabs"
        );
        let next = FlagRecovery::after_opening(&store).await;
        assert!(
            next.is_pending(),
            "a flag that never reached the disk is owed at the next start"
        );

        next.recover(&privacy_on(Disk { fails: false }), &[TabId(1)])
            .await;
        assert!(
            !FlagRecovery::after_opening(&store).await.is_pending(),
            "once every flag is on disk, no later start owes any"
        );
        Ok(())
    }
}
