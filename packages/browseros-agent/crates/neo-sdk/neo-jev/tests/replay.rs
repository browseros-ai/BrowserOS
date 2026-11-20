//! Replays the labelled eval cases (`NEO_JEV_EVAL_DIR`) through view and gate, offline from the
//! recorded answers and live (`NEO_JEV_LIVE=1`) against real Jev; skipped when the folder is unset.
//!
//! Every stop the prelude makes from a verdict is graded as one: `find` not found or ambiguous,
//! `check` unsure, `choose` not clear, `find_all` with any unsure element. The report also gives
//! the scores that came nearest the stop lines, so a threshold change can be judged by its margin.
//!
//! The eval JSON is third-party page text and is never committed. `NEO_JEV_REPLAY_OUT` names a
//! folder for `replay-<mode>.md` (pass rates, failures) and `replay-<mode>.tsv` (every case).
#![expect(clippy::print_stderr, reason = "a skipped replay says why on stderr")]

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use neo_jev::gate::{self, Answer, FindGate};
use neo_jev::view::View;
use neo_jev::{EarlierPick, FindOpts, FindOutcome, Jev, JevConfig, PageText};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

/// One eval case, the fields the replay reads.
struct Case {
    file: String,
    site: String,
    run: i64,
    step: i64,
    kind: String,
    ask: String,
    page: PageText,
    options: Vec<String>,
    jev: Value,
    final_answer: Option<Value>,
    correct: Value,
}

/// How a verdict compares with `correct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grade {
    /// The verdict is what `correct` says.
    Pass,
    /// A stop where an answer existed, or an ambiguous stop where none did: the run would hand
    /// back to the agent, never act on a wrong element. For `find_all`, also a list that only
    /// misses elements (no stop, no wrong one).
    Safe,
    /// A wrong element found, or something found where nothing was right: the run acts wrongly.
    Wrong,
}

/// The scores one case holds that a stop line reads, for the report's margins.
#[derive(Debug, Clone, Default)]
enum Edge {
    /// `find`: its lines (`ACCEPT`, `RIVAL`) did not change in v1.
    #[default]
    Find,
    /// `check`'s probability.
    Check(f32),
    /// `choose`'s top two probabilities.
    Choose { top: f32, next: f32 },
    /// Every `find_all` group score the verdict carries.
    FindAll(Vec<f32>),
}

struct Outcome {
    grade: Grade,
    verdict: String,
    /// A v1 stop fired: `check` unsure, `choose` not clear, `find_all` unsure.
    new_stop: bool,
    edge: Edge,
}

