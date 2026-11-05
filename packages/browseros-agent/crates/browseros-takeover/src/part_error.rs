//! The crate's one error. Every part returns it, so the service turns the parts' failures into
//! the contract's status codes in one place (`browseros-service`'s `api_error`).

use browseros_core::{CoreError, TabId};

/// Why a part could not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub enum PartError {
    /// No part has this id, or it ended long enough ago to be forgotten.
    #[error("nothing has this id")]
    NotFound,
    /// The tab is not open in the browser.
    #[error("tab {} is not open", .0.0)]
    TabNotFound(TabId),
    /// The lock belongs to a takeover. Ending it alone would let the agent act while the owner
    /// still has control, before the takeover's sweep, so the host releases the takeover
    /// instead (decision 29).
    #[error("the lock belongs to a takeover; release the takeover instead")]
    TakeoverLock,
    /// The browser refused or did not answer.
    #[error("the browser did not answer: {0}")]
    Browser(#[source] CoreError),
}
