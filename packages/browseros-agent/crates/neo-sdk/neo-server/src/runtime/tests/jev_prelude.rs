//! The Jev helpers' page work in `neo_prelude.js`, in the real realm and door: the re-look loop,
//! an unchanged page not asked again, the stop (an uncaught one is the run's `ok` value, a forged
//! one is not), `optional`, the short console lines, the run's earlier picks, popups, and a later
//! run carrying on from a stop's tab by ref. The Rust verbs are scripted stand-ins and the page a
//! stub object with `ariaSnapshot`, `url`, `title` and `getByRef`, so no browser or Jev is needed.

use std::collections::{HashMap, VecDeque};
use std::future::{self, Future};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use neo_trace::{Delivery, Effect, ErrorOwner, Outcome};
use serde_json::{Value, json};

use super::{BUDGET, Script, returned, run_code, run_script};
use crate::dispatch::{
    Dispatcher, ErrorCode, LeafError, Method, MethodSpec, Observer, OpCtx, OperationError,
    PageFact, PageTarget, RegistryBuilder, RunCtx,
};
use crate::runtime::{LogLine, ScriptEnd, ScriptOutcome};
use crate::test_rig::{TestError, TestRig, on_neo_runtime};

/// A method that answers in order, the last answer again once they run out, and keeps every
/// input it was sent. An answer `{ "$fault": code, "message": text }` fails the op as the Jev
/// built-in fails on that transport fault (`builtins/jev_verbs.rs` `fault`).
struct Scripted {
    name: &'static str,
    answers: Mutex<VecDeque<Value>>,
    inputs: Arc<Mutex<Vec<Value>>>,
}

impl Method for Scripted {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new(self.name, Effect::Read)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input);
        let mut answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        let answer = if answers.len() > 1 {
            answers.pop_front()
        } else {
            answers.front().cloned()
        }
        .unwrap_or(Value::Null);
        future::ready(answered(answer))
    }
}

fn answered(answer: Value) -> Result<Value, LeafError> {
    let message = answer["message"].as_str().unwrap_or_default().to_owned();
    let error = match answer["$fault"].as_str() {
        None => return Ok(answer),
        Some("jev_down") => OperationError::host(ErrorCode::JEV_DOWN, message),
        Some("jev_too_big") => OperationError::script(ErrorCode::JEV_TOO_BIG, message),
        Some(_) => OperationError::host(ErrorCode::JEV_UNAVAILABLE, message),
    };
    Err(LeafError::definite(error))
}

