//! Playwright: `server/frames.ts` `Frame._retryWithProgressIfNotConnected`
//! Playwright: `server/dom.ts` `ElementHandle._retryAction`
//! Playwright: `server/page.ts` `Page.performActionPreChecks`
//! Neo: shared retry bookkeeping over the tab seam. Loop A owns locator resolution;
//! action families retain their own attempts and result handling inside loop B.
//! Pre-checks and backoff run before an action takes the tab's input turn.
//!
//! ```text
//!  retry_with_progress_if_not_connected
//!   loop A: backoff -> prepare -> pre-checks -> describe once -> resolve
//!     ElementAction::act on this resolved element
//!       loop B: RetryAction::before -> the family's attempt and result handling
//!         detached -> loop A resolves again
//!         done or terminal error -> return to the caller
//! ```

use std::future::Future;
use std::time::Duration;

use neo_playwright_protocol::injected::ElementState;

use crate::playwright_methods::tab::{NotReady, Stop, Tab};
use crate::progress::{CallError, PlaywrightError, Progress};

/// Loop A's backoff after its immediate first try.
/// Playwright: `server/frames.ts` `Frame.retryWithProgressAndBackoff`: `const backoffScale = [20, 50, 100, 100, 500];`
const LOOP_A_BACKOFF_MS: [u32; 5] = [20, 50, 100, 100, 500];
/// Loop B's waits by retry index, the last repeating, never capped.
/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `const waitTime = [0, 20, 100, 100, 500];`
const LOOP_B_WAITS_MS: [u64; 5] = [0, 20, 100, 100, 500];
/// The longest URL the navigation pre-check prints.
/// Playwright: `server/page.ts` `Page._performWaitForNavigationCheck`: `trimStringWithEllipsis(url, 200)`
const LOGGED_URL_CHARS: usize = 200;
/// Neo: whether this attempt can run navigation and locator-handler pre-checks.
/// Callers skip them for forced input or while a drag already holds its source button.
/// The choice stays outside the retry owner, which never imports pointer policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreChecks {
    Run,
    Skip,
}

/// Neo: converts an action's existing force option into its pre-check policy.
impl PreChecks {
    pub(crate) fn unless_forced(force: bool) -> Self {
        if force { Self::Skip } else { Self::Run }
    }
}

/// Neo: the common loop-B prologue; each action still owns its attempt and result handling.
/// No input turn is held during its waits or navigation and locator-handler checks.
/// A newly resolved element gets a fresh instance, so a detach restarts its attempt sequence.
pub(crate) struct RetryAction {
    name: &'static str,
    trial: bool,
    attempts: usize,
}

/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `const waitTime = [0, 20, 100, 100, 500];`
impl RetryAction {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            trial: false,
            attempts: 0,
        }
    }

    pub(crate) fn with_trial(mut self, trial: bool) -> Self {
        self.trial = trial;
        self
    }

    /// Returns this attempt's zero-based index after its log, backoff and pre-checks.
    /// An action uses the index for its own retry-dependent preparation, such as scrolling.
    pub(crate) async fn before<T: Tab>(
        &mut self,
        tab: &T,
        pre_checks: PreChecks,
        progress: &Progress,
    ) -> Result<usize, Stop> {
        let attempt = self.attempts;
        let name = self.name;
        let trial = if self.trial { " (trial run)" } else { "" };
        if attempt == 0 {
            progress.log(format!("attempting {name} action{trial}"));
        } else {
            progress.log(format!("retrying {name} action{trial}"));
            let wait = loop_b_wait(attempt);
            if !wait.is_zero() {
                progress.log(format!("  waiting {}ms", wait.as_millis()));
                progress.wait(wait).await?;
            }
        }
        self.attempts = self.attempts.saturating_add(1);
        if pre_checks == PreChecks::Run {
            wait_out_pending_navigation(tab, progress).await?;
        }
        Ok(attempt)
    }
}

/// A per-element action inside Playwright's loop A. It returns `NotConnected` only when the
/// page proved the element detached; transient page errors use `Stop::Retry` without that log.
/// The resolved handle lives until `act` returns, including across its loop-B retries.
/// Neo: each action owns its result while loop A owns the resolved element lifetime.
pub(crate) trait ElementAction<T: Tab>: Sync {
    type Output;

    fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, Stop>> + Send;
}

/// How one per-element action ended for loop A.
/// Neo: a typed value or proof that the next iteration must resolve another element.
pub(crate) enum ActionResult<R> {
    Done(R),
    NotConnected,
}

