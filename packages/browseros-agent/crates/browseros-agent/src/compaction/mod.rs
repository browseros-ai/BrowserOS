//! Context-window compaction, an exact port of TS `prepareStep` compaction (recon A5): UTF-16
//! token estimates, `pruneMessages`, `stripBinary`, `dropOldest`, sticky within a turn. Plain
//! functions the loop calls before every model call — not a trait (one behaviour, no second
//! adapter). Implemented by P6; S0 stubs never compact.

use crate::messages::ModelMessage;

/// Default context window when the request carries none (TS `computeBudget`).
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub window: u64,
    pub reserve: u64,
    pub overhead: u64,
    pub threshold: u64,
}

impl Budget {
    /// `computeBudget`. S0 stub: the whole window, no reserve, so nothing ever compacts.
    pub fn for_context_window(context_window: Option<f64>) -> Self {
        let window = context_window
            .filter(|window| window.is_finite() && *window > 0.0)
            .map_or(DEFAULT_CONTEXT_WINDOW, |window| window as u64);
        Self {
            window,
            reserve: 0,
            overhead: 0,
            threshold: window,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStage {
    Untouched,
    PrunedOld,
    PrunedAll,
    DroppedOldest { still_over_budget: bool },
}

/// The prepareStep compaction: called by the loop before every model call; the result replaces
/// the working transcript. S0 stub: untouched.
pub fn prepare_step(
    messages: Vec<ModelMessage>,
    steps: &[StepUsage],
    budget: &Budget,
) -> (Vec<ModelMessage>, CompactionStage) {
    let _ = (steps, budget);
    (messages, CompactionStage::Untouched)
}

/// UTF-16 lengths: `ceil(chars / 3) + images * 1000`. S0 stub: 0.
pub fn estimate_tokens(messages: &[ModelMessage]) -> u64 {
    let _ = messages;
    0
}