/// What each scripted method was sent, by name.
#[derive(Default)]
struct Sent(HashMap<&'static str, Arc<Mutex<Vec<Value>>>>);

impl Sent {
    fn of(&self, name: &str) -> Vec<Value> {
        self.0.get(name).map_or_else(Vec::new, |inputs| {
            inputs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }
}

/// The real prelude's door over `rig`, with only `methods` registered, each scripted.
fn jev_prelude(
    rig: &mut TestRig,
    methods: Vec<(&'static str, Vec<Value>)>,
) -> Result<Sent, TestError> {
    let mut sent = Sent::default();
    let mut registry = RegistryBuilder::default();
    for (name, answers) in methods {
        let inputs = Arc::<Mutex<Vec<Value>>>::default();
        registry.define(Scripted {
            name,
            answers: Mutex::new(answers.into()),
            inputs: Arc::clone(&inputs),
        })?;
        sent.0.insert(name, inputs);
    }
    rig.dispatcher = Arc::new(Dispatcher::new(
        registry.build(),
        Vec::new(),
        rig.sink.clone(),
        Arc::clone(&rig.link),
        Arc::clone(&rig.tabs),
        Observer::unrecorded(),
    ));
    Ok(sent)
}

const SAW: &str =
    r#"{"elements":4,"candidates":4,"text":false,"batches":1,"long_context":false,"cut":null}"#;

fn saw() -> Value {
    serde_json::from_str(SAW).unwrap_or_default()
}

const COST: &str = r#"{"calls":1,"ms":120,"input_tokens":300}"#;

fn cost() -> Value {
    serde_json::from_str(COST).unwrap_or_default()
}

fn pick(r: &str, label: &str, p: f32) -> Value {
    json!({ "ref": r, "label": label, "p": p })
}

fn find_verdict(outcome: &str, pick_ref: Option<&str>) -> Value {
    json!({
        "outcome": outcome,
        "pick": pick_ref.map(|r| pick(r, "searchbox \"Search\"", 0.97)),
        "lookalikes": [pick("e2", "button \"Go\"", 0.02)],
        "flags": { "reused": null },
        "saw": saw(),
        "obstacle": "none",
        "cost": cost(),
    })
}

fn find_all_verdict(picks: &[Value], unsure: &[Value]) -> Value {
    json!({ "picks": picks, "unsure": unsure, "saw": saw(), "obstacle": "none", "cost": cost() })
}

fn check_verdict(p: f32) -> Value {
    json!({
        "yes": p >= 0.5, "p": p, "unsure": (0.35..=0.65).contains(&p), "saw": saw(),
        "obstacle": "none", "cost": cost(),
    })
}

fn choose_verdict(ranked: [(&str, f32); 2], clear: bool, obstacle: &str) -> Value {
    json!({
        "choice": ranked[0].0, "p": ranked[0].1,
        "ranked": [[ranked[0].0, ranked[0].1], [ranked[1].0, ranked[1].1]],
        "clear": clear, "saw": saw(), "obstacle": obstacle, "cost": cost(),
    })
}

fn fault(code: &str, message: &str) -> Value {
    json!({ "$fault": code, "message": message })
}

/// The stop's `next` line for `tab`, as the prelude writes it (design sketch, "The stop's
/// `next` line").
fn next_line(tab: u64, lookalikes: bool) -> String {
    let act = if lookalikes {
        "Act on a lookalike with page.getByRef(ref), or look with page.ariaSnapshot()."
    } else {
        "Look with page.ariaSnapshot() and act on its refs."
    };
    format!(
        "Not an error; nothing was undone. Carry on by hand on this tab: const page = context.pages().find(p => p.tabId === {tab}). {act}"
    )
}

/// The stub page: its snapshot is `first` for the first `switch_at - 1` looks, then `then`;
/// `emit('popup', tab)` stands in for the events pump handing over a popup.
const FAKE_PAGE: &str = r"
    let looks = 0
    const stub = (first, then, switchAt, url = 'https://shop.example/', tabId = 7) => {
      const handlers = {}
      return {
        ariaSnapshot: async (options) => { looks++; return looks < switchAt ? first : then },
        url: () => url,
        title: async () => 'Shop',
        getByRef: (ref) => ({ ref, url }),
        isClosed: () => false,
        tabId,
        on(event, fn) { (handlers[event] ??= []).push(fn); return this },
        emit(event, value) { for (const fn of handlers[event] ?? []) fn(value) },
      }
    }
";

/// The run's end, and its console lines apart from it.
fn split(mut end: ScriptEnd) -> (ScriptEnd, Vec<String>) {
    let (lines, _) = mem::take(&mut end.logs).into_parts();
    (
        end,
        lines
            .iter()
            .map(|line: &LogLine| line.text.clone())
            .collect(),
    )
}

/// The `Stop` record a run returned, with its `at` checked to name the script line holding
/// `marker` and then taken out, since its column is the engine's.
fn stopped(end: ScriptEnd, code: &str, marker: &str) -> Result<Value, TestError> {
    let mut value = returned(end)?;
    let mut stop = value["stopped"].take();
    let line = code
        .lines()
        .position(|line| line.contains(marker))
        .ok_or("the marker is in the script")?
        + 1;
    let at = stop["at"].as_str().unwrap_or_default().to_owned();
    assert!(
        at.starts_with(&format!("run:{line}:")),
        "`at` is the agent's line of the call that stopped: {at} (line {line})"
    );
    if let Some(fields) = stop.as_object_mut() {
        fields.remove("at");
    }
    Ok(stop)
}

/// The run's ops named `name`, as the trace recorded them.
async fn ops_named(
    rig: &TestRig,
    run: &Arc<RunCtx>,
    name: &str,
) -> Result<Vec<neo_trace::OperationRow>, TestError> {
    rig.sink.flush().await?;
    let detail = rig.store.run(run.id()).await?.ok_or("recorded run")?;
    Ok(detail
        .operations
        .into_iter()
        .filter(|op| op.name == name)
        .collect())
}

#[test]
fn find_looks_again_until_the_element_shows_and_keeps_the_runs_picks() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let sent = jev_prelude(
            &mut rig,
            vec![(
                "neo.find",
                vec![
                    find_verdict("not_found", None),
                    find_verdict("found", Some("e1")),
                    find_verdict("found", Some("e2")),
                ],
            )],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const fake = stub('- generic [ref=e0]', '- searchbox \"Search\" [ref=e1]', 3)
            const box = await neo.find(fake, 'the search box', {{ timeout: 3000 }})
            const go = await neo.find(fake, 'the search button')
            return {{ box: box.ref, go: go.ref, looks }}"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(
            returned(end)?,
            json!({ "box": "e1", "go": "e2", "looks": 4 })
        );
        let find = sent.of("neo.find");
        assert_eq!(
            find.len(),
            3,
            "look 2 saw the same page as look 1, so Jev was not asked again: {find:?}"
        );
        assert_eq!(find[0]["url"], "https://shop.example/");
        assert_eq!(find[0]["title"], "Shop");
        assert_eq!(find[0]["ask"], "the search box");
        assert_eq!(find[0]["picked"], json!([]));
        assert_eq!(
            find[2]["picked"],
            json!([{ "ref": "e1", "ask": "the search box" }]),
            "the run's earlier picks go with every later find"
        );
        assert_eq!(
            lines,
            [
                "neo.find \"the search box\" -> e1 searchbox \"Search\" p0.97",
                "neo.find \"the search button\" -> e2 searchbox \"Search\" p0.97",
            ],
            "one short line per call: no timings, counts or runner-up"
        );
        Ok(())
    })
}

#[test]
fn an_uncaught_stop_ends_the_run_ok_with_the_hand_back() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let sent = jev_prelude(
            &mut rig,
            vec![(
                "neo.find",
                vec![
                    find_verdict("found", Some("e1")),
                    find_verdict("not_found", None),
                ],
            )],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const fake = stub('- searchbox \"Search\" [ref=e1]', '', 1000)
            await neo.find(fake, 'the search box')
            await neo.find(fake, 'the cart button', {{ timeout: 700 }})
            return 'unreachable'"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        let stop = stopped(end, &code, "the cart button")?;
        assert_eq!(
            stop,
            json!({
                "call": "neo.find", "ask": "the cart button", "why": "not_found", "tab": 7,
                "url": "https://shop.example/",
                "lookalikes": [{ "ref": "e2", "is": "button \"Go\"", "p": 0.02 }],
                "next": next_line(7, true),
            }),
            "a stop is the run's value, not its error"
        );
        assert_eq!(
            lines,
            [
                "neo.find \"the search box\" -> e1 searchbox \"Search\" p0.97",
                "neo.find \"the cart button\" -> stopped: not_found · e2 0.02",
            ],
            "the console line of every finished step, then the stop's"
        );
        assert_eq!(
            sent.of("neo.find").len(),
            2,
            "the unchanged page was looked at again but not re-asked"
        );
        let finds = ops_named(&rig, &run, "neo.find").await?;
        let refused = finds.last().ok_or("the stop's row")?;
        assert_eq!(
            (
                refused.outcome,
                refused.delivery,
                refused.error.as_ref().map(|e| (e.owner, e.code.as_str()))
            ),
            (
                Some(Outcome::Error),
                Some(Delivery::NotSent),
                Some((ErrorOwner::Script, "stopped"))
            ),
            "the trace keeps the stop as a refused op that sent nothing"
        );
        let recorded = refused
            .args
            .as_ref()
            .map(|args| args["stop"].clone())
            .unwrap_or_default();
        assert_eq!(
            (&recorded["why"], &recorded["next"]),
            (&json!("not_found"), &json!(next_line(7, true))),
            "the op's row keeps the hand-back the script got"
        );
        Ok(())
    })
}

