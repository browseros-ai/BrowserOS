//! The values a run's trace records are masked with beyond the process's secret set.

use std::sync::Arc;

use neo_trace::{RunId, SecretSet};

use super::{ErrorCode, OperationError};

/// What a withheld input becomes in its record ([`Held::Lost`]).
pub(crate) const WITHHELD: &str = "[withheld]";

/// The values Neo masks out of a run's reply that the process's secret set has not learned yet,
/// so that the run's trace records keep none of them either (decision 34), and its script's realm
/// never receives one (decision 35b).
///
/// builtins' `TypedValues` owns them: what was typed into, or read from, a password, card or code
/// field that is not done yet (sdk-traces decision 32), as neo-playwright's journal and
/// reads report it. A run watches them from its start ([`HeldValues::watch`]). The door asks the
/// watch as it sends each record, and the answer rides in the record's `also_mask`. The trace
/// writer masks a record later, with the process's set as it is then, and that set learns a value
/// only once its field goes quiet. Without this, a value the reply hides stays raw in everything
/// the host reads: `/v1/runs`, `/v1/operations`, `/v1/trace`, `/v1/events` and `trace.db`.
pub(crate) trait HeldValues: Send + Sync {
    /// Starts keeping, for run `run`, which begins now, every value held from now on, until the
    /// watch it returns is dropped. The run's context owns the watch, so it lasts as long as
    /// anything can still send one of the run's records, an op that settles after the run's end
    /// included. The id names the run in what Neo logs about it.
    fn watch(self: Arc<Self>, run: RunId) -> Box<dyn HeldWatch>;
}

/// One run's watch on the held values ([`HeldValues::watch`]).
pub(crate) trait HeldWatch: Send + Sync {
    /// Every value held at any moment since the run began, as of now.
    fn held(&self) -> Held;
}

/// What a run's records, its reply and what enters its realm are masked with beyond the process's
/// set, as of asking.
pub(crate) enum Held {
    /// Every value held at any moment since the run began, the ones superseded since included, as
    /// a set of its own that registers nothing in the process's set. `None`, with nothing built,
    /// when there is none.
    Kept(Option<Arc<SecretSet>>),
    /// One of those values is out of the process's set's reach, and the run may have read it: it
    /// was forgotten past the cap on what is kept, or the process's set was full and refused it.
    /// So what the run produces is withheld from every record it sends from now on, from its
    /// reply, and from what enters its realm. The set still masks what those records keep, its
    /// end's code and host data included: every value the run kept, a refused one too, but never
    /// a forgotten one.
    Lost(Option<Arc<SecretSet>>),
}

impl Held {
    /// The set a record carries as its `also_mask`.
    #[must_use]
    pub(crate) fn set(&self) -> Option<Arc<SecretSet>> {
        match self {
            Self::Kept(set) | Self::Lost(set) => set.as_ref().map(Arc::clone),
        }
    }

    /// For a [`Held::Lost`] run, the error its reply and each later record carry in place of what
    /// the script and its pages produced. Neo's own limit is the cause, so Neo owns it.
    #[must_use]
    pub(crate) fn withheld(&self) -> Option<OperationError> {
        match self {
            Self::Kept(_) => None,
            Self::Lost(_) => Some(OperationError::neo(
                ErrorCode::OUTPUT_WITHHELD,
                "while this run went on, Neo ran out of room to mask a value from a password, \
                 card or code field, so the output could show it and is withheld",
            )),
        }
    }
}