/// Playwright's `_retryWithProgressIfNotConnected`, shared by pointer and form actions. Naming,
/// backoff, preparation, pre-checks, description and resolution stay in exactly that order.
/// The element and its object group drop after each iteration, as Playwright disposes its handle.
/// The caller chooses pre-checks before entering the loop; the retry owner has no knowledge
/// of pointer options or the action family.
/// Playwright: `server/frames.ts` `Frame._retryWithProgressIfNotConnected`: `const performActionPreChecks = !options.skipActionPreChecks && !options.force;`
pub(crate) async fn retry_with_progress_if_not_connected<T: Tab, A: ElementAction<T>>(
    tab: &T,
    selector: &str,
    strict: bool,
    pre_checks: PreChecks,
    progress: &Progress,
    action: &A,
) -> Result<A::Output, CallError> {
    // Playwright names the locator before any page call. Neo learns its text from the page, so
    // the line reads the raw selector until `describe` answers (Playwright's own fallback).
    progress.name_locator(selector);
    let mut named = false;
    let mut waits = LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let iteration = async {
            progress.race(tab.prepare()).await??;
            if pre_checks == PreChecks::Run {
                wait_out_pending_navigation(tab, progress).await?;
            }
            if !named {
                let facts = progress.race(tab.describe(selector)).await??;
                progress.name_locator(&facts.locator);
                named = true;
                // Playwright parses the selector before its first page call and fails at once.
                if let Some(message) = facts.invalid {
                    return Err(PlaywrightError::error(message).into());
                }
            }
            let resolved = progress.race(tab.resolve(selector, strict)).await??;
            let Some(resolved) = resolved else {
                return Ok(None);
            };
            progress.log(resolved_line(resolved.count, &resolved.preview));
            match action.act(tab, &resolved.element, progress).await? {
                ActionResult::Done(value) => Ok(Some(value)),
                ActionResult::NotConnected => {
                    progress.log("element was detached from the DOM, retrying");
                    Ok(None)
                }
            }
        };
        match iteration.await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) | Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// The action pre-checks wait out a pending main-frame
/// navigation, runs the locator handlers' checkpoint, then waits once more, in case a handler
/// navigated. Every caller runs it before the attempt takes the input turn, so a handler's own
/// actions can take it.
/// Playwright: `server/page.ts` `Page.performActionPreChecks`: `await this._performLocatorHandlersCheckpoint(progress);`
pub(crate) async fn wait_out_pending_navigation<T: Tab>(
    tab: &T,
    progress: &Progress,
) -> Result<(), Stop> {
    navigation_check(tab, progress).await?;
    tab.locator_handlers_checkpoint(progress).await?;
    navigation_check(tab, progress).await
}