#[test]
fn a_caught_stop_is_a_neo_error_and_optional_returns_null() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        jev_prelude(
            &mut rig,
            vec![(
                "neo.find",
                vec![
                    find_verdict("ambiguous", None),
                    find_verdict("not_found", None),
                ],
            )],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const fake = stub('- link \"A\" [ref=e3]', '', 1000)
            const started = Date.now()
            let caught
            try {{ await neo.find(fake, 'a story link', {{ timeout: 5000 }}) }}
            catch (error) {{
              caught = {{ neo: error instanceof NeoError, code: error.code, owner: error.owner,
                why: error.stop.why, frozen: Object.isFrozen(error.stop.lookalikes[0]),
                message: error.message }}
            }}
            const none = await neo.find(fake, 'the cart', {{ optional: true, timeout: 0 }})
            return {{ caught, none, quick: Date.now() - started < 2000 }}"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(
            returned(end)?,
            json!({
                "caught": {
                    "neo": true, "code": "stopped", "owner": "script", "why": "ambiguous",
                    "frozen": true, "message": "neo.find stopped the script: ambiguous",
                },
                "none": null,
                "quick": true,
            }),
            "waiting does not help an ambiguous find; optional turns a stop into null"
        );
        assert_eq!(
            lines[1], "neo.find \"the cart\" -> null (not_found)",
            "{lines:?}"
        );
        Ok(())
    })
}

