//! Progress events emitted by long-running engine operations.
//!
//! Engines report; renderers decide. `--json` and non-TTY runs install a
//! no-op sink so machine output never carries progress noise.

/// One progress signal from an engine phase.
#[derive(Clone, Copy, Debug)]
pub enum ProgressEvent<'a> {
    /// A phase began; `total` is None when the size is unknowable.
    Start {
        phase: &'a str,
        total: Option<usize>,
    },
    /// Work advanced within a phase; `item` names what is being processed.
    Tick {
        phase: &'a str,
        done: usize,
        total: Option<usize>,
        item: Option<&'a str>,
    },
    /// The phase finished.
    End { phase: &'a str },
}

/// Sink signature engines thread through long loops.
pub type ProgressSink<'a> = dyn FnMut(ProgressEvent<'_>) + 'a;

/// A sink that drops every event.
pub fn noop() -> impl FnMut(ProgressEvent<'_>) {
    |_| {}
}
