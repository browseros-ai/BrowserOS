//! Decides why a flow handed back before anything is held against it. The flow's original
//! input is replayed up to the failed step without writing: if it still works, the site is fine
//! and the new input was the problem; if it fails too, the site changed.

use super::{
    FlowExec,
    compose::Mode,
    lifecycle::Cause,
    outcome::{Handover, Outcome},
    store::FlowDoc,
};
use regex::Regex;
use std::sync::LazyLock;

static WALL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)verify you are (a )?human|captcha|are you a robot|i am not a robot|unusual traffic",
    )
    .ok()
});
static SIGN_IN_URL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)/(signin|sign-in|login|log-in|ap/signin)\b").ok());

/// Whether the page the flow stopped on is a wall or still loading, rather than the page it
/// expected. A slow page fails the original input's replay too, so it must be caught here.
fn hit_a_wall(handover: &Handover) -> bool {
    if handover
        .page
        .get("isLoading")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return true;
    }
    let view = handover.view.as_deref().unwrap_or("");
    let url = handover
        .page
        .get("url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    WALL.as_ref().is_some_and(|re| re.is_match(view))
        || SIGN_IN_URL.as_ref().is_some_and(|re| re.is_match(url))
}

pub async fn cause(exec: &dyn FlowExec, doc: &FlowDoc, handover: &Handover) -> Cause {
    if handover.kind == "time_cap" || hit_a_wall(handover) {
        return Cause::Transient;
    }
    let failed = handover.step.unwrap_or(1);
    // Verification never performs the write, so a step after it cannot be replayed. The agent
    // saw the page and may skip the patch if it had simply not updated yet.
    if doc.commit_step().is_some_and(|commit| failed > commit) {
        return Cause::SiteChanged;
    }
    let Some(original) = doc
        .meta
        .examples
        .iter()
        .find(|example| example.source == "original")
        .or_else(|| doc.meta.examples.first())
    else {
        return Cause::SiteChanged;
    };
    match exec
        .exec(doc, &original.input, Mode::Triage, Some(failed))
        .await
    {
        Outcome::Reached | Outcome::VerifiedToCommit { .. } | Outcome::Done { .. } => {
            Cause::InputOnly
        }
        Outcome::NeedsAgent(replay) if replay.kind == "time_cap" || hit_a_wall(&replay) => {
            Cause::Transient
        }
        Outcome::NeedsAgent(_) => Cause::SiteChanged,
        // The browser could not even run the replay; that says nothing about the site.
        Outcome::RunFailed(_) | Outcome::Parsed => Cause::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::flow::{
        outcome::Committed,
        store::{Effects, Example, FlowMeta, FlowStep},
    };
    use futures_util::future::BoxFuture;
    use serde_json::{Map, Value, json};
    use std::sync::Mutex;

    struct Replay(Mutex<Option<Outcome>>, Mutex<Option<(Mode, Option<u32>)>>);

    impl FlowExec for Replay {
        fn exec<'a>(
            &'a self,
            _doc: &'a FlowDoc,
            _inputs: &'a Map<String, Value>,
            mode: Mode,
            stop_after: Option<u32>,
        ) -> BoxFuture<'a, Outcome> {
            if let Ok(mut seen) = self.1.lock() {
                *seen = Some((mode, stop_after));
            }
            let outcome = self
                .0
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .unwrap_or(Outcome::RunFailed("none".into()));
            Box::pin(async move { outcome })
        }
    }

    fn handover(kind: &str, step: u32, view: &str, url: &str) -> Handover {
        Handover {
            kind: kind.into(),
            step: Some(step),
            data: json!({}),
            done: json!([]),
            committed: Committed::No,
            page: json!({ "url": url }),
            view: Some(view.into()),
            duration_ms: None,
        }
    }

    fn doc() -> FlowDoc {
        FlowDoc {
            meta: FlowMeta {
                name: "neo-x".into(),
                flow_version: 1,
                effects: Effects::Write,
                hosts: vec![],
                inputs: Default::default(),
                output: Default::default(),
                examples: vec![Example {
                    input: Map::new(),
                    source: "original".into(),
                    expect: "done".into(),
                }],
                source_session: None,
            },
            steps: (1..=4)
                .map(|n| FlowStep {
                    n,
                    commit: n == 3,
                    src: String::new(),
                })
                .collect(),
        }
    }

    async fn cause_with(
        replay: Option<Outcome>,
        failed: Handover,
    ) -> (Cause, Option<(Mode, Option<u32>)>) {
        let exec = Replay(Mutex::new(replay), Mutex::new(None));
        let cause = cause(&exec, &doc(), &failed).await;
        let seen = exec.1.lock().ok().and_then(|seen| *seen);
        (cause, seen)
    }

    #[tokio::test]
    async fn the_original_input_tells_a_site_change_from_a_bad_input() {
        let (cause, seen) = cause_with(
            Some(Outcome::Reached),
            handover("not_found", 2, "", "http://shop/s"),
        )
        .await;
        assert_eq!(cause, Cause::InputOnly);
        assert_eq!(seen, Some((Mode::Triage, Some(2))));
        let failed_again = Outcome::NeedsAgent(handover("not_found", 2, "", "http://shop/s"));
        let (cause, _) = cause_with(
            Some(failed_again),
            handover("not_found", 2, "", "http://shop/s"),
        )
        .await;
        assert_eq!(cause, Cause::SiteChanged);
    }

    #[tokio::test]
    async fn walls_and_timeouts_are_nobodys_fault() {
        let (cause, seen) = cause_with(None, handover("time_cap", 2, "", "")).await;
        assert_eq!((cause, seen), (Cause::Transient, None));
        let (cause, _) = cause_with(
            None,
            handover("wait_timeout", 2, "- heading \"Verify you are human\"", ""),
        )
        .await;
        assert_eq!(cause, Cause::Transient);
        let (cause, _) = cause_with(
            None,
            handover("not_found", 1, "", "https://shop/ap/signin?x"),
        )
        .await;
        assert_eq!(cause, Cause::Transient);
        let mut loading = handover("wait_timeout", 2, "", "http://shop/s");
        loading.page = json!({ "url": "http://shop/s", "isLoading": true });
        let (cause, _) = cause_with(None, loading).await;
        assert_eq!(cause, Cause::Transient);
    }

    #[tokio::test]
    async fn a_failure_after_the_write_is_not_replayed() {
        let (cause, seen) = cause_with(None, handover("wait_timeout", 4, "", "")).await;
        assert_eq!((cause, seen), (Cause::SiteChanged, None));
    }
}