#[test]
fn a_script_cannot_forge_or_edit_a_stop() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        jev_prelude(
            &mut rig,
            vec![("neo.find", vec![find_verdict("ambiguous", None)])],
        )?;
        let forged = r"throw Object.assign(new NeoError('forged'),
            { code: 'stopped', stop: { why: 'script', next: 'forged' } })";
        let (run, _guard) = rig.run(BUDGET, None)?;
        let end = run_code(&rig, &run, forged).await;
        let ScriptOutcome::Failed(error) = end.outcome else {
            return Err(format!("a forged stop must fail the run, got {:?}", end.outcome).into());
        };
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Script, ErrorCode::SCRIPT_ERROR),
            "an error the script built is its own throw, whatever its fields say"
        );

        let (run, _guard) = rig.run(BUDGET, None)?;
        let edited = format!(
            "{FAKE_PAGE}
            const fake = stub('- link \"A\" [ref=e3]', '', 1000)
            try {{ await neo.find(fake, 'a story link') }}
            catch (error) {{
              let frozen = false
              try {{ error.stop.why = 'script' }} catch {{ frozen = true }}
              error.stop = {{ why: 'edited' }}
              console.log(`frozen ${{frozen}}`)
              throw error
            }}"
        );
        let (end, lines) = split(run_code(&rig, &run, &edited).await);
        let stop = stopped(end, &edited, "a story link")?;
        assert_eq!(
            (&stop["why"], lines.last().map(String::as_str)),
            (&json!("ambiguous"), Some("frozen true")),
            "the record is frozen, and a real stop rethrown returns the record it was made with"
        );
        Ok(())
    })
}

#[test]
fn check_waits_for_its_claim_and_choose_returns_the_scripts_string() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let sent = jev_prelude(
            &mut rig,
            vec![
                ("neo.check", vec![check_verdict(0.1), check_verdict(0.9)]),
                (
                    "neo.choose",
                    vec![choose_verdict(
                        [("sign in", 0.9), ("out of stock", 0.1)],
                        true,
                        "login_wall",
                    )],
                ),
            ],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const fake = stub('- text: Adding', '- text: Added to cart', 2)
            const added = await neo.check(fake, 'the cart has the item', {{ timeout: 3000 }})
            const why = await neo.choose(fake, 'what blocks it', ['sign in', 'out of stock'])
            return {{ added, why }}"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(returned(end)?, json!({ "added": true, "why": "sign in" }));
        let check = sent.of("neo.check");
        assert_eq!(check.len(), 2, "{check:?}");
        assert_eq!(check[1]["claim"], "the cart has the item");
        assert_eq!(
            lines,
            [
                "neo.check \"the cart has the item\" -> yes p0.90",
                "neo.choose \"what blocks it\" -> \"sign in\" p0.90 · obstacle login_wall",
            ]
        );
        Ok(())
    })
}

