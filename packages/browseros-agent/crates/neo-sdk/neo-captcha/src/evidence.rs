//! The page evidence the verify check reads, and the text form the judge sees.
//!
//! One read-only DOM probe (`CaptchaPage::scan_dom`, DOM world, closed shadow roots included)
//! fills a [`PageBrief`]: what covers the page, the widget's face, the submit control, the page's
//! live alerts and status lines, and a little visible text. Tier 1 reads its facts; tier 2 sends
//! [`PageBrief::to_evidence`] to the judge as tagged lines. The brief never holds input values,
//! URL paths or queries, so neither the judge nor a log line can receive them.

use crate::{page::CaptchaPage, types::PageError};

/// Line and size caps on the judge's text. Irrelevant state lowers the judge's answer quality,
/// and input tokens are what a call costs.
pub(crate) const MAX_EVIDENCE_LINES: usize = 40;
pub(crate) const MAX_EVIDENCE_CHARS: usize = 2_000;
/// Below this much evidence the judge answers confidently on nothing, so it is not asked.
pub(crate) const MIN_EVIDENCE_CHARS: usize = 60;

/// One page line the probe read. `fresh` marks a line that was not there before the flow ran
/// (`PageBrief::new_since`), which is how a late "Verification failed" alert is told from an
/// old one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    pub fresh: bool,
}

/// Text read inside one visible cross-site frame, tagged with its host only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FrameText {
    pub host: String,
    pub text: String,
}

/// What one probe saw. Every field is already safe to send: hosts not URLs, labels not values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageBrief {
    pub site: Option<String>,
    pub title: Option<String>,
    /// What covers the view right now, in words ("dialog covering the whole view: …").
    pub on_top: Option<String>,
    /// The widget as measured ("Turnstile frame visible inside the dialog; state unreadable").
    pub widget: Option<String>,
    /// The submit control as measured ("the Search button is hidden behind the dialog").
    pub submit: Option<String>,
    pub alerts: Vec<Line>,
    pub statuses: Vec<Line>,
    pub frames: Vec<FrameText>,
    pub http_status: Option<u16>,
    pub headings: Vec<String>,
    pub buttons: Vec<String>,
    pub text: Option<String>,
}

impl PageBrief {
    /// The judge's state: one tagged line per fact, facts first, capped. The tags are the only
    /// trusted part; the judge's instructions say so.
    pub(crate) fn to_evidence(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut push = |tag: &str, value: &str| {
            let value = value.trim();
            if !value.is_empty() {
                lines.push(format!("[{tag}] {value}"));
            }
        };
        for (tag, value) in [
            ("SITE", &self.site),
            ("TITLE", &self.title),
            ("ON_TOP", &self.on_top),
            ("WIDGET", &self.widget),
            ("SUBMIT", &self.submit),
        ] {
            if let Some(value) = value {
                push(tag, value);
            }
        }
        for line in &self.alerts {
            push(if line.fresh { "NEW ALERT" } else { "ALERT" }, &line.text);
        }
        for line in &self.statuses {
            push(if line.fresh { "NEW STATUS" } else { "STATUS" }, &line.text);
        }
        for frame in &self.frames {
            push(&format!("FRAME {}", frame.host), &frame.text);
        }
        if let Some(status) = self.http_status {
            push("HTTP", &status.to_string());
        }
        for heading in self.headings.iter().take(8) {
            push("HEADING", heading);
        }
        for button in self.buttons.iter().take(10) {
            push("BUTTON", button);
        }
        if let Some(text) = &self.text {
            push("TEXT", text);
        }
        let mut evidence = String::new();
        for line in lines.into_iter().take(MAX_EVIDENCE_LINES) {
            let room = MAX_EVIDENCE_CHARS.saturating_sub(evidence.chars().count());
            if room <= 1 {
                break;
            }
            if !evidence.is_empty() {
                evidence.push('\n');
            }
            evidence.extend(line.chars().take(room - 1));
        }
        evidence
    }

