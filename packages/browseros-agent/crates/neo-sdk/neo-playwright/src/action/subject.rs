//! What an action runs on: a locator, found again on every pass of the outer loop, or an
//! element handle's node, found once.
//!
//! ```text
//!  locator   pass 0, 1, 2 ...  Resolver::locate ─ none yet: wait │ detached: resolve again
//!  handle    pass 0 only       HandleSite::find ─ gone: "Element is not attached to the DOM"
//!            the first request targets the node itself (`ProbeTarget::Root`)
//! ```
//!
//! Upstream runs a locator action as `_retryWithProgressIfNotConnected` around the handle
//! action, and a handle action alone, so a handle whose node detaches fails at once where a
//! locator looks again (server/dom.ts `throwRetargetableDOMError`). Every runner asks its
//! [`Subject`] instead of branching on the target itself.
//!
//! Each pass starts with the page's locator-handler checkpoint, as both of upstream's loops do
//! (`locator_handler.rs`): for a handle, the one pass stands for its first attempt's.

use crate::locator_handler::{self, Prechecks};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget};
use crate::resolve::{self, HandleSite, Resolved, Resolver};
use crate::target::{ElementTarget, LocatorTarget, Strictness, Target};
use crate::{ErrorCode, PlaywrightError};

/// One action's target, for one op.
pub(crate) struct Subject<'a> {
    page: &'a OpPage<'a>,
    scope: &'a OpScope<'a>,
    of: Of<'a>,
}

enum Of<'a> {
    Locator {
        target: &'a LocatorTarget,
        resolver: Resolver<'a>,
    },
    Handle(&'a ElementTarget),
}

impl<'a> Subject<'a> {
    pub(crate) fn new(
        page: &'a OpPage<'a>,
        scope: &'a OpScope<'a>,
        target: &'a Target,
        strictness: Strictness,
    ) -> Self {
        let of = match target {
            Target::Locator(target) => Of::Locator {
                target,
                resolver: Resolver::new(page, scope, target, strictness),
            },
            Target::Element(target) => Of::Handle(target),
        };
        Self { page, scope, of }
    }

    /// Upstream's first call-log line, `waiting for locator(..)`; a handle action has none.
    pub(crate) fn log_waiting(&self) {
        if let Of::Locator { target, .. } = &self.of {
            self.scope.log(format!("waiting for {}", target.display));
        }
    }

    /// Where the element is on outer pass `pass`: `None` while a locator matches nothing yet
    /// or a frame on its way is not there. A handle is found on the first pass only; a later
    /// pass means its node went away under the action, and it fails `element_detached`. The
    /// page's locator handlers run first unless `prechecks` skips them.
    pub(crate) async fn locate(
        &self,
        pass: usize,
        prechecks: Prechecks,
    ) -> Result<Option<Resolved>, PlaywrightError> {
        let runs = match prechecks {
            Prechecks::Run => true,
            Prechecks::LocatorPasses => matches!(self.of, Of::Locator { .. }),
            Prechecks::Skip => false,
        };
        if runs {
            self.run_handlers().await?;
        }
        match &self.of {
            Of::Locator { resolver, .. } => resolver.locate().await,
            Of::Handle(_) if pass > 0 => Err(resolve::handle_detached(self.scope)),
            Of::Handle(target) => match HandleSite::find(self.page, self.scope, target).await? {
                Some(site) => Ok(Some(Resolved::of_handle(&site))),
                None => Err(resolve::handle_detached(self.scope)),
            },
        }
    }

    /// The page's locator-handler checkpoint, for an inner loop's retry. Unbounded, as
    /// [`Subject::locate`] is: the runner races it with its op's timeout.
    pub(crate) async fn run_handlers(&self) -> Result<(), PlaywrightError> {
        locator_handler::checkpoint(self.scope, self.page).await
    }

    /// The target of each pass's first request, before anything is pinned: the locator's last
    /// chunk, or the handle's node.
    pub(crate) fn first_target(&self) -> ProbeTarget {
        match &self.of {
            Of::Locator { target, .. } => {
                ProbeTarget::Chunk(ParsedSelector::of(target.chunks.last()))
            }
            Of::Handle(_) => ProbeTarget::Root,
        }
    }

    /// The element left its document between two requests: a locator logs upstream's line and
    /// goes round the outer loop again; a handle fails.
    pub(crate) fn detached(&self) -> Result<(), PlaywrightError> {
        match &self.of {
            Of::Locator { .. } => {
                self.scope
                    .log("element was detached from the DOM, retrying".to_owned());
                Ok(())
            }
            Of::Handle(_) => Err(resolve::handle_detached(self.scope)),
        }
    }

    /// A failure upstream's locator loop goes round again on, where a handle action, which
    /// upstream runs once, fails with it: every error but one thrown by the page's script
    /// (frames.ts `isNonRetriableError`), such as a CDP error or a file that went away. The
    /// error is only built for a handle.
    pub(crate) fn retry_unless_handle(
        &self,
        error: impl FnOnce() -> PlaywrightError,
    ) -> Result<(), PlaywrightError> {
        match &self.of {
            Of::Locator { .. } => Ok(()),
            Of::Handle(_) => Err(error()),
        }
    }

    /// `request` in `located`'s world, from its root node when there is one (`Resolved::call`).
    pub(crate) async fn call(
        &self,
        located: &Resolved,
        request: &ProbeRequest,
    ) -> Result<ProbeReply, ProbeFailure> {
        located.call(request).await
    }

    /// `website / not_focused`: an edit focused the element to type into it, and the element did
    /// not keep the focus, so nothing was typed. Neo's own failure, where upstream types into
    /// whatever has the focus (owner's standing rule: Neo never acts on an element other than the
    /// one asked for). `holder` is the preview of the element that has the focus now. Never
    /// retried: the page that moved the focus would move it again.
    pub(crate) fn not_focused(&self, holder: Option<&str>) -> PlaywrightError {
        let holder = holder.unwrap_or("no element");
        self.scope.fail(
            ErrorCode::NotFocused,
            &format!("Element did not keep the focus; {holder} has it now, so nothing was typed"),
        )
    }

    /// A reply that ends the action (`resolve::refused`).
    pub(crate) fn refused(&self, reply: ProbeReply) -> PlaywrightError {
        resolve::refused(self.scope, reply)
    }

    /// The world `located` names went away under a call. It is forgotten; a locator resolves
    /// again, unless a ref jump landed there (whose document is gone for good), and a handle's
    /// document is gone with its node.
    pub(crate) fn lost(&self, located: &Resolved) -> Result<(), PlaywrightError> {
        match &self.of {
            Of::Locator { resolver, .. } => resolver.lost(located),
            Of::Handle(_) => {
                self.page.frames().invalidate(located.world());
                Err(resolve::handle_detached(self.scope))
            }
        }
    }
}
