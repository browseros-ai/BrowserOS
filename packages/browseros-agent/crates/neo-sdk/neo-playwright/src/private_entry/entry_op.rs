//! The entry points a host calls: the look alone, the whole entry, and whether a tab took one.
//! Neither call is an op of the script-facing table: each opens its own [`OpScope`] from a spec
//! of its own, so the door's rules (`recheck`, `mark_sent`) apply to it as to any op, and no
//! script can name it.

use browseros_core::TabId;
use tokio::time::Instant;

use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, OpTarget};
use crate::page::OpPage;
use crate::private_entry::look_op::{self, Look, TabAddress};
use crate::private_entry::{
    BoxLook, Entry, EntryCode, EntryError, EntryRequest, LookRequest, Sending,
};
use crate::target::CallHeader;
use crate::{OpEnv, Playwright};

/// The check call: a read, so it can never mark itself sent.
const LOOK: OpSpec = OpSpec::read(
    "host.fillCheck",
    OpTarget::None,
    "The host's look at boxes before a private entry. Types nothing.",
);

impl Playwright {
    /// Looks at every box of `request` in Neo's isolated world, in each box's own frame, and
    /// answers each box's facts and the first refusal an entry would meet there now (design
    /// sections 4.8 and 5.3). Types nothing and never marks the op sent. It may scroll a box
    /// into view, as main's fill did before its hit test. `Err` is a failure of the tab, the
    /// connection or the deadline, not of a box.
    pub async fn look_boxes(
        &self,
        env: &dyn OpEnv,
        request: &LookRequest,
    ) -> Result<Vec<BoxLook>, EntryError> {
        let error = |code| EntryError::of(code, Sending::NotYet);
        let scope = scope_until(env, LOOK, request.deadline);
        let page = look_op::within(&scope, OpPage::admit(self, &scope, request.tab))
            .await
            .map_err(error)?
            .map_err(|failure| error(look_op::playwright_code(&failure)))?;
        let address = TabAddress::read(&page, &scope, request.url.as_deref())
            .await
            .map_err(error)?;
        let mut looks = Vec::with_capacity(request.fields.len());
        for field in &request.fields {
            let look = look_op::look_at(&page, &scope, field, false, None, address)
                .await
                .map_err(error)?;
            looks.push(match look {
                Look::Found(found) => BoxLook {
                    facts: Some(found.facts),
                    refusal: found.refusal.map(error),
                },
                Look::Unfound(code) => BoxLook {
                    facts: None,
                    refusal: Some(error(code)),
                },
            });
        }
        Ok(looks)
    }

    /// Types every value of `request` into its box, in order, as one write (design section 5):
    /// every box is looked at before any character; the host's rules are asked again before
    /// each field and before each character of a `keys` field; each box is read back. Drops
    /// the values before it returns. Ends by `request.deadline` on its own.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "a stand-in until the entry lands; the host awaits the real one"
    )]
    pub async fn enter_privately(&self, _env: &dyn OpEnv, request: EntryRequest) -> Entry {
        let fields = request.fields.len();
        Entry::refused_before_sending(
            fields,
            0,
            EntryError::of(EntryCode::Internal, Sending::NotYet),
        )
    }

    /// Whether `tab` took a private entry: true from the moment an entry marks it, before its
    /// first character, until the tab closes, since going back in history restores a typed
    /// value into a new document that has no mark (rulings R15 and R28). The host's camera asks
    /// it before and after each capture and takes no picture of such a tab (design section
    /// 10.3, pull request 4).
    #[must_use]
    pub fn took_private_text(&self, _tab: TabId) -> bool {
        false
    }
}

/// A scope for one of the host's calls, timed to end at `deadline`.
fn scope_until(env: &dyn OpEnv, spec: OpSpec, deadline: Instant) -> OpScope<'_> {
    let timeout = look_op::timeout_until(deadline);
    let call = CallHeader {
        api: None,
        timeout_ms: Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
    };
    OpScope::of_spec(env, spec, call)
}