    /// This brief with every alert and status line absent from `before` marked fresh.
    #[must_use]
    pub(crate) fn new_since(mut self, before: &Self) -> Self {
        for line in &mut self.alerts {
            line.fresh = !before.alerts.iter().any(|old| old.text == line.text);
        }
        for line in &mut self.statuses {
            line.fresh = !before.statuses.iter().any(|old| old.text == line.text);
        }
        self
    }
}

/// Reads the page into a brief with one DOM call. Read-only: it never focuses, clicks or writes.
///
/// Wave 2 (piece V3a) adds the probe script (hit-test at the submit control and five view points,
/// alerts, status lines, marker words, `responseStatus`, the frame texts) and drops lines holding
/// an e-mail address or a long digit run. Until then it reads nothing, and an empty brief is
/// below [`MIN_EVIDENCE_CHARS`], so the check stays `unsure` rather than guess.
pub(crate) async fn brief<P: CaptchaPage>(_page: &P) -> Result<PageBrief, PageError> {
    Ok(PageBrief::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny_failure_dialog() -> PageBrief {
        PageBrief {
            site: Some("ouf.osc.ny.gov".into()),
            title: Some("Search for Unclaimed Property".into()),
            on_top: Some(
                "dialog covering the whole view: Please check the box below to continue. Close"
                    .into(),
            ),
            widget: Some(
                "Cloudflare Turnstile frame is visible inside the dialog; its state could not be \
                 read; an older response token is present"
                    .into(),
            ),
            submit: Some("the Search button is hidden behind the dialog".into()),
            alerts: vec![Line {
                text: "There was an issue verifying your browser. Please try again.".into(),
                fresh: false,
            }],
            ..PageBrief::default()
        }
    }

    #[test]
    fn tests_that_evidence_puts_measured_facts_first_with_their_tags() {
        let evidence = ny_failure_dialog().to_evidence();
        let tags: Vec<&str> = evidence
            .lines()
            .filter_map(|line| line.split(']').next())
            .collect();
        assert_eq!(
            tags,
            ["[SITE", "[TITLE", "[ON_TOP", "[WIDGET", "[SUBMIT", "[ALERT"],
            "the judge trusts only the tag, and facts lead so they survive the cap"
        );
        assert!(evidence.chars().count() >= MIN_EVIDENCE_CHARS);
    }

    #[test]
    fn tests_that_evidence_is_capped_in_lines_and_characters() {
        let brief = PageBrief {
            buttons: (0..100).map(|n| format!("Button {n}")).collect(),
            headings: (0..100).map(|n| format!("Heading {n}")).collect(),
            frames: (0..100)
                .map(|n| FrameText {
                    host: format!("frame{n}.example"),
                    text: "x".repeat(300),
                })
                .collect(),
            ..ny_failure_dialog()
        };
        let evidence = brief.to_evidence();
        assert!(evidence.lines().count() <= MAX_EVIDENCE_LINES);
        assert!(evidence.chars().count() <= MAX_EVIDENCE_CHARS);
    }

    #[test]
    fn tests_that_only_lines_new_since_the_solve_began_are_fresh() {
        let before = ny_failure_dialog();
        let mut after = ny_failure_dialog();
        after.alerts.push(Line {
            text: "Verification failed".into(),
            fresh: false,
        });
        let after = after.new_since(&before);
        assert_eq!(
            after
                .alerts
                .iter()
                .map(|line| line.fresh)
                .collect::<Vec<_>>(),
            [false, true]
        );
        assert!(
            after
                .to_evidence()
                .contains("[NEW ALERT] Verification failed")
        );
    }

    #[test]
    fn tests_that_an_empty_brief_is_too_thin_to_ask_about() {
        assert!(PageBrief::default().to_evidence().chars().count() < MIN_EVIDENCE_CHARS);
    }
}
