//! `agent`: runs one turn for a target. The server's own tool loop over a one-call model, or
//! a coding agent through the acpx CLI; which one ran is invisible above this module, because
//! both emit `TurnEvent`s into the run's sink and return one `TurnEnd`.
//!
//! ```text
//!   Target::Provider ──▶ store row + credentials ──▶ own_loop: Model::step ⇄ Browser::call
//!   Target::Coding   ──▶ agent row ──────────────▶ acpx: child process, ACP lines → TurnEvent
//! ```
//!
//! A turn starts from the stored transcript every time (design rule 3): nothing is kept in
//! memory between turns for replay. acpx owns its native session history.

mod acpx;
mod context;
mod own_loop;
mod probe;
mod prompt;

pub use probe::{AgentProbe, ProbeError, ProbeModel, ProbeReasoning, ProbedAgentInfo};

use std::sync::Arc;
use std::time::Duration;

use crate::browser::Browser;
use crate::model::Models;
use crate::store::Store;
use crate::turn::{AgentKind, ConversationId, EventSink, Target, TurnEnd, TurnInput, TurnRunner};

/// Today's limit on model calls in one turn of the own loop.
const MAX_STEPS: usize = 100;
/// The longest one turn of the own loop may run; it then ends as if its steps were spent.
const MAX_TURN_TIME: Duration = Duration::from_secs(60 * 60);

/// The one production `TurnRunner`. Cheap to clone.
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

struct Inner {
    models: Models,
    browser: Browser,
    store: Store,
    runtime: AgentRuntime,
    acpx_sessions: acpx::Sessions,
    model_rejections: probe::ModelRejections,
}

/// Where the bundled Bun is and where agent state goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntime {
    /// `<resources>/bin/third_party/bun`, when the resources ship it; else the host's `npx`.
    pub bun: Option<std::path::PathBuf>,
    /// Where acpx's package cache, session store and the shared workspace live.
    pub state_dir: std::path::PathBuf,
}

impl Agent {
    pub fn new(models: Models, browser: Browser, store: Store, runtime: AgentRuntime) -> Self {
        Self {
            inner: Arc::new(Inner {
                models,
                browser,
                store,
                runtime,
                acpx_sessions: acpx::Sessions::default(),
                model_rejections: probe::ModelRejections::default(),
            }),
        }
    }

    /// Models and reasoning levels a coding agent advertises. A probe that cannot run
    /// returns a result with `error` set, never an `Err`: the dialog must still open.
    pub async fn probe(&self, kind: AgentKind, timeout: std::time::Duration) -> AgentProbe {
        let mut report = probe::probe(&self.inner.runtime, kind, timeout).await;
        self.inner
            .model_rejections
            .annotate(kind.into(), &mut report.models);
        report
    }

    /// Retire this server's acpx owners after conversations stop admitting and running turns.
    pub async fn shutdown_sessions(&self, within: Duration) {
        self.inner
            .acpx_sessions
            .shutdown(&self.inner.runtime, within)
            .await;
    }
}

/// Resolves the target (a provider row with credentials, or an agent row), then runs the
/// turn. A target that cannot be resolved is a `TurnEnd::Failed`, shown as an `error`
/// chunk: there is no second, pre-stream error path.
#[async_trait::async_trait]
impl TurnRunner for Agent {
    async fn run_turn(
        &self,
        target: &Target,
        input: &TurnInput,
        sink: &mut dyn EventSink,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> TurnEnd {
        let inner = &self.inner;
        match target {
            Target::Provider(id) => {
                own_loop::run(
                    &inner.models,
                    &inner.browser,
                    &inner.store,
                    id.as_deref(),
                    input,
                    sink,
                    cancel,
                )
                .await
            }
            Target::Coding { kind, agent_id } => {
                acpx::run(inner, *kind, agent_id, input, sink, cancel).await
            }
        }
    }

    async fn end_session(&self, conversation: ConversationId) {
        acpx::end_session(&self.inner, conversation).await;
    }
}
