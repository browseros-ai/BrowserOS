//! The values a run's trace records are masked with beyond the process's secret set.

use std::sync::Arc;

use browseros_trace::SecretSet;

use super::{ErrorCode, OperationError};

/// What a withheld input becomes in its record ([`Held::Lost`]).
pub(crate) const WITHHELD: &str = "[withheld]";

/// The values Neo masks out of a run's reply that the process's secret set has not learned yet,
/// so that the run's trace records keep none of them either (decision 34).
///
/// builtins' `TypedValues` owns them: what was typed into, or read from, a password, card or code
/// field that is not done yet (sdk-traces decision 32), as browseros-playwright's journal and
/// reads report it. A run watches them from its start ([`HeldValues::watch`]). The door asks the
/// watch as it sends each record, and the answer rides in the record's `also_mask`. The trace
/// writer masks a record later, with the process's set as it is then, and that set learns a value
/// only once its field goes quiet. Without this, a value the reply hides stays raw in everything
/// the host reads: `/v1/runs`, `/v1/operations`, `/v1/trace`, `/v1/events` and `trace.db`.
pub(crate) trait HeldValues: Send + Sync {
    /// Starts keeping, for a run that begins now, every value held from now on, until the watch
    /// it returns is dropped. The run's context owns the watch, so it lasts as long as anything
    /// can still send one of the run's records, an op that settles after the run's end included.
    fn watch(self: Arc<Self>) -> Box<dyn HeldWatch>;
}

/// One run's watch on the held values ([`HeldValues::watch`]).
pub(crate) trait HeldWatch: Send + Sync {
    /// Every value held at any moment since the run began, as of now.
    fn held(&self) -> Held;
}

/// What a run's records and its reply are masked with beyond the process's set, as of asking.
pub(crate) enum Held {
    /// Every value held at any moment since the run began, the ones superseded since included, as
    /// a set of its own that registers nothing in the process's set. `None`, with nothing built,
    /// when there is none.
    Kept(Option<Arc<SecretSet>>),
    /// One of those values is gone, forgotten past the cap on what is kept, and the run may have
    /// read it. The rest still mask its records, but nothing masks that one: what the run
    /// produces is withheld from every record it sends from now on, and from its reply.
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
                "while this run went on, more values were typed into password, card or code \
                 fields than Neo keeps to mask its output, so the output could show one of them \
                 and is withheld",
            )),
        }
    }
}
