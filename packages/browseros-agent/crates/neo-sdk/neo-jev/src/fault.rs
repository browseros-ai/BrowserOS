//! Transport faults: the only errors a Jev call returns. Messages never carry the key or page text.

/// Why a Jev call could not produce a verdict. A miss is a verdict, never one of these.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JevFault {
    /// Jev did not answer: network error, timeout, a 5xx, or a reply we could not read.
    #[error("Jev is down: {0}")]
    Down(String),
    /// The request is over the hard cap even with the long-context header; scope it with `role`.
    #[error("the page is too big for Jev ({tokens} tokens); narrow it with `role`")]
    TooBig {
        /// Estimated input tokens of the request that was refused.
        tokens: u32,
    },
    /// Jev refused the request (a 4xx other than the one long-context retry): bad key, bad model.
    #[error("Jev refused the request (HTTP {status})")]
    Refused {
        /// The HTTP status Jev answered.
        status: u16,
    },
}