/// The navigation half of the action pre-checks:
/// when a requested main-frame navigation has not settled, log, wait for it, and log where it
/// went.
/// Playwright: `server/page.ts` `Page._performWaitForNavigationCheck`
async fn navigation_check<T: Tab>(tab: &T, progress: &Progress) -> Result<(), Stop> {
    let Some(url) = tab.pending_navigation() else {
        return Ok(());
    };
    let to = if url.is_empty() {
        String::new()
    } else {
        format!(" \"{}\"", trim_with_ellipsis(&url, LOGGED_URL_CHARS))
    };
    progress.log(format!("  waiting for{to} navigation to finish..."));
    if let Some(committed) = progress.race(tab.navigation_settled()).await?? {
        progress.log(format!(
            "  navigated to \"{}\"",
            trim_with_ellipsis(&committed, LOGGED_URL_CHARS)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Pure policy, unit-tested without a browser.
// ---------------------------------------------------------------------------------------------

/// Loop A's waits for a call timeout in ms (`0` = none): `0`, then [`LOOP_A_BACKOFF_MS`] with
/// trailing entries over `timeout / 5` dropped, the last repeating forever. A 500 ms timeout
/// gives 0, 20, 50, 100, 100, 100….
/// Neo: stores the current position in Playwright's timeout-capped backoff sequence.
pub(crate) struct LoopAWaits {
    waits_ms: Vec<u32>,
    index: usize,
}

/// Playwright: `server/frames.ts` `Frame.retryWithProgressAndBackoff`: `while (backoffScale.length && backoffScale[backoffScale.length - 1] > progress.timeout / 5)`
/// Playwright: `server/frames.ts` `Frame.retryWithProgressAndTimeouts`: `timeouts = [0, ...timeouts];`
impl LoopAWaits {
    pub(crate) fn new(timeout_ms: f64) -> Self {
        let mut backoff = LOOP_A_BACKOFF_MS.to_vec();
        // Playwright's `if (progress.timeout)`: a negative timeout caps too, dropping every entry.
        if timeout_ms != 0.0 {
            while backoff
                .last()
                .is_some_and(|&wait| f64::from(wait) > timeout_ms / 5.0)
            {
                backoff.pop();
            }
        }
        let mut waits_ms = vec![0];
        waits_ms.extend(backoff);
        Self { waits_ms, index: 0 }
    }

    pub(crate) fn next_wait(&mut self) -> Duration {
        let last = self.waits_ms.len() - 1;
        let wait = self.waits_ms[self.index.min(last)];
        self.index += 1;
        Duration::from_millis(u64::from(wait))
    }
}

/// Loop B's wait before retry `retry` (≥ 1): `LOOP_B_WAITS_MS[min(retry - 1, 4)]`. Retry 1
/// waits 0 ms and logs no wait line.
/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `const timeout = waitTime[Math.min(retry - 1, waitTime.length - 1)];`
pub(crate) fn loop_b_wait(retry: usize) -> Duration {
    let index = retry.saturating_sub(1).min(LOOP_B_WAITS_MS.len() - 1);
    Duration::from_millis(LOOP_B_WAITS_MS[index])
}

/// `  locator resolved to <preview>`, or for a non-strict multi-match
/// `  locator resolved to N elements. Proceeding with the first one: <preview>`.
/// Playwright: `server/frames.ts` `Frame._retryWithProgressIfNotConnected`
fn resolved_line(count: u32, preview: &str) -> String {
    if count > 1 {
        format!("  locator resolved to {count} elements. Proceeding with the first one: {preview}")
    } else {
        format!("  locator resolved to {preview}")
    }
}

/// The line loop B logs for `reason`; `None` for `NotConnected`, which leaves loop B.
/// Playwright: `server/dom.ts` `ElementHandle._retryAction`
pub(crate) fn not_ready_line(reason: &NotReady) -> Option<String> {
    match reason {
        NotReady::NotConnected => None,
        NotReady::NotVisible => Some("  element is not visible".to_string()),
        NotReady::NotInViewport => Some("  element is outside of the viewport".to_string()),
        NotReady::Missing(ElementState::NonInert) => Some("  element is inert".to_string()),
        NotReady::Missing(state) => Some(format!("  element is not {}", state_name(*state))),
        NotReady::Intercepted(target) => Some(format!("  {target} intercepts pointer events")),
    }
}

/// An element state as Playwright spells it.
/// Neo: prints the generated element-state names in the shared retry log.
fn state_name(state: ElementState) -> &'static str {
    match state {
        ElementState::Visible => "visible",
        ElementState::Hidden => "hidden",
        ElementState::Enabled => "enabled",
        ElementState::Disabled => "disabled",
        ElementState::Editable => "editable",
        ElementState::Checked => "checked",
        ElementState::Unchecked => "unchecked",
        ElementState::Indeterminate => "indeterminate",
        ElementState::Stable => "stable",
        ElementState::NonInert => "non-inert",
    }
}

/// Playwright's `trimStringWithEllipsis`: unchanged within `cap` UTF-16 units, else the first
/// `cap - 1` code points and `…`.
/// Playwright: `server/page.ts` `Page._performWaitForNavigationCheck`: `trimStringWithEllipsis(url, 200)`
fn trim_with_ellipsis(input: &str, cap: usize) -> String {
    if input.encode_utf16().count() <= cap {
        return input.to_string();
    }
    if input.chars().count() <= cap {
        return input.to_string();
    }
    let mut trimmed: String = input.chars().take(cap - 1).collect();
    trimmed.push('\u{2026}');
    trimmed
}

#[cfg(test)]
mod tests {
    use std::future::{self, Future};
    use std::sync::Mutex;

    use super::*;
    use crate::playwright_methods::tab::fake::{FakeTab, SELECTOR};
    use crate::run::Delivery;

    struct ReadAction(Mutex<usize>);

    impl ElementAction<FakeTab> for ReadAction {
        type Output = String;
        fn act(
            &self,
            tab: &FakeTab,
            _element: &u32,
            _progress: &Progress,
        ) -> impl Future<Output = Result<ActionResult<String>, Stop>> + Send {
            tab.record("read action");
            let mut attempts = self.0.lock().expect("attempts");
            *attempts += 1;
            future::ready(if *attempts == 1 {
                Ok(ActionResult::NotConnected)
            } else {
                Ok(ActionResult::Done("value".into()))
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_loop_a_returns_an_action_value_and_re_resolves_detached_elements() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(30_000.0);
        let result = retry_with_progress_if_not_connected(
            &tab,
            SELECTOR,
            true,
            PreChecks::Run,
            &progress,
            &ReadAction(Mutex::new(0)),
        )
        .await;
        assert_eq!(result, Ok("value".into()));
        assert_eq!(tab.count("describe"), 1);
        assert_eq!(tab.count("resolve"), 2);
        assert_eq!(tab.count("read action"), 2);
        assert!(
            progress
                .folded_log()
                .contains(&"  - element was detached from the DOM, retrying".to_string())
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
    }
}
