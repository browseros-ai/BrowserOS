//! Whether this server offers the decision loop, which is one environment
//! variable away from being a different product.
//!
//! It lives here rather than beside the tool because more than the tool needs
//! the answer: the catalogue, the instructions, the tool's own description and
//! the managed skill all behave differently when the loop is present, and the
//! skill loader is a service that must not reach up into the API layer to ask.

/// Whether the decision loop is offered by this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursueOffering {
    /// There is a credential, so the loop can run and is advertised.
    Offered,
    /// There is none. Everything behaves exactly as it did before the loop
    /// existed, which is what every production install sees.
    Absent,
}

impl PursueOffering {
    /// Asks the environment, now.
    #[must_use]
    pub fn read() -> Self {
        if credential().is_some() {
            Self::Offered
        } else {
            Self::Absent
        }
    }

    #[must_use]
    pub fn is_offered(self) -> bool {
        self == Self::Offered
    }
}

/// The credential itself, for the one place that builds a client with it.
///
/// Separate from [`PursueOffering`] because the value is a secret and the answer
/// to "should this exist" is not. Never logged.
pub fn credential() -> Option<String> {
    std::env::var("JEV_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::PursueOffering;

    /// A blank or whitespace credential is no credential, which is what keeps a
    /// half-set variable from advertising a tool that cannot run. Asserted as a
    /// shape rather than through the environment, which no test here owns.
    #[test]
    fn the_offering_is_a_yes_or_a_no() {
        assert!(PursueOffering::Offered.is_offered());
        assert!(!PursueOffering::Absent.is_offered());
    }
}
