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

use crate::PlaywrightError;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::site_sweep::{self, ReadMask};
use crate::probe::{ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget};
use crate::resolve::{self, HandleSite, Resolved, Resolver};
use crate::target::{ElementTarget, LocatorTarget, Strictness, Target};

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
    /// pass means its node went away under the action, and it fails `element_detached`.
    pub(crate) async fn locate(&self, pass: usize) -> Result<Option<Resolved>, PlaywrightError> {
        match &self.of {
            Of::Locator { resolver, .. } => resolver.locate().await,
            Of::Handle(_) if pass > 0 => Err(resolve::handle_detached(self.scope)),
            Of::Handle(target) => match HandleSite::find(self.page, self.scope, target).await? {
                Some(site) => Ok(Some(Resolved::of_handle(&site))),
                None => Err(resolve::handle_detached(self.scope)),
            },
        }
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

    /// `request` in `located`'s world, from its root node when there is one, and once more with
    /// the site's values when its reply would hold page text the engine cut (`Resolved::call`).
    pub(crate) async fn call(
        &self,
        located: &Resolved,
        request: &ProbeRequest,
    ) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
        located.call(self.page, self.scope, request).await
    }

    /// A reply in `located`'s world that ends the action. A strict violation previews each
    /// match's markup, which is page text, so its message is masked with the element's site
    /// first (`site_sweep::refused`); any other reply is `resolve::refused`'s.
    pub(crate) async fn refused(&self, located: &Resolved, reply: ProbeReply) -> PlaywrightError {
        let world = located.world().clone();
        let pw = self.page.playwright();
        site_sweep::refused(pw, self.scope, self.page, world, reply, ReadMask::Textless).await
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
