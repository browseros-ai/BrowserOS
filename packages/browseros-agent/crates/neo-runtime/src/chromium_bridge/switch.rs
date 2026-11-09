//! Neo: the settings of the browser connection that a `Runtime`'s runs share. CDP keeps target
//! discovery (`discovery.rs`) and download events (`downloads.rs`) per connection, and one
//! `Runtime` serves every run on its connection, so each such setting is on while any run holds
//! it and off once the last lets go.
//!
//! ```text
//!  Switch::acquire  holders 0 → 1: the setting's method with its "on" params, in the run's scope
//!  Hold::release    holders 1 → 0: its "off" params, in the scope the first holder switched on
//!  a Hold dropped unreleased lets go on the Runtime's cleanup tracker
//! ```
//!
//! The lock spans each switching call, so a concurrent run's acquire returns only once the
//! setting is on, and a release's "off" can never land after a later acquire's "on".

use std::sync::Arc;

use browseros_cdp::{CdpClient, CdpError};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::discovery;
use crate::downloads;

/// A setting of the connection: the CDP method that switches it, and its params on (`true`) or
/// off, for the scope it was switched on in (a browser context id, or none).
pub(crate) struct Setting {
    pub(crate) method: &'static str,
    pub(crate) params: fn(bool, Option<&str>) -> Value,
}

/// The `Runtime`'s switch of one [`Setting`]: how many of its runs hold it. Clones share it.
#[derive(Clone)]
pub(crate) struct Switch {
    state: Arc<Mutex<State>>,
    setting: &'static Setting,
    /// The `Runtime`'s cleanup tracker, for a hold dropped without [`Hold::release`].
    cleanup: TaskTracker,
}

#[derive(Default)]
struct State {
    holders: usize,
    /// The scope the first holder switched the setting on in, which the last switches off.
    scope: Option<String>,
}

/// One run's hold on a [`Switch`]. [`Hold::release`] lets go; dropping an unreleased one lets go
/// on the `Runtime`'s cleanup tracker.
pub(crate) struct Hold {
    switch: Switch,
    cdp: CdpClient,
    released: bool,
}

impl Switch {
    pub(crate) fn new(setting: &'static Setting, cleanup: TaskTracker) -> Self {
        Self {
            state: Arc::default(),
            setting,
            cleanup,
        }
    }

    /// Takes a hold, switching the setting on in `scope` for the first holder.
    ///
    /// # Errors
    ///
    /// The CDP error of the switching call; the hold is not taken.
    pub(crate) async fn acquire(
        &self,
        cdp: &CdpClient,
        scope: Option<&str>,
    ) -> Result<Hold, CdpError> {
        let mut state = self.state.lock().await;
        if state.holders == 0 {
            cdp.send(
                self.setting.method,
                (self.setting.params)(true, scope),
                None,
            )
            .await?;
            state.scope = scope.map(str::to_string);
        }
        state.holders += 1;
        Ok(Hold {
            switch: self.clone(),
            cdp: cdp.clone(),
            released: false,
        })
    }

    /// How many runs hold the setting now.
    #[cfg(test)]
    pub(crate) async fn holders(&self) -> usize {
        self.state.lock().await.holders
    }
}

impl Hold {
    /// Lets go, switching the setting off when this was the last hold. A failed switch-off is
    /// logged: the next acquire switches it on again either way.
    pub(crate) async fn release(mut self) {
        self.released = true;
        let_go(self.switch.clone(), self.cdp.clone()).await;
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        if self.released || Handle::try_current().is_err() {
            return;
        }
        self.switch
            .cleanup
            .spawn(let_go(self.switch.clone(), self.cdp.clone()));
    }
}

async fn let_go(switch: Switch, cdp: CdpClient) {
    let mut state = switch.state.lock().await;
    state.holders = state.holders.saturating_sub(1);
    if state.holders > 0 {
        return;
    }
    let scope = state.scope.take();
    let off = (switch.setting.params)(false, scope.as_deref());
    if let Err(error) = cdp.send(switch.setting.method, off, None).await {
        debug!(%error, method = switch.setting.method, "switching a shared setting off failed");
    }
}

/// The `Runtime`'s shared switches, which each run's `Tabs` holds. Clones share them.
#[derive(Clone)]
pub(crate) struct Switches {
    /// Target discovery, on while any run listens for new pages.
    pub(crate) discovery: Switch,
    /// Download events, on while any run listens for downloads.
    pub(crate) downloads: Switch,
}

impl Switches {
    pub(crate) fn new(cleanup: TaskTracker) -> Self {
        Self {
            discovery: Switch::new(&discovery::DISCOVERY, cleanup.clone()),
            downloads: Switch::new(&downloads::DOWNLOAD_EVENTS, cleanup),
        }
    }
}

impl Default for Switches {
    fn default() -> Self {
        Self::new(TaskTracker::new())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fake_browser::{FakeBrowser, Replies};

    fn scoped(on: bool, scope: Option<&str>) -> Value {
        json!({ "on": on, "scope": scope })
    }

    static SCOPED: Setting = Setting {
        method: "Browser.setDownloadBehavior",
        params: scoped,
    };

    #[tokio::test]
    async fn tests_that_a_setting_is_on_while_any_run_holds_it_and_off_in_its_first_scope() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let switch = Switch::new(&SCOPED, TaskTracker::new());
        let first = switch.acquire(&cdp, Some("A")).await.unwrap();
        let second = switch.acquire(&cdp, Some("B")).await.unwrap();
        assert_eq!(switch.holders().await, 2);
        first.release().await;
        assert_eq!(switch.holders().await, 1);
        drop(second);
        let cleanup = switch.cleanup.clone();
        cleanup.close();
        cleanup.wait().await;
        assert_eq!(switch.holders().await, 0, "a dropped hold lets go too");
        let third = switch.acquire(&cdp, None).await.unwrap();
        third.release().await;
        let switches: Vec<Value> = browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method() == "Browser.setDownloadBehavior")
            .map(|exchange| exchange.command["params"].clone())
            .collect();
        assert_eq!(
            switches,
            [
                json!({ "on": true, "scope": "A" }),
                json!({ "on": false, "scope": "A" }),
                json!({ "on": true, "scope": null }),
                json!({ "on": false, "scope": null }),
            ],
            "on and off once per period, in the first holder's scope"
        );
    }
}
