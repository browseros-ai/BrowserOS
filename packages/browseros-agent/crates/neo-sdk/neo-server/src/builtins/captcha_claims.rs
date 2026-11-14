//! Which tabs a CAPTCHA solve holds, and the rule that keeps other writes off them.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::TabId;
use neo_trace::{Effect, OperationId};

use crate::dispatch::{ErrorCode, OperationError, Policy, PolicyCheck};

/// The tabs `neo.solveCaptcha` ops hold, and the CAPTCHA busy rule (design D7 rule 4).
///
/// The solver drives the page itself, through core and not through the door, so while it works
/// the door must keep the script's other writes off that tab. The crate reserves the page too,
/// but only inside `CaptchaSolver::solve`, after a CDP round trip: a rule that read those
/// reservations would let two solves in one `Promise.all` both pass, and the loser would be
/// refused only after it was marked sent. A claim here is taken synchronously before anything
/// is sent and released when the solve's op ends, so it covers the crate's reservation in time,
/// and it is the one record of "busy" claw-sdk reads. Every solve in the process goes through
/// `neo.solveCaptcha`, so nothing reserves a page behind its back.
///
/// Reads pass: the solver tolerates them, as claw-server's busy guard does.
#[derive(Default)]
pub(crate) struct CaptchaClaims {
    /// Keyed by tab, like every rule's input (`PolicyCheck::tab`).
    held: Mutex<HashMap<TabId, OperationId>>,
}

/// One solve's hold on a tab. Dropping it releases the tab, also when the op is cut short by
/// the run's stop or its task is aborted, because the op's future owns it.
#[must_use]
pub(crate) struct CaptchaClaim {
    claims: Arc<CaptchaClaims>,
    tab: TabId,
    holder: OperationId,
}

impl CaptchaClaims {
    /// Claims `tab` for the solve `holder`, or names the op that already holds it.
    pub(crate) fn try_claim(
        self: &Arc<Self>,
        tab: TabId,
        holder: OperationId,
    ) -> Result<CaptchaClaim, OperationId> {
        match self.lock().entry(tab) {
            Entry::Occupied(held) => Err(*held.get()),
            Entry::Vacant(slot) => {
                slot.insert(holder);
                Ok(CaptchaClaim {
                    claims: Arc::clone(self),
                    tab,
                    holder,
                })
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TabId, OperationId>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Policy for CaptchaClaims {
    /// Refuses every write to a claimed tab except the claiming solve's own recheck.
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let tab = match (check.method.effect, check.tab) {
            (Effect::Write, Some(tab)) => tab,
            (Effect::Read, _) | (Effect::Write, None) => return Ok(()),
        };
        match self.lock().get(&tab) {
            Some(holder) if *holder != check.operation => Err(busy(*holder)),
            Some(_) | None => Ok(()),
        }
    }
}

impl Drop for CaptchaClaim {
    fn drop(&mut self) {
        let mut held = self.claims.lock();
        // Only its own hold: the entry is replaced by nothing else while it lives, but a claim
        // never removes a hold it does not own.
        if held.get(&self.tab) == Some(&self.holder) {
            held.remove(&self.tab);
        }
    }
}

/// `script / page_busy`: the script overlapped a write with a solve on the same page. Nothing
/// was sent; awaiting the solve first is the fix.
pub(super) fn busy(holder: OperationId) -> OperationError {
    OperationError::script(
        ErrorCode::PAGE_BUSY,
        format_args!(
            "neo.solveCaptcha (operation {holder}) is working on this page; await it, then look at the page again"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::TabId;
    use neo_trace::{Effect, OperationId, RunId};

    use super::CaptchaClaims;
    use crate::dispatch::{Caller, ErrorCode, Listing, MethodSpec, Policy, PolicyCheck};

    fn check(
        claims: &CaptchaClaims,
        effect: Effect,
        tab: Option<TabId>,
        operation: OperationId,
    ) -> Result<(), ErrorCode> {
        let method = MethodSpec::new("test.op", effect);
        claims
            .check(&PolicyCheck {
                method: &method,
                caller: Caller::Script,
                tab,
                operation,
                lock_pass: None,
            })
            .map_err(|error| error.code())
    }

    #[test]
    fn a_claimed_tab_refuses_other_writes_until_the_claim_drops() {
        let claims = Arc::new(CaptchaClaims::default());
        let run = RunId::new();
        let (solve, click) = (OperationId::new(run, 1), OperationId::new(run, 2));
        let tab = TabId(7);
        let claim = claims.try_claim(tab, solve);
        assert!(claim.is_ok(), "a free tab is claimed");
        assert_eq!(
            claims.try_claim(tab, click).err(),
            Some(solve),
            "a second solve learns which op holds the tab"
        );
        assert_eq!(
            check(&claims, Effect::Write, Some(tab), click),
            Err(ErrorCode::PAGE_BUSY)
        );
        assert_eq!(
            check(&claims, Effect::Write, Some(tab), solve),
            Ok(()),
            "the solve's own recheck passes"
        );
        assert_eq!(
            check(&claims, Effect::Read, Some(tab), click),
            Ok(()),
            "reads pass"
        );
        assert_eq!(
            check(&claims, Effect::Write, Some(TabId(8)), click),
            Ok(()),
            "other tabs are free"
        );
        drop(claim);
        assert_eq!(
            check(&claims, Effect::Write, Some(tab), click),
            Ok(()),
            "the claim ends with the solve's op"
        );
    }

    #[test]
    fn enabled_cdp_respects_every_other_claim_and_their_release() -> Result<(), Box<dyn Error>> {
        let claims = Arc::new(CaptchaClaims::default());
        let run = RunId::new();
        let (solve, other) = (OperationId::new(run, 1), OperationId::new(run, 2));
        let own = claims
            .try_claim(TabId(7), solve)
            .map_err(|holder| format!("tab 7 already claimed by {holder}"))?;
        let foreign = claims
            .try_claim(TabId(8), other)
            .map_err(|holder| format!("tab 8 already claimed by {holder}"))?;
        let method = MethodSpec {
            listing: Listing::Own("neo.cdp"),
            ..MethodSpec::new("neo.cdp", Effect::Write)
        };
        let check = |tab, operation| {
            claims
                .check(&PolicyCheck {
                    method: &method,
                    caller: Caller::Script,
                    tab,
                    operation,
                    lock_pass: None,
                })
                .map_err(|error| error.code())
        };
        for tab in [None, Some(TabId(7)), Some(TabId(8)), Some(TabId(9))] {
            assert_eq!(
                check(tab, solve),
                Err(ErrorCode::PAGE_BUSY),
                "raw CDP reaches another solve's tab even through its holder's own page"
            );
        }
        drop(foreign);
        for tab in [None, Some(TabId(7)), Some(TabId(9))] {
            assert_eq!(check(tab, solve), Ok(()), "an op may pass its own claim");
            assert_eq!(check(tab, other), Err(ErrorCode::PAGE_BUSY));
        }
        drop(own);
        for tab in [None, Some(TabId(7)), Some(TabId(9))] {
            assert_eq!(check(tab, other), Ok(()), "release restores raw CDP");
        }
        Ok(())
    }
}
