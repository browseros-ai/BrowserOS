//! Explicit screenshot masks resolve all matches in their owning utility worlds. The parent
//! preparation guard retains these worlds and the shared core capture lease through cleanup,
//! including lost mutation replies and cancellation. No elements are selected automatically.

use std::time::Duration;

use futures_util::future;
use serde::Deserialize;
use tokio::time;
use ts_rs::TS;

use crate::PlaywrightError;

/// The prelude's locator brand, including its `$neo` discriminant. Element handles are not
/// accepted as mask options; the caller names locators that resolve all current matches.
#[derive(Debug, Deserialize, TS)]
#[serde(tag = "$neo")]
pub(super) enum MaskLocator {
    #[serde(rename = "locator")]
    Locator(LocatorTarget),
}
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{
    self, Control, ParsedSelector, ProbeFailure, ProbePolicy, ProbeReply, ProbeRequest, ProbeTarget,
};
use crate::resolve::Resolver;
use crate::target::{LocatorTarget, Strictness};
use crate::world::IsolatedWorld;

/// Worlds remembered before applying any overlay, so a lost reply cannot lose its cleanup.
#[derive(Default)]
pub(super) struct ScreenshotMasks {
    /// Retain the operation policy alongside each world: restoration may run after the
    /// screenshot's scope was dropped, so it uses the restricted cleanup sender.
    worlds: Vec<MaskedWorld>,
}

/// One overlay's cleanup authority survives the operation that installed it.
struct MaskedWorld {
    world: IsolatedWorld,
    policy: ProbePolicy,
}

impl ScreenshotMasks {
    pub(super) fn is_empty(&self) -> bool {
        self.worlds.is_empty()
    }
    pub(super) async fn apply(
        &mut self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        locators: &[MaskLocator],
        color: &str,
    ) -> Result<(), PlaywrightError> {
        for MaskLocator::Locator(locator) in locators {
            // A locator on another page cannot affect this image. Do not mutate a page whose
            // capture queue this screenshot does not own.
            if locator.page.tab() != page.tab() {
                continue;
            }
            let resolver = Resolver::new(page, scope, locator, Strictness::FirstMatch);
            let Some(resolved) = resolver.locate().await? else {
                continue;
            };
            self.worlds.push(MaskedWorld {
                world: resolved.world().clone(),
                policy: ProbePolicy::for_cleanup(scope),
            });
            let request = ProbeRequest::ScreenshotMask {
                target: ProbeTarget::Chunk(ParsedSelector::of(locator.chunks.last())),
                color: color.to_owned(),
            };
            match resolved.call(scope, page, &request).await {
                Ok(ProbeReply::Value { .. }) | Err(ProbeFailure::Gone(_)) => {}
                Ok(reply) => return Err(resolver.refused(reply)),
                Err(failure) => return Err(resolver.failed(failure)),
            }
        }
        Ok(())
    }

    pub(super) async fn restore(&mut self) {
        future::join_all(self.worlds.iter().map(|masked| async {
            let _restored = time::timeout(
                Duration::from_secs(5),
                probe::cleanup(masked.policy, &masked.world, Control::ScreenshotMaskCleanup),
            )
            .await;
        }))
        .await;
        self.worlds.clear();
    }
}
