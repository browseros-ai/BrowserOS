//! Replays the recorded Nov 19 pages (`tests/briefs/*.json`) through the verify check: each
//! reaches its expected standing with its Jev reading, no page labelled blocked, checking or
//! policy ever reads clear, a lying judge cannot clear a proven block, an unavailable judge
//! leaves an unclear page unsure, and the evidence sent to Jev stays capped and private.

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use futures_util::future::BoxFuture;
use neo_captcha::{
    judge::{HttpJudge, Judge, JudgeAnswer, JudgeConfig, JudgeFault, JudgeReply, JudgeRequest},
    testing::{Answer, Replay, replay_brief},
    types::{BlockCause, SolveReason, SolveStatus, VerifyResult, VerifyTier},
};
use serde::Deserialize;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

/// What one recording is labelled and expected to give.
#[derive(Deserialize)]
struct Brief {
    case: String,
    truth: Truth,
    /// The Jev reading the replay's judge answers with; absent when tier 1 settles the page.
    #[serde(default)]
    jev: Option<Reading>,
    expect: Expect,
}

/// What a person reading the trace saw: the ground truth the check must never contradict
/// with a clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Truth {
    Blocked,
    Checking,
    Policy,
    Clear,
}

#[derive(Deserialize)]
struct Reading {
    /// Measured (model and date) or assumed; assumed readings only pin the code path.
    source: String,
    standing: BTreeMap<String, f64>,
    blocked: f64,
}

#[derive(Debug, PartialEq, Deserialize)]
struct Expect {
    result: VerifyResult,
    tier: VerifyTier,
    #[serde(default)]
    cause: Option<BlockCause>,
}

struct Recording {
    path: PathBuf,
    text: String,
    brief: Brief,
}

fn briefs() -> Result<Vec<Recording>, Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/briefs");
    let mut recordings = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let text = fs::read_to_string(&path)?;
            let brief: Brief = serde_json::from_str(&text)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            recordings.push(Recording { path, text, brief });
        }
    }
    recordings.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(recordings)
}

/// A judge that answers every call with one reading, as Jev's wire would carry it. Options the
/// reading leaves out are zero, taken from the request so the replay needs no copy of them.
struct Fixed {
    standing: BTreeMap<String, f64>,
    noul: f64,
}

impl Fixed {
    fn of(reading: &Reading) -> Self {
        Self {
            standing: reading.standing.clone(),
            noul: reading.blocked,
        }
    }

    /// Certain the page is clear, on every page.
    fn liar() -> Self {
        Self {
            standing: BTreeMap::from([("clear".to_owned(), 1.0)]),
            noul: 0.0,
        }
    }
}

impl Judge for Fixed {
    fn ask<'a>(
        &'a self,
        request: &'a JudgeRequest,
        _: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>> {
        let mut answers = BTreeMap::new();
        for question in &request.questions {
            let answer = if question.options.is_empty() {
                JudgeAnswer {
                    noul: Some(self.noul),
                    probabilities: BTreeMap::new(),
                }
            } else {
                JudgeAnswer {
                    noul: None,
                    probabilities: question
                        .options
                        .iter()
                        .map(|option| {
                            let p = self.standing.get(option.id).copied().unwrap_or(0.0);
                            (option.id.to_owned(), p)
                        })
                        .collect(),
                }
            };
            answers.insert(question.id.to_owned(), answer);
        }
        Box::pin(std::future::ready(Ok(JudgeReply {
            answers,
            input_tokens: None,
            model: Some("replay".into()),
        })))
    }
}

/// Jev down: no key, a timeout, a refusal.
struct Down;

impl Judge for Down {
    fn ask<'a>(
        &'a self,
        _: &'a JudgeRequest,
        _: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>> {
        Box::pin(std::future::ready(Err(JudgeFault::Unavailable(
            "replay: Jev is down",
        ))))
    }
}

async fn replay(recording: &Recording, judge: Option<&dyn Judge>) -> Result<Replay, String> {
    replay_brief(&recording.text, judge)
        .await
        .map_err(|error| format!("{}: {error}", recording.brief.case))
}

fn expected(replay: &Replay) -> Expect {
    Expect {
        result: replay.report.result,
        tier: replay.report.tier,
        cause: replay.report.cause,
    }
}

