//! Which tabs the camera must not picture: those that took a host's private typing (claw-sdk's
//! `POST /v1/tabs/{tabId}/fill`, protected-fill design section 10.3, rulings R15 and R28).
//!
//! ```text
//!  the app's entry ─ marks a tab before its first private character, for the tab's life
//!        └─ Service::arm_private_tabs(took) ─▶ PrivateTabs
//!  camera: a job ─ PrivateTabs::withholds(tab)? ─ yes ─▶ Shot::Withheld, no capture
//!                └ no ─ capture ─ withholds(tab)? ─ yes ─▶ Shot::Withheld, pixels dropped
//!                                               └ no ─▶ the JPEG written
//! ```
//!
//! The state belongs to the app, which typed the values; this crate only asks. It is asked twice,
//! because a capture already running when a fill starts would otherwise store the page with the
//! value painted in. It is never cleared while the tab lives: going back in history restores a
//! text box's value into a new document. Unarmed, it withholds nothing, which is exact: only the
//! app's fill marks a tab, and the app arms this before its routes serve.

use browseros_core::TabId;
use std::sync::{Arc, OnceLock};

/// Whether `tab` took a private fill, as the app answers it. Called on the camera's task for
/// each job, so it must be cheap and must not block.
pub type TookPrivateText = Arc<dyn Fn(TabId) -> bool + Send + Sync>;

/// The armed question, shared by the service and its camera.
#[derive(Default)]
pub(crate) struct PrivateTabs {
    took: OnceLock<TookPrivateText>,
}

impl PrivateTabs {
    /// Arms the question. The first arm stays: Neo has one app.
    pub(crate) fn arm(&self, took: TookPrivateText) {
        if self.took.set(took).is_err() {
            tracing::warn!("the private tabs were armed twice; the first answer stays");
        }
    }

    /// Whether the camera must not picture `tab`.
    pub(crate) fn withholds(&self, tab: TabId) -> bool {
        self.took.get().is_some_and(|took| took(tab))
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
