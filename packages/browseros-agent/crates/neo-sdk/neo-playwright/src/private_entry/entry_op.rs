//! The entry points a host calls: the look alone, the whole entry, and whether a tab took one.
//! Neither call is an op of the script-facing table: each opens its own [`OpScope`] from a spec
//! of its own, so the door's rules (`recheck`, `mark_sent`) apply to it as to any op, and no
//! script can name it.

use browseros_core::TabId;
use tokio::time::Instant;

use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, OpTarget};
use crate::page::OpPage;
use crate::private_entry::entry_run::EntryRun;
use crate::private_entry::look_op::{self, Look, MainNavigation, TabAddress};
use crate::private_entry::{
    BoxLook, Entry, EntryCode, EntryError, EntryRequest, LookRequest, Sending,
};
use crate::target::CallHeader;
use crate::{OpEnv, Playwright};

/// The entry: a write, so the door's rules run again right before its first change.
const ENTRY: OpSpec = OpSpec::write(
    "host.fill",
    OpTarget::None,
    "The host's private entry: types values the host names into boxes it names.",
);

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
    /// connection or the deadline, or a host policy's refusal at admission; any other failure is
    /// that box's refusal, and the other boxes are still looked at (ruling R32).
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
            .map_err(|failure| look_op::error_of(&failure, Sending::NotYet))?;
        let mut navigation = MainNavigation::watch(self, &page);
        let address = navigation
            .race(TabAddress::read(&page, &scope, request.url.as_deref()))
            .await
            .unwrap_or(Ok(TabAddress::Changed))
            .map_err(error)?;
        let mut looks = Vec::with_capacity(request.fields.len());
        for field in &request.fields {
            let look = navigation
                .race(look_op::look_at(&page, &scope, field, false, None, address))
                .await
                // The main frame began to navigate: the page is changing under the approval.
                .unwrap_or(Ok(Look::Unfound(EntryCode::PageChanged)));
            looks.push(match look {
                Ok(Look::Found(found)) => BoxLook {
                    facts: Some(found.facts),
                    refusal: found.refusal.map(error),
                },
                // The tab, the connection or the deadline ends the call for every box.
                Err(code) if look_op::ends_the_call(code) => return Err(error(code)),
                // Anything else is this box's answer, and the rest are still looked at.
                Ok(Look::Unfound(code)) | Err(code) => BoxLook {
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
    /// the values before it returns. Ends by `request.deadline` on its own. An entry whose
    /// deadline has passed when it is called types nothing and sends nothing: it answers
    /// `run_timeout`, not sent and retryable, before it makes its scope (ruling R42), because a
    /// scope's own timeout cuts its waits one by one and could let a first focus or key through.
    pub async fn enter_privately(&self, env: &dyn OpEnv, request: EntryRequest) -> Entry {
        let EntryRequest {
            tab,
            url,
            fields,
            deadline,
        } = request;
        if fields.is_empty() {
            return Entry::refused_before_sending(
                0,
                0,
                EntryError::of(EntryCode::Internal, Sending::NotYet),
            );
        }
        if deadline <= Instant::now() {
            return Entry::refused_before_sending(
                fields.len(),
                0,
                EntryError::of(EntryCode::RunTimeout, Sending::NotYet),
            );
        }
        let scope = scope_until(env, ENTRY, deadline);
        let entry = EntryRun::new(self, &scope, fields.len())
            .enter(tab, url.as_deref(), &fields)
            .await;
        // The values are zeroed here, before the answer leaves.
        drop(fields);
        entry
    }

    /// Whether `tab` took a private entry: true from the moment an entry marks it, before its
    /// first character, until the tab closes, since going back in history restores a typed
    /// value into a new document that has no mark (rulings R15 and R28). The host's camera asks
    /// it before and after each capture and takes no picture of such a tab (design section
    /// 10.3, pull request 4).
    #[must_use]
    pub fn took_private_text(&self, tab: TabId) -> bool {
        self.private_tabs().took(tab)
    }

    /// Marks `tab` as having taken a private entry, as if this process had typed into it: for
    /// a host that kept the fact across a Neo restart while the browser kept the tab (ruling
    /// R45). From then on Neo's camera and a script's own pictures refuse the tab, as
    /// [`Playwright::took_private_text`] says. Nothing unmarks a tab.
    pub fn keep_private_tab(&self, tab: TabId) {
        self.private_tabs().mark(tab);
    }

    /// Treats every tab as having taken a private entry, for this instance's life: for a host
    /// that cannot tell which tabs its record names (it could not list the browser's tabs at
    /// start). Neo's camera and a script's own pictures then refuse every tab: the record fails
    /// closed.
    pub fn keep_every_tab(&self) {
        self.private_tabs().mark_every();
    }
}

/// A scope for one of the host's calls, timed to end at `deadline`.
fn scope_until(env: &dyn OpEnv, spec: OpSpec, deadline: Instant) -> OpScope<'_> {
    let timeout = look_op::timeout_until(deadline);
    let call = CallHeader {
        api: None,
        timeout_ms: Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        conformance: false,
    };
    OpScope::of_spec(env, spec, call)
}
