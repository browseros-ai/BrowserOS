//! One viewer's live view of several sessions at once, for a client that shows many of them
//! over one connection (the cockpit's `/api/v1/screencasts` socket).

use super::{CaptureHub, Frame, Quality};
use futures_util::StreamExt;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, task::JoinHandle};

/// How long a watch that has nothing to show waits before trying again.
const WATCH_RETRY: Duration = Duration::from_secs(1);
/// Watches one viewer may hold. The set comes from the client, and each watch is a task that
/// re-reads its session every second and may start a capture in the browser.
const MAX_WATCHES: usize = 32;

/// What a live view shows: a session's current tab, or one tab of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Watch {
    pub session_id: String,
    pub tab: Option<i64>,
}

/// One viewer's live view of a set of [`Watch`]es, all at one quality; see
/// [`CaptureHub::watcher`]. Each watch runs as its own [`CaptureHub::screencast`] stream, so
/// following the current tab, pacing, sharing a capture with other viewers and ending with the
/// session all work as they do for a single stream. Dropping the watcher releases them all.
pub struct Watcher {
    hub: Arc<CaptureHub>,
    quality: Quality,
    tasks: HashMap<Watch, JoinHandle<()>>,
    pending: Arc<Pending>,
}

impl Watcher {
    pub(super) fn new(hub: Arc<CaptureHub>, quality: Quality) -> Self {
        Self {
            hub,
            quality,
            tasks: HashMap::new(),
            pending: Arc::new(Pending::default()),
        }
    }

    /// Replaces what this viewer watches with the first [`MAX_WATCHES`] of `watches`. A watch
    /// left out releases its capture; a new one starts. A watch with nothing to show yet (an
    /// unknown session, no open tab, a closed pinned tab, a browser error) or whose stream ended
    /// is tried again every [`WATCH_RETRY`] for as long as it stays in the set, so errors never
    /// surface: such a watch simply has no frames.
    pub fn set(&mut self, watches: impl IntoIterator<Item = Watch>) {
        let wanted = watches
            .into_iter()
            .take(MAX_WATCHES)
            .collect::<HashSet<_>>();
        // Aborting a task drops its stream, which is how its capture learns this viewer left.
        self.tasks.retain(|watch, task| {
            let keep = wanted.contains(watch);
            if !keep {
                task.abort();
            }
            keep
        });
        for watch in wanted {
            if !self.tasks.contains_key(&watch) {
                let task = tokio::spawn(forward(
                    self.hub.clone(),
                    watch.clone(),
                    self.quality,
                    self.pending.clone(),
                ));
                self.tasks.insert(watch, task);
            }
        }
    }

    /// The next frame of any watch, tagged with the watch as given to [`Watcher::set`]. Only
    /// the newest frame of each watch waits to be read, so a slow reader skips the ones it
    /// missed instead of holding them. `None` once the server shuts down.
    pub async fn next(&mut self) -> Option<(Watch, Frame)> {
        loop {
            let changed = self.pending.changed.notified();
            match self.pending.take() {
                // A task aborted by `set` may have put one last frame on its way out.
                Some((watch, _)) if !self.tasks.contains_key(&watch) => {}
                Some(item) => return Some(item),
                None => tokio::select! {
                    () = self.hub.shutdown.cancelled() => return None,
                    () = changed => {}
                },
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.abort();
        }
    }
}

/// Shows `watch` for as long as the task lives: a stream that can't open yet, or that ended
/// (the session ended, the pinned tab closed), is opened again after [`WATCH_RETRY`].
async fn forward(hub: Arc<CaptureHub>, watch: Watch, quality: Quality, pending: Arc<Pending>) {
    loop {
        match hub.screencast(&watch.session_id, quality, watch.tab).await {
            Ok(mut frames) => {
                while let Some(frame) = frames.next().await {
                    pending.put(watch.clone(), frame);
                }
            }
            Err(error) => {
                tracing::trace!(session_id = %watch.session_id, %error, "watch not showable yet")
            }
        }
        tokio::select! {
            () = hub.shutdown.cancelled() => return,
            () = tokio::time::sleep(WATCH_RETRY) => {}
        }
    }
}

/// The newest undelivered frame of each watch, in the order the watches first had one, so a
/// watch that paints often cannot keep a slower one waiting.
#[derive(Default)]
struct Pending {
    queue: Mutex<PendingQueue>,
    changed: Notify,
}

#[derive(Default)]
struct PendingQueue {
    order: VecDeque<Watch>,
    frames: HashMap<Watch, Frame>,
}

impl Pending {
    fn put(&self, watch: Watch, frame: Frame) {
        let mut queue = lock(&self.queue);
        if queue.frames.insert(watch.clone(), frame).is_none() {
            queue.order.push_back(watch);
        }
        drop(queue);
        self.changed.notify_one();
    }

    fn take(&self) -> Option<(Watch, Frame)> {
        let mut queue = lock(&self.queue);
        let watch = queue.order.pop_front()?;
        let frame = queue.frames.remove(&watch)?;
        Some((watch, frame))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
