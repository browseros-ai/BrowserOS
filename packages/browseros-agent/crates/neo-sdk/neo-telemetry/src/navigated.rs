//! One navigation, as the server hands it to telemetry.

use crate::label::{HttpStatus, NetError};

/// A navigation the browser was sent and that ended on its own ([`crate::RunTelemetry::navigated`]):
/// what its document arrived with, or why the network refused it. A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Navigated {
    /// The new document's HTTP status, when the reply carried one.
    pub status: Option<HttpStatus>,
    /// Why it failed on the network, when it did.
    pub error: Option<NetError>,
}