fn load(dir: &Path) -> Result<Vec<Case>, Box<dyn Error>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for path in files {
        let d: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let parse_maybe_string = |v: &Value| -> Value {
            match v {
                Value::String(s) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
                other => other.clone(),
            }
        };
        let options = match parse_maybe_string(&d["options"]) {
            Value::Array(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        cases.push(Case {
            file: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            site: text(&d["site"]),
            run: d["run"].as_i64().unwrap_or_default(),
            step: d["step"].as_i64().unwrap_or_default(),
            kind: text(&d["kind"]),
            ask: text(&d["ask"]),
            page: PageText {
                url: text(&d["state"]["url"]),
                title: text(&d["state"]["title"]),
                snapshot: text(&d["state"]["page"]),
            },
            options,
            jev: parse_maybe_string(&d["jev"]),
            final_answer: d.get("final").filter(|v| !v.is_null()).cloned(),
            correct: d["correct"].clone(),
        });
    }
    // Runs replay in step order, so `picked` (the `reused` flag) sees earlier decisions only.
    cases.sort_by(|a, b| (&a.site, a.run, a.step, &a.file).cmp(&(&b.site, b.run, b.step, &b.file)));
    Ok(cases)
}

#[expect(clippy::cast_possible_truncation, reason = "probabilities fit f32")]
fn pairs(v: &Value) -> Vec<(String, f32)> {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|pair| {
                    Some((
                        pair.get(0)?.as_str()?.to_owned(),
                        pair.get(1)?.as_f64()? as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn correct_refs(case: &Case) -> Vec<String> {
    case.correct
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The recorded answer as the gate's input. The harness kept the top five refs per decision and
/// `none` per batch (older runs: no `none`); when a re-ask (`final`) happened, it decided.
#[expect(clippy::cast_possible_truncation, reason = "probabilities fit f32")]
fn recorded_answer(case: &Case) -> Answer {
    let rest = |probs: &[(String, f32)]| (1.0 - probs.iter().map(|(_, p)| p).sum::<f32>()).max(0.0);
    if let Some(final_answer) = &case.final_answer {
        let probs = pairs(&final_answer["top"]);
        let none = rest(&probs);
        return Answer { probs, none };
    }
    let probs = pairs(&case.jev["top"]);
    let none = case.jev["none"]
        .as_array()
        .and_then(|a| a.iter().filter_map(Value::as_f64).reduce(f64::min))
        .map_or_else(|| rest(&probs), |n| n as f32);
    Answer { probs, none }
}

fn grade_find(
    case: &Case,
    outcome: FindOutcome,
    pick: Option<&str>,
    flags: &str,
    detail: &str,
) -> Outcome {
    let correct = correct_refs(case);
    let grade = match (outcome, pick) {
        (FindOutcome::Found, Some(r)) if correct.iter().any(|c| c == r) => Grade::Pass,
        (FindOutcome::Found, _) => Grade::Wrong,
        (FindOutcome::NotFound, _) if correct.is_empty() => Grade::Pass,
        (FindOutcome::NotFound | FindOutcome::Ambiguous, _) => Grade::Safe,
    };
    let verdict = match pick {
        Some(r) => format!("{outcome:?} {r}{flags} · {detail}"),
        None => format!("{outcome:?}{flags} · {detail}"),
    };
    Outcome {
        grade,
        verdict,
        new_stop: false,
        edge: Edge::Find,
    }
}

fn flags_text(reused: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(ask) = reused {
        let _ = write!(out, " [reused from {ask:?}]");
    }
    out
}

fn picks_text(picks: &[(String, f32)]) -> String {
    picks
        .iter()
        .take(3)
        .map(|(r, p)| format!("{r} {p:.2}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `find_all`: `picks` and `unsure` are refs; `scores` every group score the verdict carries.
fn grade_set(case: &Case, picks: &[String], unsure: &[String], scores: Vec<f32>) -> Outcome {
    let correct = correct_refs(case);
    let extra: Vec<&String> = picks.iter().filter(|r| !correct.contains(r)).collect();
    let missing: Vec<&String> = correct.iter().filter(|r| !picks.contains(r)).collect();
    let new_stop = !unsure.is_empty();
    let grade = if new_stop {
        Grade::Safe
    } else if extra.is_empty() && missing.is_empty() && picks == correct.as_slice() {
        Grade::Pass
    } else if extra.is_empty() {
        Grade::Safe
    } else {
        Grade::Wrong
    };
    let stop = if new_stop {
        format!("stopped: unsure {unsure:?} · ")
    } else {
        String::new()
    };
    Outcome {
        grade,
        verdict: format!(
            "{stop}{} picks, {} extra {:?}, {} missing {:?}",
            picks.len(),
            extra.len(),
            extra,
            missing.len(),
            missing
        ),
        new_stop,
        edge: Edge::FindAll(scores),
    }
}

fn grade_check(case: &Case, yes: bool, p: f32, unsure: bool) -> Outcome {
    let right = case.correct.as_bool() == Some(yes);
    let grade = match (unsure, right) {
        (true, _) => Grade::Safe,
        (false, true) => Grade::Pass,
        (false, false) => Grade::Wrong,
    };
    Outcome {
        grade,
        verdict: format!(
            "{}yes={yes} p={p:.2}",
            if unsure { "stopped: unsure · " } else { "" }
        ),
        new_stop: unsure,
        edge: Edge::Check(p),
    }
}

fn grade_choose(case: &Case, ranked: &[(String, f32)], clear: bool) -> Outcome {
    let choice = ranked.first().map(|(c, _)| c.as_str()).unwrap_or_default();
    let right = case.correct.as_str() == Some(choice);
    let grade = match (clear, right) {
        (false, _) => Grade::Safe,
        (true, true) => Grade::Pass,
        (true, false) => Grade::Wrong,
    };
    let p = |i: usize| ranked.get(i).map_or(0.0, |(_, p)| *p);
    Outcome {
        grade,
        verdict: format!(
            "{}{choice:?} · {}",
            if clear { "" } else { "stopped: unsure · " },
            picks_text(ranked)
        ),
        new_stop: !clear,
        edge: Edge::Choose {
            top: p(0),
            next: p(1),
        },
    }
}

/// Picks of earlier finds in the same site run, for the `reused` flag.
#[derive(Default)]
struct Picked(HashMap<(String, i64), Vec<EarlierPick>>);

impl Picked {
    fn of(&self, case: &Case) -> Vec<EarlierPick> {
        self.0
            .get(&(case.site.clone(), case.run))
            .cloned()
            .unwrap_or_default()
    }

    fn add(&mut self, case: &Case, r: &str) {
        self.0
            .entry((case.site.clone(), case.run))
            .or_default()
            .push(EarlierPick {
                r#ref: r.to_owned(),
                ask: case.ask.clone(),
            });
    }
}

#[expect(clippy::cast_possible_truncation, reason = "probabilities fit f32")]
fn offline(case: &Case, picked: &mut Picked) -> Result<Outcome, String> {
    match case.kind.as_str() {
        "find" => {
            let view = View::new(&case.page.snapshot);
            let answer = recorded_answer(case);
            let FindGate::Decided(d) = gate::find(&view, &[answer], &case.ask, &picked.of(case))
            else {
                return Err("one batch asked for a re-ask".into());
            };
            let pick = d.pick.as_ref().map(|p| p.r#ref.as_str());
            if let Some(r) = pick {
                picked.add(case, r);
            }
            let top: Vec<(String, f32)> = d
                .pick
                .iter()
                .chain(&d.lookalikes)
                .map(|p| (p.r#ref.clone(), p.p))
                .collect();
            Ok(grade_find(
                case,
                d.outcome,
                pick,
                &flags_text(d.flags.reused.as_deref()),
                &picks_text(&top),
            ))
        }
        "check" => {
            let p = case.jev["p"].as_f64().ok_or("no recorded p")? as f32;
            let d = gate::check(p);
            Ok(grade_check(case, d.yes, p, d.unsure))
        }
        "choose" => {
            let recorded: HashMap<String, f32> = pairs(&case.jev["top"]).into_iter().collect();
            let probs: Vec<f32> = case
                .options
                .iter()
                .map(|o| recorded.get(o).copied().unwrap_or(0.0))
                .collect();
            let d = gate::choose(&case.options, &probs);
            Ok(grade_choose(case, &d.ranked, d.clear))
        }
        "findAll" => {
            let view = View::new(&case.page.snapshot);
            // One Noul per offered ref was recorded; a group scores its best member.
            let mut best: BTreeMap<usize, f32> = BTreeMap::new();
            for (r, p) in pairs(&case.jev["all"]) {
                if let Some(g) = view.group_of(&r) {
                    let slot = best.entry(g).or_default();
                    *slot = slot.max(p);
                }
            }
            let scores: Vec<(usize, f32)> = best.into_iter().collect();
            let d = gate::find_all(&view, &scores);
            let refs = |picks: &[neo_jev::Pick]| -> Vec<String> {
                picks.iter().map(|p| p.r#ref.clone()).collect()
            };
            let all: Vec<f32> = scores.iter().map(|&(_, p)| p).collect();
            Ok(grade_set(case, &refs(&d.picks), &refs(&d.unsure), all))
        }
        other => Err(format!("unknown kind {other}")),
    }
}

async fn live(jev: &Jev, case: &Case, picked: &mut Picked) -> Result<(Outcome, String), String> {
    let fault = |e: neo_jev::JevFault| format!("fault: {e}");
    match case.kind.as_str() {
        "find" => {
            let opts = FindOpts {
                role: None,
                picked: picked.of(case),
            };
            let v = jev
                .find(&case.page, &case.ask, &opts)
                .await
                .map_err(fault)?;
            let pick = v.pick.as_ref().map(|p| p.r#ref.as_str());
            if let Some(r) = pick {
                picked.add(case, r);
            }
            let top: Vec<(String, f32)> = v
                .pick
                .iter()
                .chain(&v.lookalikes)
                .map(|p| (p.r#ref.clone(), p.p))
                .collect();
            let cost = format!(
                "{} calls {} ms {} tok b{} cands {}{} obstacle {:?}",
                v.cost.calls,
                v.cost.ms,
                v.cost.input_tokens,
                v.saw.batches,
                v.saw.candidates,
                if v.saw.long_context { " long" } else { "" },
                v.obstacle
            );
            Ok((
                grade_find(
                    case,
                    v.outcome,
                    pick,
                    &flags_text(v.flags.reused.as_deref()),
                    &picks_text(&top),
                ),
                cost,
            ))
        }
        "check" => {
            let v = jev.check(&case.page, &case.ask).await.map_err(fault)?;
            let cost = format!(
                "{} calls {} ms {} tok{} obstacle {:?}",
                v.cost.calls,
                v.cost.ms,
                v.cost.input_tokens,
                if v.saw.long_context { " long" } else { "" },
                v.obstacle
            );
            Ok((grade_check(case, v.yes, v.p, v.unsure), cost))
        }
        "choose" => {
            let v = jev
                .choose(&case.page, &case.ask, &case.options)
                .await
                .map_err(fault)?;
            let cost = format!(
                "{} calls {} ms {} tok obstacle {:?}",
                v.cost.calls, v.cost.ms, v.cost.input_tokens, v.obstacle
            );
            Ok((grade_choose(case, &v.ranked, v.clear), cost))
        }
        "findAll" => {
            let v = jev
                .find_all(&case.page, &case.ask, &FindOpts::default())
                .await
                .map_err(fault)?;
            let picks: Vec<String> = v.picks.iter().map(|p| p.r#ref.clone()).collect();
            let unsure: Vec<String> = v.unsure.iter().map(|p| p.r#ref.clone()).collect();
            // A live verdict carries picks and unsure groups only, not the clear noes.
            let scores: Vec<f32> = v.picks.iter().chain(&v.unsure).map(|p| p.p).collect();
            let cost = format!(
                "{} calls {} ms {} tok b{} cands {} obstacle {:?}",
                v.cost.calls,
                v.cost.ms,
                v.cost.input_tokens,
                v.saw.batches,
                v.saw.candidates,
                v.obstacle
            );
            Ok((grade_set(case, &picks, &unsure, scores), cost))
        }
        other => Err(format!("unknown kind {other}")),
    }
}

/// One graded case: file, site, kind, the grade (or why it was excluded), and what it cost.
type Row = (String, String, String, Result<Outcome, String>, String);

/// Every score a v1 stop line reads, gathered for `Report::margins`.
#[derive(Default)]
struct Margins {
    check: Vec<f32>,
    choose_top: Vec<f32>,
    choose_next: Vec<f32>,
    find_all: Vec<f32>,
}

/// Collects grades and writes the report.
#[derive(Default)]
struct Report {
    rows: Vec<Row>,
}

impl Report {
    fn add(&mut self, case: &Case, result: Result<Outcome, String>, cost: String) {
        self.rows.push((
            case.file.clone(),
            case.site.clone(),
            case.kind.clone(),
            result,
            cost,
        ));
    }

    fn counts(&self) -> (usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0);
        for (_, _, _, r, _) in &self.rows {
            match r {
                Ok(o) if o.grade == Grade::Pass => c.0 += 1,
                Ok(o) if o.grade == Grade::Safe => c.1 += 1,
                Ok(_) => c.2 += 1,
                Err(_) => c.3 += 1,
            }
        }
        c
    }

    /// The scores nearest each v1 stop line, over every graded case.
    fn margins(&self) -> String {
        let mut m = Margins::default();
        for (_, _, _, r, _) in &self.rows {
            let Ok(o) = r else { continue };
            match &o.edge {
                Edge::Find => {}
                Edge::Check(p) => m.check.push(*p),
                Edge::Choose { top, next } => {
                    m.choose_top.push(*top);
                    m.choose_next.push(*next);
                }
                Edge::FindAll(scores) => m.find_all.extend(scores),
            }
        }
        let (lo, hi) = gate::UNSURE;
        let below = |v: &[f32]| v.iter().copied().filter(|p| *p < lo).reduce(f32::max);
        let above = |v: &[f32]| v.iter().copied().filter(|p| *p > hi).reduce(f32::min);
        let inside = |v: &[f32]| v.iter().filter(|p| (lo..=hi).contains(*p)).count();
        let show = |p: Option<f32>| p.map_or_else(|| "none".to_owned(), |p| format!("{p:.2}"));
        let mut md = String::from("\n## Margins to the v1 stop lines\n\n");
        let _ = writeln!(
            md,
            "- check ({} cases, unsure {lo}–{hi}): nearest yes {}, nearest no {}, inside {}.",
            m.check.len(),
            show(above(&m.check)),
            show(below(&m.check)),
            inside(&m.check)
        );
        let _ = writeln!(
            md,
            "- choose ({} cases, clear = top ≥ {} and next < {}): lowest top {}, highest next {}.",
            m.choose_top.len(),
            gate::ACCEPT,
            gate::RIVAL,
            show(m.choose_top.iter().copied().reduce(f32::min)),
            show(m.choose_next.iter().copied().reduce(f32::max))
        );
        let _ = writeln!(
            md,
            "- findAll ({} group scores, unsure {lo}–{hi}): nearest above {}, nearest below {}, inside {}.",
            m.find_all.len(),
            show(above(&m.find_all)),
            show(below(&m.find_all)),
            inside(&m.find_all)
        );
        md
    }

    fn write(&self, mode: &str, cases: &[Case]) -> Result<String, Box<dyn Error>> {
        let by_file: HashMap<&str, &Case> = cases.iter().map(|c| (c.file.as_str(), c)).collect();
        let (pass, safe, wrong, excluded) = self.counts();
        let graded = pass + safe + wrong;
        let mut md = format!(
            "# neo-jev replay — {mode}\n\n{pass}/{graded} pass, {safe} safe stop, {wrong} wrong, {excluded} excluded (of {} cases).\n\n",
            self.rows.len()
        );
        md.push_str("Pass = the verdict matches `correct`. Safe stop = the script stops where an answer existed (`find` not found or ambiguous; v1: `check` unsure, `choose` not clear, `findAll` unsure) and hands back, never acting on a wrong element. Wrong = acted on the wrong element or answer. New stops = the v1 stops among them.\n\n");
        md.push_str(
            "| site | kind | pass | safe | wrong | excluded | new stops |\n|---|---|---|---|---|---|---|\n",
        );
        let mut table: BTreeMap<(String, String), [usize; 5]> = BTreeMap::new();
        for (_, site, kind, r, _) in &self.rows {
            let slot = table.entry((site.clone(), kind.clone())).or_default();
            let i = match r {
                Ok(o) if o.grade == Grade::Pass => 0,
                Ok(o) if o.grade == Grade::Safe => 1,
                Ok(_) => 2,
                Err(_) => 3,
            };
            if let Some(n) = slot.get_mut(i) {
                *n += 1;
            }
            if let (Ok(o), Some(n)) = (r, slot.get_mut(4))
                && o.new_stop
            {
                *n += 1;
            }
        }
        for ((site, kind), [p, s, w, e, stops]) in &table {
            let _ = writeln!(
                md,
                "| {site} | {kind} | {p}/{} | {s} | {w} | {e} | {stops} |",
                p + s + w
            );
        }
        md.push_str(&self.margins());
        md.push_str("\n## Not passing\n\n");
        for (file, _, kind, r, cost) in &self.rows {
            let case = by_file.get(file.as_str());
            let ask = case.map(|c| c.ask.as_str()).unwrap_or_default();
            let correct = case.map(|c| c.correct.to_string()).unwrap_or_default();
            match r {
                Ok(o) if o.grade == Grade::Pass => {}
                Ok(o) => {
                    let _ = writeln!(
                        md,
                        "- **{file}** ({kind}, {:?}) ask: {ask:?}\n  verdict: {} · correct: {correct} {cost}",
                        o.grade, o.verdict
                    );
                }
                Err(why) => {
                    let _ = writeln!(md, "- **{file}** ({kind}, excluded) ask: {ask:?} — {why}");
                }
            }
        }
        let mut tsv = String::from("file\tkind\tgrade\tverdict\tcost\n");
        for (file, _, kind, r, cost) in &self.rows {
            let (grade, verdict) = match r {
                Ok(o) => (format!("{:?}", o.grade), o.verdict.clone()),
                Err(why) => ("Excluded".into(), why.clone()),
            };
            let _ = writeln!(tsv, "{file}\t{kind}\t{grade}\t{verdict}\t{cost}");
        }
        if let Ok(out) = std::env::var("NEO_JEV_REPLAY_OUT") {
            std::fs::create_dir_all(&out)?;
            std::fs::write(Path::new(&out).join(format!("replay-{mode}.md")), &md)?;
            std::fs::write(Path::new(&out).join(format!("replay-{mode}.tsv")), &tsv)?;
        }
        Ok(format!("{md}\n{tsv}"))
    }
}

fn eval_dir() -> Option<PathBuf> {
    let dir = std::env::var("NEO_JEV_EVAL_DIR")
        .ok()
        .filter(|d| !d.is_empty());
    if dir.is_none() {
        eprintln!("NEO_JEV_EVAL_DIR unset: replay skipped");
    }
    dir.map(PathBuf::from)
}

#[test]
fn offline_replay_matches_the_labels() -> TestResult {
    let Some(dir) = eval_dir() else { return Ok(()) };
    let cases = load(&dir)?;
    let mut picked = Picked::default();
    let mut report = Report::default();
    for case in &cases {
        let result = offline(case, &mut picked);
        report.add(case, result, String::new());
    }
    let text = report.write("offline", &cases)?;
    eprintln!("{text}");
    assert!(
        !cases.is_empty(),
        "NEO_JEV_EVAL_DIR names a folder of eval cases"
    );
    Ok(())
}

#[tokio::test]
async fn live_replay_matches_the_labels() -> TestResult {
    let Some(dir) = eval_dir() else { return Ok(()) };
    if std::env::var("NEO_JEV_LIVE").as_deref() != Ok("1") {
        eprintln!("NEO_JEV_LIVE != 1: live replay skipped");
        return Ok(());
    }
    let config = JevConfig::from_env().ok_or("NEO_JEV_KEY_FILE names no readable key")?;
    let jev = Jev::new(config);
    let cases = load(&dir)?;
    let mut picked = Picked::default();
    let mut report = Report::default();
    for case in &cases {
        match live(&jev, case, &mut picked).await {
            Ok((outcome, cost)) => report.add(case, Ok(outcome), cost),
            Err(why) => report.add(case, Err(why), String::new()),
        }
    }
    let text = report.write("live", &cases)?;
    eprintln!("{text}");
    Ok(())
}