#[test]
fn unsure_answers_stop_find_all_check_and_choose() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let unsure = vec![
            pick("e4", "link \"Ad\"", 0.52),
            pick("e5", "link \"Promo\"", 0.41),
            pick("e6", "link \"New\"", 0.6),
            pick("e7", "link \"Old\"", 0.38),
        ];
        let sent = jev_prelude(
            &mut rig,
            vec![
                (
                    "neo.findAll",
                    vec![find_all_verdict(&[pick("e1", "link \"A\"", 0.99)], &unsure)],
                ),
                ("neo.check", vec![check_verdict(0.52)]),
                (
                    "neo.choose",
                    vec![choose_verdict([("a", 0.45), ("b", 0.4)], false, "none")],
                ),
            ],
        )?;
        let page = format!("{FAKE_PAGE}\nconst fake = stub('- link \"A\" [ref=e1]', '', 1000)\n");

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{page}return await neo.findAll(fake, 'each story link')");
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(
            stopped(end, &code, "each story link")?,
            json!({
                "call": "neo.findAll", "ask": "each story link", "why": "unsure", "tab": 7,
                "url": "https://shop.example/",
                "lookalikes": [
                    { "ref": "e4", "is": "link \"Ad\"", "p": 0.52 },
                    { "ref": "e5", "is": "link \"Promo\"", "p": 0.41 },
                    { "ref": "e6", "is": "link \"New\"", "p": 0.6 },
                ],
                "next": next_line(7, true),
            }),
            "the elements Jev cannot call are the lookalikes, three at most"
        );
        assert_eq!(
            lines,
            ["neo.findAll \"each story link\" -> stopped: unsure · e4 0.52 · e5 0.41 · e6 0.60"]
        );

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{page}return await neo.check(fake, 'the order went through', {{ timeout: 700 }})"
        );
        assert_eq!(
            stopped(
                run_code(&rig, &run, &code).await,
                &code,
                "the order went through"
            )?,
            json!({
                "call": "neo.check", "ask": "the order went through", "why": "unsure", "tab": 7,
                "url": "https://shop.example/", "lookalikes": [], "detail": "p 0.52",
                "next": next_line(7, false),
            }),
            "unsure at the last look stops; without lookalikes, the next line says to look"
        );
        assert_eq!(
            sent.of("neo.check").len(),
            1,
            "a timed check kept looking for a clear answer, but the page never changed"
        );

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{page}return await neo.choose(fake, 'which tab is open', ['a', 'b'])");
        assert_eq!(
            stopped(
                run_code(&rig, &run, &code).await,
                &code,
                "which tab is open"
            )?,
            json!({
                "call": "neo.choose", "ask": "which tab is open", "why": "unsure", "tab": 7,
                "url": "https://shop.example/", "lookalikes": [],
                "detail": "\"a\" 0.45 · \"b\" 0.40", "next": next_line(7, false),
            }),
            "no option clearly leads: the top two go in the detail"
        );
        Ok(())
    })
}

#[test]
fn jev_down_and_too_big_stop_with_the_fault_as_detail() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        jev_prelude(
            &mut rig,
            vec![
                (
                    "neo.findAll",
                    vec![fault("jev_down", "Jev is down: could not connect")],
                ),
                (
                    "neo.find",
                    vec![fault("jev_too_big", "the page is over Jev's limit")],
                ),
                ("neo.check", vec![fault("jev_unavailable", "no Jev key")]),
            ],
        )?;
        let page = format!("{FAKE_PAGE}\nconst fake = stub('- link \"A\" [ref=e3]', '', 1000)\n");

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{page}return await neo.findAll(fake, 'each story link')");
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        let stop = stopped(end, &code, "each story link")?;
        assert_eq!(
            (&stop["why"], &stop["detail"], &stop["lookalikes"]),
            (
                &json!("jev_down"),
                &json!("Jev is down: could not connect"),
                &json!([])
            )
        );
        assert_eq!(
            lines,
            ["neo.findAll \"each story link\" -> stopped: jev_down"]
        );
        let rows = ops_named(&rig, &run, "neo.findAll").await?;
        let codes: Vec<Option<&str>> = rows
            .iter()
            .map(|row| row.error.as_ref().map(|error| error.code.as_str()))
            .collect();
        assert_eq!(
            codes,
            [Some("jev_down"), Some("stopped")],
            "the built-in's op keeps its fault; the stop is a row of its own"
        );

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{page}return await neo.find(fake, 'the cart', {{ optional: true }})");
        let stop = stopped(run_code(&rig, &run, &code).await, &code, "the cart")?;
        assert_eq!(
            (&stop["why"], &stop["detail"]),
            (&json!("too_big"), &json!("the page is over Jev's limit")),
            "a fault stops even an optional find: null would read as 'not there'"
        );

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!("{page}return await neo.check(fake, 'it shows')");
        let end = run_code(&rig, &run, &code).await;
        let ScriptOutcome::OperationFailed(failure) = end.outcome else {
            return Err(format!("no key is an error, got {:?}", end.outcome).into());
        };
        assert_eq!(
            failure.error.code(),
            ErrorCode::JEV_UNAVAILABLE,
            "only Jev down and too big become stops"
        );
        Ok(())
    })
}

