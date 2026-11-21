//! Explicit access restrictions on the top document. These stop the solve before CAPTCHA routing.

use serde_json::Value;

use crate::{page::CaptchaPage, types::PageError};

/// The site has explicitly refused automated access. Page text stays out of the outcome.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PolicyWall {
    /// The host from the same document read as the notice, for the handoff log.
    pub(crate) host: Option<String>,
}

const READ_NOTICE: &str = r#"(() => {
    /* captcha:policy-wall */
    const footer = 'footer, [role="contentinfo"]';
    const headings = Array.from(document.querySelectorAll('h1, h2, h3, h4, h5, h6, [role="heading"]'))
        .filter(node => !node.closest(footer) && node.checkVisibility({checkVisibilityCSS:true, checkOpacity:true}))
        .slice(0, 50)
        .map(node => node.innerText.slice(0, 300));
    let text = document.body?.getClientRects().length ? document.body.innerText : '';
    // A site's standing terms can use the same wording on pages that are open.
    for (const node of document.querySelectorAll(footer)) {
        if (node.getClientRects().length) text = text.replace(node.innerText, '\n');
    }
    // Only the start crosses CDP: a wall states itself at the top, the rules match whole lines,
    // and a long page's full text would be megabytes on every solve and every round.
    return {title: document.title.slice(0, 300), headings, text: text.slice(0, 20000), host: location.hostname};
})()"#;

pub(crate) async fn policy_wall<P: CaptchaPage>(page: &P) -> Result<Option<PolicyWall>, PageError> {
    let notice = page.eval_main(READ_NOTICE).await?;
    let headings = notice
        .get("headings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .flat_map(str::lines);
    let restriction_heading = headings.clone().any(|line| {
        matches!(
            normalize(line).trim_end_matches([':', '.', '!']),
            "access restricted" | "access denied"
        )
    });
    let title = notice
        .get("title")
        .and_then(Value::as_str)
        .into_iter()
        .flat_map(str::lines);
    let denied = title
        .chain(headings)
        .any(|line| is_denial(&normalize(line)))
        || notice
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| {
                text.lines().any(|line| {
                    let line = normalize(line);
                    is_denial(&line) || is_unauthorized_agent_notice(&line, restriction_heading)
                })
            });
    Ok(denied.then(|| PolicyWall {
        host: notice
            .get("host")
            .and_then(Value::as_str)
            .filter(|host| !host.is_empty())
            .map(str::to_owned),
    }))
}

fn normalize(line: &str) -> String {
    let words: Vec<_> = line.split_whitespace().collect();
    words.join(" ").to_ascii_lowercase()
}

/// A sentence naming an unauthorized AI agent is a wall only under the page's own "Access
/// restricted"/"Access denied" heading. Without the heading the same words are just as likely a
/// quotation (a news article about the wall), and a false wall stops the agent on an open page.
fn is_unauthorized_agent_notice(line: &str, restriction_heading: bool) -> bool {
    restriction_heading && line.contains("unauthorized ai agent")
}

fn is_denial(line: &str) -> bool {
    let line = line.trim_end_matches(['.', '!']);
    let notice = [
        " on this site",
        " on this website",
        " to access this site",
        " to access this website",
    ]
    .into_iter()
    .find_map(|suffix| line.strip_suffix(suffix))
    .unwrap_or(line);
    matches!(
        notice,
        "access restricted: unauthorized ai agent"
            | "unauthorized ai agent"
            | "no ai agents"
            | "no ai agents allowed"
            | "ai agents are not allowed"
            | "ai agents are not permitted"
            | "automated agents are not allowed"
            | "automated agents are not permitted"
    )
}

#[cfg(test)]
mod tests {
    use browseros_core::PageId;
    use serde_json::{Value, json};
    use tokio::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;

    use super::{PolicyWall, READ_NOTICE, policy_wall};
    use crate::{
        SolveCtx, SolveDriver,
        orchestration_tests::service,
        page::CaptchaPage,
        providers::CaptchaBackend,
        types::{
            Attempts, CaptchaEventName, Challenge, Detection, DocumentId, FlowResult, FrameRef,
            PageError, Rect, SolveMethod, SolveReason, SolveStatus, Verdict,
        },
    };

    struct Page(Value);

    const AMAZON_NOTICE: &str = "Continued access by an unauthorized AI agent violates Amazon's Conditions of Use, to which our customers have agreed.";

    fn amazon_wall() -> Page {
        Page(json!({
            "title": "Amazon.com",
            "headings": ["Access restricted"],
            "text": AMAZON_NOTICE,
            "host": "www.amazon.com"
        }))
    }

    impl CaptchaPage for Page {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            assert_eq!(expression, READ_NOTICE);
            Ok(self.0.clone())
        }

