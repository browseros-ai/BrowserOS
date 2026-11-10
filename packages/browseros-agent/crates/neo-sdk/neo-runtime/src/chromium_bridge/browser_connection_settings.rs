//! Neo: the settings of the browser connection that a `Runtime`'s runs share. CDP keeps target
//! discovery (`discovery.rs`) and download events (`downloads.rs`) per connection, and one
//! `Runtime` serves every run on its connection, so each such setting is on while any run holds
//! it and off once the last lets go.
//!
//! ```text
//!  Switch::acquire  holders 0 → 1: the setting's command switched on, in the run's scope
//!  Hold::release    holders 1 → 0: switched off, in the scope the first holder switched on
//!  a Hold dropped unreleased lets go on the Runtime's cleanup tracker
//! ```
//!
//! The lock spans each switching call, so a concurrent run's acquire returns only once the
//! setting is on, and a release's "off" can never land after a later acquire's "on".

use std::marker::PhantomData;
use std::sync::Arc;

use browseros_cdp::CdpError;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::browser_representations::downloads::DownloadEvents;
use crate::chromium_bridge::cdp::{self, Command, OnBrowser};
use crate::chromium_bridge::discovery::Discovery;

/// A setting of the connection: the browser-wide command that switches it on (`true`) or off,
/// for the scope it was switched on in (a browser context id, or none).
pub(crate) trait Setting: 'static {
    type Command: Command<Via = OnBrowser>;

    fn command(on: bool, scope: Option<&str>) -> Self::Command;
}

/// The `Runtime`'s switch of one [`Setting`]: how many of its runs hold it. Clones share it.
pub(crate) struct Switch<S> {
    state: Arc<Mutex<State>>,
    /// The `Runtime`'s cleanup tracker, for a hold dropped without [`Hold::release`].
    cleanup: TaskTracker,
    setting: PhantomData<fn() -> S>,
}

impl<S> Clone for Switch<S> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            cleanup: self.cleanup.clone(),
            setting: PhantomData,
        }
    }
}

#[derive(Default)]
struct State {
    holders: usize,
    /// The scope the first holder switched the setting on in, which the last switches off.
    scope: Option<String>,
}

/// One run's hold on a [`Switch`]. [`Hold::release`] lets go; dropping an unreleased one lets go
/// on the `Runtime`'s cleanup tracker.
pub(crate) struct Hold<S: Setting> {
    switch: Switch<S>,
    browser: cdp::Browser,
    released: bool,
}

impl<S: Setting> Switch<S> {
    pub(crate) fn new(cleanup: TaskTracker) -> Self {
        Self {
            state: Arc::default(),
            cleanup,
            setting: PhantomData,
        }
    }

    /// Takes a hold, switching the setting on in `scope` for the first holder.
    ///
    /// # Errors
    ///
    /// The CDP error of the switching call; the hold is not taken.
    pub(crate) async fn acquire(
        &self,
        browser: &cdp::Browser,
        scope: Option<&str>,
    ) -> Result<Hold<S>, CdpError> {
        let mut state = self.state.lock().await;
        if state.holders == 0 {
            browser.send(S::command(true, scope)).await?;
            state.scope = scope.map(str::to_string);
        }
        state.holders += 1;
        Ok(Hold {
            switch: self.clone(),
            browser: browser.clone(),
            released: false,
        })
    }

    /// How many runs hold the setting now.
    #[cfg(test)]
    pub(crate) async fn holders(&self) -> usize {
        self.state.lock().await.holders
    }
}

impl<S: Setting> Hold<S> {
    /// Lets go, switching the setting off when this was the last hold. A failed switch-off is
    /// logged: the next acquire switches it on again either way.
    pub(crate) async fn release(mut self) {
        self.released = true;
        let_go(self.switch.clone(), self.browser.clone()).await;
    }
}

impl<S: Setting> Drop for Hold<S> {
    fn drop(&mut self) {
        if self.released || Handle::try_current().is_err() {
            return;
        }
        self.switch
            .cleanup
            .spawn(let_go(self.switch.clone(), self.browser.clone()));
    }
}

async fn let_go<S: Setting>(switch: Switch<S>, browser: cdp::Browser) {
    let mut state = switch.state.lock().await;
    state.holders = state.holders.saturating_sub(1);
    if state.holders > 0 {
        return;
    }
    let scope = state.scope.take();
    if let Err(error) = browser.send(S::command(false, scope.as_deref())).await {
        let method = S::Command::METHOD;
        debug!(%error, method, "switching a shared setting off failed");
    }
}

/// The `Runtime`'s shared switches, which each run's `Tabs` holds. Clones share them.
#[derive(Clone)]
pub(crate) struct Switches {
    /// Target discovery, on while any run listens for new pages.
    pub(crate) discovery: Switch<Discovery>,
    /// Download events, on while any run listens for downloads.
    pub(crate) downloads: Switch<DownloadEvents>,
}

impl Switches {
    pub(crate) fn new(cleanup: TaskTracker) -> Self {
        Self {
            discovery: Switch::new(cleanup.clone()),
            downloads: Switch::new(cleanup),
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
    use serde_json::{Value, json};

    use super::*;
    use crate::fake_browser::{FakeBrowser, Replies};

    #[tokio::test]
    async fn tests_that_a_setting_is_on_while_any_run_holds_it_and_off_in_its_first_scope() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = cdp::Browser::new(browser.cdp().await);
        let switch = Switch::<DownloadEvents>::new(TaskTracker::new());
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
                json!({ "behavior": "default", "eventsEnabled": true, "browserContextId": "A" }),
                json!({ "behavior": "default", "browserContextId": "A" }),
                json!({ "behavior": "default", "eventsEnabled": true }),
                json!({ "behavior": "default" }),
            ],
            "on and off once per period, in the first holder's scope"
        );
    }
}