#[test]
fn a_pick_reused_under_another_ask_stops_with_the_earlier_ask() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let mut reused = find_verdict("ambiguous", None);
        reused["lookalikes"] = json!([pick("e1", "combobox \"State\"", 0.9)]);
        reused["flags"]["reused"] = json!("the state dropdown");
        jev_prelude(
            &mut rig,
            vec![("neo.find", vec![find_verdict("found", Some("e1")), reused])],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const fake = stub('- combobox \"State\" [ref=e1]', '', 1000)
            await neo.find(fake, 'the state dropdown')
            await neo.find(fake, 'the city dropdown')"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        let stop = stopped(end, &code, "the city dropdown")?;
        assert_eq!(
            (&stop["why"], &stop["detail"]),
            (
                &json!("ambiguous"),
                &json!("e1 is already picked for \"the state dropdown\"")
            )
        );
        assert_eq!(
            lines[1],
            "neo.find \"the city dropdown\" -> stopped: ambiguous · e1 0.90"
        );
        Ok(())
    })
}

#[test]
fn every_verb_follows_a_popup_the_page_opened_after_the_first_look() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let sent = jev_prelude(
            &mut rig,
            vec![
                (
                    "neo.find",
                    vec![
                        find_verdict("found", Some("e1")),
                        find_verdict("found", Some("e9")),
                    ],
                ),
                ("neo.check", vec![check_verdict(0.95)]),
            ],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const results = stub('- link \"Sensodyne\" [ref=e1]', '', 1000)
            const product = stub('- button \"Add to cart\" [ref=e9]', '', 1000, 'https://shop.example/dp/1', 8)
            await (await neo.find(results, 'the product link'))
            results.emit('popup', product)
            const onProduct = await neo.check(results, 'this is the product page')
            const add = await neo.find(results, 'the Add to cart button')
            return {{ onProduct, add }}"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(
            returned(end)?,
            json!({ "onProduct": true, "add": { "ref": "e9", "url": "https://shop.example/dp/1" } }),
            "the Locator belongs to the popup"
        );
        assert_eq!(
            sent.of("neo.check")[0]["url"],
            "https://shop.example/dp/1",
            "check read the popup"
        );
        assert_eq!(
            sent.of("neo.find")[1]["url"],
            "https://shop.example/dp/1",
            "find read the popup"
        );
        assert_eq!(
            lines[1],
            "neo.check follows the new tab https://shop.example/dp/1 (opened from https://shop.example/)",
            "one console line per follow, at the verb that first looks: {lines:?}"
        );
        assert_eq!(lines.len(), 4, "a follow is logged once: {lines:?}");
        Ok(())
    })
}

#[test]
fn neo_stop_needs_no_await_and_hands_back_the_newest_tab() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let sent = jev_prelude(
            &mut rig,
            vec![("neo.find", vec![find_verdict("found", Some("e1"))])],
        )?;
        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            const results = stub('- link \"Sensodyne\" [ref=e1]', '', 1000)
            const product = stub('- text: Sold out', '', 1000, 'https://shop.example/dp/1', 8)
            await (await neo.find(results, 'the product link'))
            results.emit('popup', product)
            neo.stop(results, 'the product page has no Add to cart button')
            return 'unreachable'"
        );
        let (end, lines) = split(run_code(&rig, &run, &code).await);
        assert_eq!(
            stopped(end, &code, "no Add to cart button")?,
            json!({
                "call": "neo.stop", "ask": "the product page has no Add to cart button",
                "why": "script", "tab": 8, "url": "https://shop.example/dp/1", "lookalikes": [],
                "next": next_line(8, false),
            }),
            "the script's own stop hands back the tab a helper would have looked at"
        );
        assert_eq!(
            lines[1..],
            [
                "neo.stop follows the new tab https://shop.example/dp/1 (opened from https://shop.example/)",
                "neo.stop \"the product page has no Add to cart button\" -> stopped: script",
            ]
        );
        assert_eq!(sent.of("neo.find").len(), 1, "neo.stop asks Jev nothing");

        let (run, _guard) = rig.run(BUDGET, None)?;
        let code = format!(
            "{FAKE_PAGE}
            neo.stop(stub('', '', 1000), 'not the page I expected')"
        );
        let stop = stopped(
            run_code(&rig, &run, &code).await,
            &code,
            "not the page I expected",
        )?;
        assert_eq!(
            (&stop["tab"], &stop["url"]),
            (&json!(7), &json!("https://shop.example/")),
            "a page no helper looked at is its own tab"
        );
        Ok(())
    })
}