#[tokio::test]
async fn tests_that_every_brief_reaches_its_expected_standing() -> TestResult {
    for recording in briefs()? {
        let brief = &recording.brief;
        let judge: Box<dyn Judge> = match &brief.jev {
            Some(reading) => Box::new(Fixed::of(reading)),
            None => Box::new(Down),
        };
        let replay = replay(&recording, Some(judge.as_ref())).await?;
        assert_eq!(
            expected(&replay),
            brief.expect,
            "{}: evidence was\n{}",
            brief.case,
            replay.evidence
        );
        assert_eq!(
            replay.report.judge_calls,
            u32::from(brief.jev.is_some()),
            "{}: Jev is asked exactly when tier 1 found hints and no proof",
            brief.case
        );
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_no_blocked_checking_or_policy_page_reads_clear() -> TestResult {
    let mut guarded = 0;
    for recording in briefs()? {
        let brief = &recording.brief;
        if brief.truth == Truth::Clear {
            continue;
        }
        guarded += 1;
        let reading = brief.jev.as_ref().map(Fixed::of);
        for judge in [
            reading.as_ref().map(|judge| judge as &dyn Judge),
            Some(&Down as &dyn Judge),
            None,
        ] {
            let replay = replay(&recording, judge).await?;
            assert_ne!(
                replay.report.result,
                VerifyResult::Clear,
                "{} is {:?} but read clear",
                brief.case,
                brief.truth
            );
        }
    }
    assert!(
        guarded >= 12,
        "the zero-wrong-clear claim needs a real set of blocked pages, not {guarded}"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_lying_judge_cannot_clear_a_proven_block() -> TestResult {
    let liar = Fixed::liar();
    for recording in briefs()? {
        let brief = &recording.brief;
        if brief.expect.tier != VerifyTier::Deterministic {
            continue;
        }
        let replay = replay(&recording, Some(&liar)).await?;
        assert_eq!(
            expected(&replay),
            brief.expect,
            "{}: tier 1 settled it, so no judge is asked",
            brief.case
        );
        assert_eq!(replay.report.judge_calls, 0, "{}", brief.case);
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_without_jev_an_unclear_page_is_unsure() -> TestResult {
    let mut unclear = 0;
    for recording in briefs()? {
        let brief = &recording.brief;
        if brief.expect.tier != VerifyTier::Jev {
            continue;
        }
        unclear += 1;
        for (judge, calls) in [(None, 0), (Some(&Down as &dyn Judge), 1)] {
            let replay = replay(&recording, judge).await?;
            assert_eq!(
                (replay.report.result, replay.report.judge_calls),
                (VerifyResult::Unsure, calls),
                "{}: no Jev answer is unknown, never a guess",
                brief.case
            );
        }
    }
    assert!(unclear >= 6, "only {unclear} briefs reach Jev");
    Ok(())
}

#[tokio::test]
async fn tests_that_the_evidence_stays_capped_and_private() -> TestResult {
    const TAGS: [&str; 14] = [
        "[SITE] ",
        "[TITLE] ",
        "[ON_TOP] ",
        "[WIDGET] ",
        "[SUBMIT] ",
        "[ALERT] ",
        "[NEW ALERT] ",
        "[STATUS] ",
        "[NEW STATUS] ",
        "[FRAME ",
        "[HTTP] ",
        "[HEADING] ",
        "[BUTTON] ",
        "[TEXT] ",
    ];
    const JARGON: [&str; 7] = [
        "token_applied",
        "already_clear",
        "already_passed",
        "not_blocking",
        "recaptcha_v2",
        "solved",
        "token",
    ];
    for recording in briefs()? {
        let case = &recording.brief.case;
        let evidence = replay(&recording, Some(&Fixed::liar())).await?.evidence;
        assert!(evidence.chars().count() <= 2_000, "{case}");
        assert!(evidence.lines().count() <= 40, "{case}");
        for line in evidence.lines() {
            assert!(
                TAGS.iter().any(|tag| line.starts_with(tag)),
                "{case}: untagged line {line}"
            );
        }
        assert!(!evidence.contains('@'), "{case}: an e-mail address");
        assert!(!evidence.contains("://"), "{case}: a link with its path");
        assert!(
            !evidence
                .split_whitespace()
                .any(|word| word.contains("www.") && word.contains('/')),
            "{case}: a scheme-less link with its path"
        );
        assert!(
            !evidence
                .split(|c: char| !c.is_ascii_digit())
                .any(|run| run.len() >= 7),
            "{case}: a long digit run"
        );
        let lower = evidence.to_lowercase();
        for word in JARGON {
            assert!(
                !lower.contains(word),
                "{case}: solver jargon {word:?} pulls the judge toward the claim"
            );
        }
    }
    Ok(())
}

#[test]
fn tests_that_the_set_covers_both_sides_and_says_where_readings_come_from() -> TestResult {
    let recordings = briefs()?;
    let clear = recordings
        .iter()
        .filter(|recording| recording.brief.truth == Truth::Clear)
        .count();
    assert!(recordings.len() >= 24, "{} briefs", recordings.len());
    assert!(clear >= 10 && recordings.len() - clear >= 12);
    for recording in &recordings {
        if let Some(reading) = &recording.brief.jev {
            assert!(
                reading.source.starts_with("measured") || reading.source.starts_with("assumed"),
                "{}: a reading must say whether it was measured",
                recording.brief.case
            );
        }
    }
    Ok(())
}

/// A host with no judge keeps a claim only when the page merely hid part of itself: a page that
/// warns (an alert saying verification failed, a heading saying access is restricted) stays
/// unknown, while an ordinary dialog over a clear page keeps `already_clear`.
#[tokio::test]
async fn tests_that_without_a_judge_warnings_stay_unknown_and_soft_hints_keep_the_claim()
-> TestResult {
    let recordings = briefs()?;
    let answer = |case: &str| {
        let recording = recordings
            .iter()
            .find(|recording| recording.brief.case == case)
            .ok_or_else(|| format!("{case} is missing"));
        async move { Ok::<_, String>(replay(recording?, None).await?.answer) }
    };
    for case in [
        "06-amazon-wall-reworded-for-jev",
        "12-mm-failure-alert-without-dialog",
    ] {
        assert_eq!(
            answer(case).await?,
            Some(Answer {
                status: SolveStatus::Failed,
                reason: SolveReason::AcceptanceUnconfirmed,
            }),
            "{case}: the page warns, and no judge can clear it"
        );
    }
    for case in ["20-airfrance-cookie-dialog", "22-amazon-choose-location"] {
        assert_eq!(
            answer(case).await?,
            Some(Answer {
                status: SolveStatus::AlreadyClear,
                reason: SolveReason::NoCaptcha,
            }),
            "{case}: an ordinary dialog is no reason to doubt a clear page"
        );
    }
    Ok(())
}

/// The three pages claudef measured Jev on (design-claudef §5) render the same tagged shape it
/// sent: facts first, the page's own words after, host only.
#[tokio::test]
async fn tests_that_the_measured_pages_render_claudefs_shapes() -> TestResult {
    let shapes: [(&str, &[&str]); 3] = [
        (
            "01-ny-failure-dialog-already-passed",
            &[
                "[SITE] ouf.osc.ny.gov",
                "[ON_TOP] dialog covering the whole view: Please check the box below to continue. Close",
                "[WIDGET] Cloudflare Turnstile widget is visible inside the dialog",
                "[SUBMIT] the Search button is covered by the dialog",
                "[ALERT] There was an issue verifying your browser. Please try again. If this problem persists, refresh the page and submit the form again.",
            ],
        ),
        (
            "15-mm-results-hidden-widget-for-jev",
            &[
                "[SITE] missingmoney.com",
                "[WIDGET] Cloudflare Turnstile widget is on the page but hidden; its own frame could not be read",
                "[SUBMIT] the SEARCH button is visible and enabled",
            ],
        ),
        (
            "06-amazon-wall-reworded-for-jev",
            &[
                "[SITE] www.amazon.com",
                "[HEADING] Access restricted",
                "[TEXT] Access restricted Continued access by an unauthorized AI agent violates Amazon's Conditions of Use, to which our customers have agreed. Continue Conditions of Use Privacy Notice",
            ],
        ),
    ];
    let recordings = briefs()?;
    for (case, lines) in shapes {
        let recording = recordings
            .iter()
            .find(|recording| recording.brief.case == case)
            .ok_or_else(|| format!("{case} is missing"))?;
        let evidence = replay(recording, Some(&Fixed::liar())).await?.evidence;
        let first: Vec<&str> = evidence.lines().take(lines.len()).collect();
        assert_eq!(first, lines, "{case}: facts lead, in claudef's order");
    }
    Ok(())
}

/// Links quoted in page copy (behind brackets or quotes, or with no scheme) reach Jev as their
/// host only: a reset link's token or a claim reference never does.
#[tokio::test]
async fn tests_that_links_in_page_text_reach_jev_as_hosts_only() -> TestResult {
    let recordings = briefs()?;
    let recording = recordings
        .iter()
        .find(|recording| recording.brief.case == "27-synthetic-links-in-page-text")
        .ok_or("27-synthetic-links-in-page-text is missing")?;
    let evidence = replay(recording, Some(&Fixed::liar())).await?.evidence;
    for host in ["acct.example.gov", "help.example.gov", "www.example.gov"] {
        assert!(evidence.contains(host), "{host} missing from:\n{evidence}");
    }
    for leak in [
        "token=", "abc123", "/reset", "faq", "topic=", "privacy", ".pdf", "/claims", "ref=", "XYZ",
    ] {
        assert!(!evidence.contains(leak), "{leak} leaked into:\n{evidence}");
    }
    Ok(())
}

/// Records Jev's live reading for the briefs named in `VERIFY_REPLAY_LIVE` (comma-separated
/// case names) into the JSON file `VERIFY_REPLAY_LIVE_OUT`, through the crate's own HTTP judge.
/// Needs `NEO_JEV_KEY_FILE` (and `NEO_JEV_MODEL` to pin a model). Each brief is one paid call.
#[tokio::test]
#[ignore = "live: calls Jev with a real key; run by hand to re-measure the thresholds"]
async fn live_jev_readings_for_named_briefs() -> TestResult {
    let lookup = |name: &str| std::env::var(name).ok();
    let config = JudgeConfig::from_lookup(&lookup).ok_or("NEO_JEV_KEY_FILE names no key")?;
    let judge = Keeping {
        inner: HttpJudge::new(config),
        last: Mutex::new(None),
    };
    let wanted = lookup("VERIFY_REPLAY_LIVE").ok_or("VERIFY_REPLAY_LIVE is unset")?;
    let out = lookup("VERIFY_REPLAY_LIVE_OUT").ok_or("VERIFY_REPLAY_LIVE_OUT is unset")?;
    let mut readings = serde_json::Map::new();
    for recording in briefs()? {
        let case = &recording.brief.case;
        if !wanted.split(',').any(|name| name.trim() == case) {
            continue;
        }
        let replay = replay(&recording, Some(&judge)).await?;
        let reply = judge
            .last
            .lock()
            .map_err(|error| error.to_string())?
            .take()
            .ok_or_else(|| format!("{case}: Jev was not asked or did not answer"))?;
        readings.insert(
            case.clone(),
            json!({
                "evidence": replay.evidence,
                "model": reply.model,
                "inputTokens": reply.input_tokens,
                "standing": reply.answers.get("standing").map(|answer| &answer.probabilities),
                "blocked": reply.answers.get("blocked").and_then(|answer| answer.noul),
                "report": serde_json::to_value(replay.report)?,
            }),
        );
    }
    fs::write(out, serde_json::to_string_pretty(&Value::Object(readings))?)?;
    Ok(())
}

/// Passes calls through and keeps the last reply, so the live test can record the raw reading.
struct Keeping<J> {
    inner: J,
    last: Mutex<Option<JudgeReply>>,
}

impl<J: Judge> Judge for Keeping<J> {
    fn ask<'a>(
        &'a self,
        request: &'a JudgeRequest,
        budget: Duration,
    ) -> BoxFuture<'a, Result<JudgeReply, JudgeFault>> {
        Box::pin(async move {
            let reply = self.inner.ask(request, budget).await;
            if let (Ok(reply), Ok(mut last)) = (&reply, self.last.lock()) {
                *last = Some(reply.clone());
            }
            reply
        })
    }
}
