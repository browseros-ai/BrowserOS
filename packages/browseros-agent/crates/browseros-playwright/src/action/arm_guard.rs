//! The armed hit-target interceptor of one pointer attempt, and its disarm on every exit.

use std::sync::Arc;

use browseros_core::ProtocolSession;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

use crate::input::TabInput;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::world::SealedWorld;
use crate::{PlaywrightError, resolve};

/// An interceptor armed in one world. Playwright's interceptor blocks every later pointer
/// event in its frame once it saw a miss (and every one in a trial), so one left armed would
/// break the page for its owner too. Every exit stops it:
/// - the normal path asks for the [`ArmGuard::verdict`], and the probe stops it while
///   answering;
/// - an error, a timeout or a dropped future spawns a `disarm` from `Drop`;
/// - the next arming request in the same world stops a stale one first;
/// - the probe stops one whose verdict never came by its expiry, which Rust sets 3 s past the
///   op's timeout (`pointer_action.rs`, `arm_expiry`), once no event of the op can still come; a
///   verdict asked after that fails instead of vouching for the events it no longer saw.
///
/// The spawned disarm first takes the tab's input lock. An interceptor is live only while its
/// op holds that lock, so a disarm that waits for it can only find a stale one, never stop
/// another op's live interceptor in the same world. Holding the lock, it first sends the
/// releases the op's unfinished burst owes (`PageInput`), so they reach the page while the
/// interceptor still guards it: a cancelled `trial` never turns into a real click.
#[must_use = "dropping the guard disarms the interceptor, which ends the attempt's hit check"]
#[derive(Debug)]
pub(crate) struct ArmGuard {
    /// `None` once the probe answered the verdict.
    world: Option<SealedWorld>,
    token: u32,
    input: Arc<Mutex<TabInput>>,
    /// The tab's root session and pointer scale, for the owed releases.
    session: ProtocolSession,
    pointer_scale: f64,
}

impl ArmGuard {
    /// The interceptor `act { arm }` armed with `token` in `world`.
    pub(crate) fn new(
        world: SealedWorld,
        token: u32,
        input: Arc<Mutex<TabInput>>,
        session: ProtocolSession,
        pointer_scale: f64,
    ) -> Self {
        Self {
            world: Some(world),
            token,
            input,
            session,
            pointer_scale,
        }
    }

    /// Stops the interceptor and reads what it saw. Once the probe answered (with a verdict or
    /// an error), or the world is gone, there is nothing left to disarm; any other failure
    /// leaves it to `Drop`.
    ///
    /// The interceptor described what was in the way during the trusted event, outside any
    /// probe call, where no mask is in force; the probe masks that text whole (page-mask.ts)
    /// and describes the element again while answering. That second description is cut under
    /// the call's mask, so the verdict is asked as any action request is (`resolve::ask`): a
    /// miss answers `needsMask` first, and the probe keeps the verdict for its token, so asking
    /// again answers the same one.
    pub(crate) async fn verdict(
        &mut self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
    ) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
        let Some(world) = self.world.clone() else {
            return Ok(Err(ProbeFailure::Broken(
                "the interceptor's verdict was already read".to_owned(),
            )));
        };
        let request = ProbeRequest::Verdict { token: self.token };
        let reply = resolve::ask(page, scope, &world, &request, None).await;
        // A sweep that failed after the first answer still found the interceptor stopped.
        if matches!(reply, Ok(Ok(_) | Err(ProbeFailure::Gone(_))) | Err(_)) {
            self.world = None;
        }
        reply
    }
}

impl Drop for ArmGuard {
    fn drop(&mut self) {
        let Some(world) = self.world.take() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        drop(runtime.spawn(disarm_when_idle(
            world,
            Arc::clone(&self.input),
            self.session.clone(),
            self.pointer_scale,
        )));
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "the releases and the disarm run while no op holds the tab's input; see ArmGuard"
)]
async fn disarm_when_idle(
    world: SealedWorld,
    input: Arc<Mutex<TabInput>>,
    session: ProtocolSession,
    pointer_scale: f64,
) {
    let mut tab = input.lock().await;
    tab.flush_owed(&session, pointer_scale).await;
    // Best effort: a world that is gone took its interceptor with it.
    let _disarmed = probe::call(&world, &ProbeRequest::Disarm).await;
}
