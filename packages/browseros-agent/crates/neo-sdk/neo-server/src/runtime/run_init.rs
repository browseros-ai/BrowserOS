//! What a run's realm starts with, before the agent's first statement.

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::browser::PageRef;
use crate::dispatch::{FrameVersion, Observation, PageFact};

/// The run's init: the bootstrap's `__neo.run` hands it to each parameter's build
/// (`__neo.provide`), which the prelude's bridge.ts `provide` reads. `run_script` and the hosted
/// run send the same text.
#[derive(Debug, Serialize, TS)]
pub(crate) struct RunInit<'a> {
    /// The task's tab when the run service knows one; null opens one on the first op that
    /// names the page.
    page: Option<&'a PageRef>,
    /// Every tab open when the run started, as reply facts describe them, in
    /// `context.pages()`'s order; empty when the run service could not list them.
    pages: &'a [PageFact],
    /// The host observation that read `pages`, which later replies' observations follow.
    observed: Option<&'a Observation>,
    /// Every tab's frame-tree version when the run started, as every reply carries them
    /// (`Envelope::Ok` `frames`): the trees the prelude reads at the start are read at these, so
    /// the run's first reply does not take them for changes. Absent before any change.
    #[serde(skip_serializing_if = "<[FrameVersion]>::is_empty")]
    #[ts(optional = nullable)]
    frames: &'a [FrameVersion],
}

impl<'a> RunInit<'a> {
    /// The init of a run on `page` with these facts.
    #[must_use]
    pub(crate) fn new(
        page: Option<&'a PageRef>,
        pages: &'a [PageFact],
        observed: Option<&'a Observation>,
        frames: &'a [FrameVersion],
    ) -> Self {
        Self {
            page,
            pages,
            observed,
            frames,
        }
    }

    /// The init as JSON text. It goes through a `Value`, whose maps order every object's keys,
    /// nested facts included: the text the realm reads does not follow field declaration order,
    /// and `tests/envelope_generated.rs` pins it.
    #[must_use]
    pub(crate) fn to_json(&self) -> String {
        // Unreachable: these types serialize without fallible map keys. A realm given `null`
        // starts as one with no tab and no facts.
        serde_json::to_value(self)
            .unwrap_or(Value::Null)
            .to_string()
    }
}