        async fn document(&self) -> Result<DocumentId, PageError> {
            Ok(DocumentId {
                loader_id: "original".into(),
                url: "https://fixture.test/shop".into(),
            })
        }

        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            unreachable!("the policy check only reads the top document")
        }

        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            unreachable!("a frame cannot declare the top document's policy")
        }

        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            unreachable!("a policy wall forbids interaction")
        }

        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            unreachable!("the policy check reads notice text")
        }

        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            unreachable!("the policy check only reads the top document")
        }
    }

    #[tokio::test]
    async fn tests_that_each_top_document_field_can_report_an_explicit_wall() -> anyhow::Result<()>
    {
        for notice in [
            json!({"title": "  Access restricted:   unauthorized AI agent  "}),
            json!({"headings": ["Access restricted", "No AI agents allowed"]}),
            json!({"text": "Access denied\nAutomated agents are not permitted to access this site."}),
        ] {
            assert!(policy_wall(&Page(notice)).await?.is_some());
        }
        Ok(())
    }

    #[test]
    fn tests_that_the_notice_read_caps_what_crosses_cdp() -> anyhow::Result<()> {
        for cap in [
            "text.slice(0, 20000)",
            ".slice(0, 50)",
            "innerText.slice(0, 300)",
            "title.slice(0, 300)",
        ] {
            assert!(
                READ_NOTICE.contains(cap),
                "the read runs before every solve and every round, so a long page must not \
                 send its whole text: {cap}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_recorded_amazon_notice_is_a_policy_wall() -> anyhow::Result<()> {
        assert_eq!(
            policy_wall(&amazon_wall()).await?,
            Some(PolicyWall {
                host: Some("www.amazon.com".into()),
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_agent_notice_needs_a_restriction_heading() -> anyhow::Result<()> {
        let page = Page(json!({
            "headings": ["ACCESS   DENIED"],
            "text": "We identified an UNAUTHORIZED  AI\tagent on this page."
        }));
        assert!(policy_wall(&page).await?.is_some());
        // An open article quoting the wall's sentence is not the wall (found by the live rig).
        for notice in [
            json!({
                "title": "Amazon tells AI shopping agents to stay out",
                "headings": ["Amazon tells AI shopping agents to stay out"],
                "text": format!("The page reads: \"{AMAZON_NOTICE}\"\nAnalysts expect others to follow.")
            }),
            json!({"text": AMAZON_NOTICE}),
            json!({"text": "An unauthorized AI agent is not permitted here."}),
        ] {
            assert_eq!(policy_wall(&Page(notice)).await?, None);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_ordinary_terms_and_mentions_of_bots_do_not_match() -> anyhow::Result<()> {
        let page = Page(json!({
            "title": "Store",
            "headings": ["Products", "Terms of service"],
            "text": "Browse our products.\nOur terms prohibit bots and automated agents.\nSee how unauthorized AI agent detection works."
        }));
        assert_eq!(policy_wall(&page).await?, None);

        let page = Page(json!({
            "title": "Access restricted",
            "headings": ["How detection works"],
            "text": "See how unauthorized AI agent detection works.\nEditing this article is not allowed."
        }));
        assert_eq!(policy_wall(&page).await?, None);
        Ok(())
    }

    struct NoRouting;

    impl SolveDriver for NoRouting {
        async fn detect<P: CaptchaPage>(&self, _: &P) -> Result<Detection, PageError> {
            unreachable!("an explicit access restriction takes precedence over CAPTCHA detection")
        }

        async fn solve<P: CaptchaPage, B: CaptchaBackend>(
            &self,
            _: &SolveCtx<'_, P, B>,
            _: &Challenge,
        ) -> FlowResult {
            unreachable!("no CAPTCHA flow or provider may run on a policy wall")
        }
    }

    #[tokio::test]
    async fn tests_that_a_policy_wall_hands_off_before_detecting_or_solving() -> anyhow::Result<()>
    {
        let page = amazon_wall();
        for (human_help, guidance) in [
            (true, "Call request_human_help with kind \"captcha\"."),
            (false, "Ask the user to take over this page."),
        ] {
            let outcome = service()
                .solve_with(
                    &page,
                    PageId(1),
                    CancellationToken::new(),
                    human_help,
                    Instant::now() + Duration::from_secs(5),
                    &NoRouting,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_eq!(outcome.verdict(), Verdict::PolicyWall);
            assert_eq!(outcome.status, SolveStatus::PolicyWall);
            assert_eq!(outcome.reason, SolveReason::PolicyWall);
            assert_eq!(outcome.method, SolveMethod::None);
            assert_eq!(outcome.kind, None);
            assert_eq!(outcome.attempts, Attempts::default());
            assert_eq!(outcome.paid_tasks, 0);
            assert_eq!(outcome.provider, None);
            assert!(outcome.message.contains("Stop acting on the page."));
            assert!(outcome.message.ends_with(guidance));
            let [event] = outcome.events.as_slice() else {
                anyhow::bail!("expected only the handoff event");
            };
            assert_eq!(event.event, CaptchaEventName::Handoff);
            assert_eq!(event.data.get("reason"), Some(&json!("policy_wall")));
            assert_eq!(event.data.get("humanHelp"), Some(&json!(human_help)));
        }
        Ok(())
    }
}