/// Design §1 "Refs": a later run carries on from a stop by hand, on a real Page. Run 1 stops on
/// the owner's tab; run 2 runs the stop's `next` line as written, then acts on a lookalike's ref.
/// The ops a browser would serve (`page.title`, `locator.click`) are scripted; the Page objects,
/// `context.pages()`, `getByRef` and the door are real.
#[test]
fn a_later_run_carries_on_from_the_stop_with_its_next_line() -> Result<(), TestError> {
    on_neo_runtime(async {
        let mut rig = TestRig::new().await?;
        let tab = rig.owner_tab("https://flights.example/").await?;
        let mut ambiguous = find_verdict("ambiguous", None);
        ambiguous["lookalikes"] = json!([
            pick("e541", "combobox \"San Francisco\"", 0.7),
            pick("e542", "combobox \"San Francisco\"", 0.22),
        ]);
        let sent = jev_prelude(
            &mut rig,
            vec![
                ("neo.find", vec![ambiguous]),
                ("page.title", vec![json!("Flights")]),
                ("locator.click", vec![Value::Null]),
            ],
        )?;
        let fact = PageFact {
            task_id: None,
            viewport_size: None,
            workers: None,
            tab: tab.tab,
            url: Some("https://flights.example/".to_owned()),
            title: Some("Flights".to_owned()),
            closed: false,
        };

        let (run, _guard) = rig.run(BUDGET, None)?;
        let first = Script {
            code: r#"
                page.ariaSnapshot = async () => '- combobox "San Francisco" [ref=e541]'
                await (await neo.find(page, 'the "Where from?" field')).fill('San Francisco')"#
                .to_owned(),
            page: Some(tab.clone()),
            pages: vec![fact.clone()],
            observed: None,
        };
        let value = returned(run_script(&run, &rig.dispatcher, first).await)?;
        let stop = &value["stopped"];
        assert_eq!(
            (&stop["tab"], &stop["url"], &stop["lookalikes"][0]["ref"]),
            (
                &json!(tab.tab.0),
                &json!("https://flights.example/"),
                &json!("e541")
            ),
            "the stop names the real Page's tab: {value}"
        );
        let next = stop["next"].as_str().unwrap_or_default();
        let carry_on = next
            .split_once("on this tab: ")
            .and_then(|(_, rest)| rest.split_once(". Act on"))
            .map(|(line, _)| line)
            .ok_or("the next line holds the code to find the tab")?;

        let (run, _guard) = rig.run(BUDGET, None)?;
        let second = Script {
            code: format!("{carry_on}\nawait page.getByRef('e541').click()\nreturn page.tabId"),
            page: None,
            pages: vec![fact],
            observed: None,
        };
        assert_eq!(
            returned(run_script(&run, &rig.dispatcher, second).await)?,
            json!(tab.tab.0),
            "the next line finds the stop's tab in a later run's context.pages()"
        );
        let clicks = sent.of("locator.click");
        let target = clicks
            .first()
            .map(|click| click["target"].clone())
            .unwrap_or_default();
        assert_eq!(
            target.pointer("/page/tab"),
            Some(&json!(tab.tab.0)),
            "the click goes to the stop's tab: {target}"
        );
        assert!(
            target.to_string().contains("e541"),
            "the click addresses the lookalike's ref: {target}"
        );
        Ok(())
    })
}
