//! Where keyboard input lands, for design D14. A `keyboard.*` op asks before it sends: keys
//! that copy or cut from a password, card or one-time-code field are refused.
//! A burst with a key down that copies or cuts (`input::copies`: Meta+C
//! on a Mac, Control+C elsewhere, and their cut and Insert/Delete forms) is refused there
//! instead, with nothing sent and the op not marked sent.
//!
//! ```text
//!  main frame's world ── focused ── sensitive: refuse
//!         │                       ├ an iframe: hop into its frame (hop_pinned), ask again
//!         │                       └ a possible shadow host: DOM.describeNode, a closed root: refuse
//!         └ nothing gets the keys from a sensitive field: send
//! ```
//!
//! The walk fails closed: a focused frame it cannot reach or ask in time is refused too,
//! because the selection could be in it, and so is a focused host of a closed shadow root. The
//! sealed world sees only the host of a closed root (its `shadowRoot` reads null there, and
//! `activeElement` stops at it), so the control inside that has the focus cannot be classified.

use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::resolve::{self, Hopped};
use crate::world::{Pin, Place, SealedWorld};
use crate::{ErrorCode, PlaywrightError};

/// How many focused iframes the walk follows down before it gives up and refuses.
const MAX_DEPTH: usize = 16;

/// What keyboard input reaches now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    /// Not a sensitive field.
    Plain,
    Sensitive,
    /// A frame on the way could not be asked (its document went away, or it is not there
    /// yet), or the focus is behind a closed shadow root.
    Unknown,
}

/// `DOM.describeNode`'s answer for a focused element: the shadow roots it hosts.
#[derive(Deserialize)]
struct Described {
    node: DescribedHost,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedHost {
    #[serde(default)]
    shadow_roots: Vec<DescribedRoot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedRoot {
    #[serde(default)]
    shadow_root_type: Option<RootType>,
}

/// CDP's `DOM.ShadowRootType`. Only the two the sealed world can see into, or needs not to,
/// are named; `closed`, and any type a later Chromium adds, is `Unseen`.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RootType {
    /// The world follows it (`shadowRoot.activeElement`).
    Open,
    /// A built-in control's own root, such as an `<input>`'s; the seal classifies the control.
    UserAgent,
    #[serde(other)]
    Unseen,
}

/// `script / sensitive_copy_refused`: the burst copies or cuts in a sensitive field.
pub(crate) fn copy_refused(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(
        ErrorCode::SensitiveCopyRefused,
        "copying or cutting from a password, card or one-time-code field is refused; read \
         what the page shows instead",
    )
}

/// Where keyboard input on `page` would land now. A main world that went away (a navigation
/// since the last op) is installed again once, as the resolver's liveness rule does.
pub(crate) async fn focus_of(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
) -> Result<Focus, PlaywrightError> {
    for _ in 0..2 {
        if let Some(focus) = focus(page, scope).await? {
            return Ok(focus);
        }
    }
    Ok(Focus::Unknown)
}

/// Refuses a copy or cut where `focus` says the keys land in a sensitive field, or where the
/// walk could not tell: the refusal fails closed.
pub(crate) fn refuse_copy(scope: &OpScope<'_>, focus: Focus) -> Result<(), PlaywrightError> {
    match focus {
        Focus::Plain => Ok(()),
        Focus::Sensitive => Err(copy_refused(scope)),
        Focus::Unknown => Err(scope.fail(
            ErrorCode::SensitiveCopyRefused,
            "Neo could not tell which field has the focus, so it refuses to copy or cut there",
        )),
    }
}

/// One walk from the main frame down; `None` when the main frame's world was gone, after the
/// liveness rule forgot it.
async fn focus(page: &OpPage<'_>, scope: &OpScope<'_>) -> Result<Option<Focus>, PlaywrightError> {
    let main = match page.world(&page.main_endpoint(), Place::Main).await {
        Ok(world) => world,
        Err(ProbeFailure::Gone(_)) => return Ok(None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let mut world = main.clone();
    for _ in 0..MAX_DEPTH {
        let iframe = match probe::call(&world, &ProbeRequest::Focused).await {
            Ok(ProbeReply::Focused {
                sensitive: true, ..
            }) => return Ok(Some(Focus::Sensitive)),
            Ok(ProbeReply::Focused {
                iframe: Some(pin), ..
            }) => pin,
            Ok(ProbeReply::Focused {
                host: Some(host), ..
            }) => return behind_closed_root(page, scope, &world, &main, host).await,
            Ok(ProbeReply::Focused { .. }) => return Ok(Some(Focus::Plain)),
            Ok(other) => return Err(resolve::refused(scope, other)),
            Err(ProbeFailure::Gone(_)) => return Ok(gone(page, &world, &main)),
            Err(failure) => return Err(resolve::failed(scope, failure)),
        };
        world = match resolve::hop_pinned(page, scope, &world, iframe).await? {
            Hopped::Into { child, .. } => child,
            Hopped::ParentGone => return Ok(gone(page, &world, &main)),
            Hopped::Missing | Hopped::ChildGone => return Ok(Some(Focus::Unknown)),
        };
    }
    Ok(Some(Focus::Unknown))
}

/// What the focused element `host`, pinned in `world`, hosts: `Unknown` behind a closed shadow
/// root, `Plain` otherwise. `DOM.describeNode` lists every root of a host with its type, the
/// closed ones included (measured on BrowserOS 155: depth 0 is enough, and an `<input>`'s own
/// root reads `user-agent`).
async fn behind_closed_root(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    main: &SealedWorld,
    host: Pin,
) -> Result<Option<Focus>, PlaywrightError> {
    let object = match probe::pinned_object(scope, world, host).await {
        Ok(Some(object)) => object,
        // The element left the page since `focused`: the focus moved, and went unasked.
        Ok(None) => return Ok(Some(Focus::Unknown)),
        Err(ProbeFailure::Gone(_)) => return Ok(gone(page, world, main)),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let described = world
        .session()
        .send::<_, Described>(
            "DOM.describeNode",
            json!({ "objectId": object, "depth": 0 }),
        )
        .await;
    let described = match described {
        Ok(described) => described,
        Err(error) => {
            return match ProbeFailure::from_core(error) {
                ProbeFailure::Gone(_) => Ok(gone(page, world, main)),
                failure => Err(resolve::failed(scope, failure)),
            };
        }
    };
    let unseen = described.node.shadow_roots.iter().any(|root| {
        !matches!(
            root.shadow_root_type,
            Some(RootType::Open | RootType::UserAgent)
        )
    });
    Ok(Some(if unseen { Focus::Unknown } else { Focus::Plain }))
}

/// `world` went away under the walk. It is forgotten; the main frame's world is asked again
/// once, a child frame's is not.
fn gone(page: &OpPage<'_>, world: &SealedWorld, main: &SealedWorld) -> Option<Focus> {
    page.frames().invalidate(world);
    if world.same_install(main) {
        None
    } else {
        Some(Focus::Unknown)
    }
}
