//! Playwright's load states and the Chromium lifecycle events that report them.

use std::fmt;

use serde::Deserialize;
use ts_rs::TS;

/// How far a document has loaded, as Playwright names it in `waitUntil` and
/// `waitForLoadState`. Each state is reported per document by `Page.lifecycleEvent` (and the
/// commit by `Page.frameNavigated`), keyed by the document's loader id. A cached restore's
/// navigation event supplies every state because Chromium does not fire them again.
///
/// Upstream counts `networkidle` itself, from the requests of the frame's whole subtree. Neo
/// waits for Chromium's own `networkIdle` lifecycle event in each current subtree document,
/// then checks the topology stayed stable, without enabling the Network domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LoadState {
    /// The document committed: its response arrived and it replaced the previous one.
    Commit,
    /// `DOMContentLoaded` fired.
    DomContentLoaded,
    /// The `load` event fired. Playwright's default.
    #[default]
    Load,
    /// No network connections for 500 ms. Upstream also accepts `networkidle0`.
    #[serde(alias = "networkidle0")]
    NetworkIdle,
}

impl LoadState {
    /// The state a `Page.lifecycleEvent` name reports. A live commit fires `init`, and enabling
    /// lifecycle events replays an already committed document as `commit`; the other names
    /// (paints, `networkAlmostIdle`) are no load state.
    pub(crate) fn from_lifecycle(name: &str) -> Option<Self> {
        match name {
            "init" | "commit" => Some(Self::Commit),
            "DOMContentLoaded" => Some(Self::DomContentLoaded),
            "load" => Some(Self::Load),
            "networkIdle" => Some(Self::NetworkIdle),
            _ => None,
        }
    }

    /// Playwright's name, as call logs print it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::DomContentLoaded => "domcontentloaded",
            Self::Load => "load",
            Self::NetworkIdle => "networkidle",
        }
    }
}

impl fmt::Display for LoadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::LoadState;

    #[test]
    fn tests_that_waituntil_takes_playwrights_names() -> Result<(), Box<dyn Error>> {
        for (name, state) in [
            ("commit", LoadState::Commit),
            ("domcontentloaded", LoadState::DomContentLoaded),
            ("load", LoadState::Load),
            ("networkidle", LoadState::NetworkIdle),
            ("networkidle0", LoadState::NetworkIdle),
        ] {
            let parsed: LoadState = serde_json::from_value(serde_json::json!(name))?;
            assert_eq!(parsed, state, "{name}");
        }
        assert!(
            serde_json::from_value::<LoadState>(serde_json::json!("networkIdle")).is_err(),
            "the CDP spelling is not a Playwright state"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_live_commit_and_a_replayed_one_are_both_the_commit() {
        assert_eq!(LoadState::from_lifecycle("init"), Some(LoadState::Commit));
        assert_eq!(LoadState::from_lifecycle("commit"), Some(LoadState::Commit));
        assert_eq!(
            LoadState::from_lifecycle("networkAlmostIdle"),
            None,
            "almost idle is not networkidle"
        );
        assert_eq!(LoadState::from_lifecycle("firstPaint"), None);
    }
}
