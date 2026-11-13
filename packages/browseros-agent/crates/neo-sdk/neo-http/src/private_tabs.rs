//! Which tabs the camera must not picture: those that took a host's private typing (claw-sdk's
//! `POST /v1/tabs/{tabId}/fill`, protected-fill design section 10.3, rulings R15 and R28), while
//! the host's `fill.pictures` is `never` (ruling R58). Under the default, `allowed`, the app's
//! answer is `false` for every tab, and only the second question below withholds a picture.
//!
//! ```text
//!  the app's entry ─ marks a tab before its first private character, for the tab's life;
//!                    the app answers it withheld only under `fill.pictures: never`
//!        └─ Service::arm_private_tabs(took) ─▶ PrivateTabs
//!  camera: a job ─ PrivateTabs::withholds(tab)? ─ yes ─▶ Shot::Withheld, no capture
//!                └ no ─ capture ─ withholds(tab)? ─ yes ─▶ Shot::Withheld, pixels dropped
//!                                               └ no ─▶ the JPEG written
//! ```
//!
//! A second question covers what the host did not type (ruling R45): a box the page declares
//! private (a card number, a code), typed by a script or the owner, is painted in clear. The app
//! reads every frame of the tab and answers [`PictureVerdict`]; the camera asks it before and
//! after a capture, keeps a picture only when both answers are clear and read the same (every
//! frame's document and its private boxes' input count), and treats a verdict that does not come
//! in time as private.
//!
//! The state belongs to the app, which typed the values; this crate only asks. It is asked twice,
//! because a capture already running when a fill starts would otherwise store the page with the
//! value painted in. It is never cleared while the tab lives: going back in history restores a
//! text box's value into a new document. Unarmed, it withholds nothing, which is exact: only the
//! app's fill marks a tab, and the app arms this before its routes serve.

use browseros_core::TabId;
use futures_util::future::BoxFuture;
use std::sync::{Arc, OnceLock};

/// Whether `tab` took a private fill, as the app answers it. Called on the camera's task for
/// each job, so it must be cheap and must not block.
pub type TookPrivateText = Arc<dyn Fn(TabId) -> bool + Send + Sync>;

/// What a tab shows now, as the app reads it in every frame (ruling R45).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureVerdict {
    /// No frame shows a private box's value. `reading` names what the app read (each document
    /// that holds a private box and how many inputs its private boxes took), so a capture taken
    /// between two clear verdicts is kept only when both readings are the same.
    Clear {
        /// The app's reading, compared as it is.
        reading: String,
    },
    /// A frame shows a private box's value, or the tab could not be read.
    Private,
}

/// The app's reading of a tab, asked before and after each capture. It reads every frame, so
/// it may take a moment; the camera bounds it.
pub type ChecksPicture = Arc<dyn Fn(TabId) -> BoxFuture<'static, PictureVerdict> + Send + Sync>;

/// The armed questions, shared by the service and its camera.
#[derive(Default)]
pub(crate) struct PrivateTabs {
    took: OnceLock<TookPrivateText>,
    check: OnceLock<ChecksPicture>,
}

impl PrivateTabs {
    /// Arms the question. The first arm stays: Neo has one app.
    pub(crate) fn arm(&self, took: TookPrivateText) {
        if self.took.set(took).is_err() {
            tracing::warn!("the private tabs were armed twice; the first answer stays");
        }
    }

    /// Arms the reading of what a tab shows. The first arm stays.
    pub(crate) fn arm_check(&self, check: ChecksPicture) {
        if self.check.set(check).is_err() {
            tracing::warn!("the picture check was armed twice; the first stays");
        }
    }

    /// Whether the camera must not picture `tab`.
    pub(crate) fn withholds(&self, tab: TabId) -> bool {
        self.took.get().is_some_and(|took| took(tab))
    }

    /// What `tab` shows now, or `None` when no app armed the reading: nothing to withhold then,
    /// as no app serves the fill or masks a page.
    pub(crate) async fn verdict(&self, tab: TabId) -> Option<PictureVerdict> {
        let check = Arc::clone(self.check.get()?);
        Some(check(tab).await)
    }
}

#[cfg(test)]
mod tests {
    use super::PrivateTabs;
    use browseros_core::TabId;
    use std::sync::Arc;

    #[test]
    fn unarmed_it_withholds_nothing_and_armed_it_asks_the_app() {
        let tabs = PrivateTabs::default();
        assert!(!tabs.withholds(TabId(3)));
        tabs.arm(Arc::new(|tab| tab == TabId(3)));
        tabs.arm(Arc::new(|_| false));
        assert!(tabs.withholds(TabId(3)), "the first arm stays");
        assert!(!tabs.withholds(TabId(4)));
    }
}
