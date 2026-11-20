//! The values that cross the seam to `neo-server`: inputs from the prelude, verdicts back to it.

use serde::{Deserialize, Serialize};

/// One snapshot of the page as the prelude rendered it (`page.ariaSnapshot(..)` text).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PageText {
    /// The page URL, shown to Jev and echoed in a miss.
    pub url: String,
    /// The page title, shown to Jev and echoed in a miss.
    pub title: String,
    /// The snapshot text: `- role "name" [ref=e12]` lines, indentation = nesting.
    pub snapshot: String,
}

/// Options for `find` and `find_all`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case", default)]
pub struct FindOpts {
    /// Keep only candidates of this ARIA role (also the way to shrink a page that is too big).
    pub role: Option<String>,
    /// Earlier picks this run; a pick of the same ref under another ask sets `Flags::reused`.
    pub picked: Vec<EarlierPick>,
}

/// One earlier `find` decision of this run.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EarlierPick {
    /// The ref that was picked.
    pub r#ref: String,
    /// The ask it was picked for.
    pub ask: String,
}

/// What `find` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindOutcome {
    /// One twin group leads clearly; `pick` holds it.
    Found,
    /// `none` leads: the element is not on the page (yet). The prelude re-looks until its timeout.
    NotFound,
    /// Jev thinks it is there but cannot pick one. Waiting will not help.
    Ambiguous,
}

/// `find`'s verdict.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FindVerdict {
    /// Found, not found or ambiguous.
    pub outcome: FindOutcome,
    /// The chosen element; `Some` exactly when `outcome` is `Found`.
    pub pick: Option<Pick>,
    /// The top three OTHER twin groups, best first, for the miss report and the trail.
    pub lookalikes: Vec<Pick>,
    /// Warnings for the trail.
    pub flags: Flags,
    /// What Jev was shown.
    pub saw: Saw,
    /// The obstacle Jev saw on the page, if any.
    pub obstacle: Obstacle,
    /// What the decision cost.
    pub cost: Cost,
}

/// One element (twin group) as a verdict names it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Pick {
    /// The ref the prelude passes to `page.getByRef` (the group's representative).
    pub r#ref: String,
    /// `role "name"` for the trail.
    pub label: String,
    /// Jev's probability, summed over the twin group.
    pub p: f32,
}

/// What the trail and a stop report about how `find` decided.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Flags {
    /// The top group was already picked this run for a different ask (that ask). The gate then
    /// answers `Ambiguous`, never `Found`.
    pub reused: Option<String>,
}

/// `find_all`'s verdict: every matching element in page order.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FindAllVerdict {
    /// Matching groups in document order (at or above `gate::MATCH`, outside `gate::UNSURE`).
    pub picks: Vec<Pick>,
    /// Groups Jev could not call either way (inside `gate::UNSURE`), document order. Not empty:
    /// the prelude stops the script with these as the lookalikes instead of returning `picks`.
    pub unsure: Vec<Pick>,
    /// What Jev was shown.
    pub saw: Saw,
    /// The obstacle Jev saw on the page, if any.
    pub obstacle: Obstacle,
    /// What the decision cost.
    pub cost: Cost,
}

/// `check`'s verdict.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckVerdict {
    /// The claim holds.
    pub yes: bool,
    /// Jev's probability that it holds.
    pub p: f32,
    /// `p` sits inside `gate::UNSURE`. At the last look the prelude stops the script on it.
    pub unsure: bool,
    /// What Jev was shown.
    pub saw: Saw,
    /// The obstacle Jev saw on the page, if any.
    pub obstacle: Obstacle,
    /// What the decision cost.
    pub cost: Cost,
}

/// `choose`'s verdict: one of the script's own strings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ChooseVerdict {
    /// The winning option, exactly as the script spelled it.
    pub choice: String,
    /// Its probability.
    pub p: f32,
    /// Every option with its probability, best first.
    pub ranked: Vec<(String, f32)>,
    /// The top option reaches `gate::ACCEPT` and the next stays under `gate::RIVAL`. Not clear:
    /// the prelude stops the script instead of returning `choice`.
    pub clear: bool,
    /// What Jev was shown.
    pub saw: Saw,
    /// The obstacle Jev saw on the page, if any.
    pub obstacle: Obstacle,
    /// What the decision cost.
    pub cost: Cost,
}

/// What Jev was shown, so a miss can say "it never saw the text" or "the page was cut".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Saw {
    /// Snapshot lines that carry a ref.
    pub elements: usize,
    /// Candidates offered to Jev after twin merging and the role filter.
    pub candidates: usize,
    /// The snapshot carried `text:` lines (check/choose mode).
    pub text: bool,
    /// Choice batches sent (each ≤ 254 candidates + `none`).
    pub batches: usize,
    /// The long-context header was sent.
    pub long_context: bool,
    /// Why anything was left out; `None` when nothing was.
    pub cut: Option<String>,
}

/// The obstacle question asked alongside every request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Obstacle {
    /// Nothing in the way.
    #[default]
    None,
    /// A CAPTCHA or bot check.
    Captcha,
    /// A sign-in wall.
    LoginWall,
    /// A cookie consent banner.
    CookieBanner,
    /// An error page (404, 500, blocked).
    ErrorPage,
    /// Another modal dialog on top.
    Modal,
}

/// What one verb call cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Cost {
    /// HTTP requests made (retries included).
    pub calls: u32,
    /// Wall time of the verb, milliseconds.
    pub ms: u32,
    /// Billable input tokens Jev reported, summed.
    pub input_tokens: u32,
}
