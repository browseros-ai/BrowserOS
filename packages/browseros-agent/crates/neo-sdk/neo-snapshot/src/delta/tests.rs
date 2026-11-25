//! The delta in Rust against its judges (`crate::judges`), through `render` with `Call::Delta`:
//! the committed cases; the private sets of saved pages, where a folder names them; and today's
//! TypeScript renderer in QuickJS on changed copies of seeded random trees. Each is held to its
//! text and to what it keeps: nothing when the stored text was not read, otherwise a baseline
//! recorded as returned with the answer's very text.
//!
//! One judge stays below the interface, at its oracle's grain: the on-top expression, held to
//! `onTopDelta`'s own loop and expression in QuickJS, one text at a time.

use std::collections::BTreeMap;
use std::error::Error;
use std::{env, fs};

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::call::Call;
use crate::form::Passed;
use crate::judges::{Bundles, CaseFile, EarlierKind, Generator, NEO_CASES, Shown, count, seed};
use crate::neo::on_top_delta;
use crate::render::render;
use crate::snapshot::Snapshot;

use super::{Baseline, Earlier};

/// How many changed copies the gate's run draws.
const GATE_TREES: usize = 300;

/// How many on-top texts the gate's run of the expression draws.
const GATE_LINES: usize = 3_000;

/// The five first lines a delta can print, by their opening words.
const FIRST_LINES: [&str; 5] = [
    "# whole page: no earlier snapshot",
    "# whole page: no time was left",
    "# whole page: too much differs",
    "# no change since",
    "# changes since",
];

/// A delta call.
fn deltas(call: &Call<'_>) -> bool {
    matches!(call, Call::Delta { .. })
}

#[test]
fn tests_that_the_delta_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let mut differing = Vec::new();
    for found in cases.rendered_cases(deltas) {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        if let Some(difference) = case.expected().difference(&call, &answer, Shown::Lines)? {
            differing.push(format!("{}: {difference}", case.name));
        }
        judged += 1;
    }
    assert!(
        judged > 0,
        "neo.json holds delta cases, or this test judges nothing"
    );
    assert!(
        differing.is_empty(),
        "{} of {judged} delta cases differ from the committed text:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

/// The private sets are real pages' content, so they live outside the repository and never go to
/// the remote pool: the test reads the folders `NEO_SNAPSHOT_PRIVATE_CASES` names, each `*.json`
/// file in the committed cases' shape, and is run by hand where they are kept. Without the
/// variable it passes and says it was skipped. Nothing it prints holds a page's text: a
/// difference is named by its file, its case's number, its line and the two lengths.
#[test]
#[expect(
    clippy::print_stdout,
    reason = "the counts are the evidence a reviewer pastes; a silent skip must not read as a pass"
)]
fn tests_that_the_delta_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!("private sets, the delta: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)");
        return Ok(());
    };
    let mut differing: Vec<String> = Vec::new();
    let (mut judged, mut identical) = (0, 0);
    for file in &CaseFile::private_files(&folders)? {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
        let (mut file_judged, mut file_identical) = (0, 0);
        for (index, found) in cases.rendered_cases(deltas).enumerate() {
            let (case, call, name) = found?;
            let answer = render(&cases.snapshot(name)?, call);
            file_judged += 1;
            match case.expected().difference(&call, &answer, Shown::Lengths)? {
                Some(difference) => {
                    differing.push(format!("{shown}: delta case {index}: {difference}"));
                }
                None => file_identical += 1,
            }
        }
        println!(
            "private sets, the delta: {shown}: {file_identical} of {file_judged} saved answers \
             identical, the kept baseline too"
        );
        judged += file_judged;
        identical += file_identical;
    }
    println!("private sets, the delta: {identical} of {judged} identical");
    assert!(
        judged > 0,
        "the named folders hold no delta case, which proves nothing"
    );
    assert!(
        differing.is_empty(),
        "{} differences:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

/// The differential on changed copies: a generated tree is rendered as a whole page to get the
/// tab's baseline, a changed copy of it is asked for as a delta against that baseline (or with no
/// time left, or nothing stored, or another page's text), and the answer and what it keeps must
/// be today's renderer's. The run counts each first line it printed, and a run that never printed
/// one of them, or never named a layer gone, proves less than it says and fails.
#[test]
#[expect(
    clippy::print_stdout,
    reason = "the long run's counts of each answer are evidence a reviewer pastes"
)]
fn tests_that_the_delta_is_todays_on_changed_copies() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    let mut first_lines: BTreeMap<&str, usize> =
        FIRST_LINES.iter().map(|line| (*line, 0)).collect();
    let mut gone = 0;
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let first = generator.delta_snapshot();
        let kept = generator.passed();
        let second = generator.changed(&first);
        let passed = generator.delta_passed(kept);
        let kind = generator.earlier_kind();
        let stored = match kind {
            EarlierKind::NotRead | EarlierKind::Nothing => None,
            EarlierKind::OtherPage => Some(page_baseline(&generator.delta_snapshot(), &kept)?),
            EarlierKind::ThisPage => Some(page_baseline(&first, &kept)?),
        };
        let earlier = match (kind, &stored) {
            (_, Some(stored)) => Earlier::Text(stored),
            (EarlierKind::NotRead, None) => Earlier::NotRead,
            (_, None) => Earlier::Nothing,
        };
        let call = Call::Delta {
            passed: &passed,
            earlier,
        };
        let answer = render(&second, call);
        let todays = bundles.todays(&call, &second)?;
        let difference = todays.expected().difference(&call, &answer, Shown::Lines)?;
        assert!(
            difference.is_none(),
            "prints and keeps what today's renderer does: {difference:?}\nseed {seed:#x}, copy \
             {index}, {passed:?}, {earlier:?}: {first:?} then {second:?}"
        );
        if let Some(seen) = first_lines
            .iter_mut()
            .find_map(|(line, seen)| answer.text().starts_with(line).then_some(seen))
        {
            *seen += 1;
        }
        if answer
            .text()
            .split('\n')
            .any(|line| line.starts_with("# no longer on top: "))
        {
            gone += 1;
        }
    }
    println!(
        "the delta on changed copies, seed {seed:#x}: {first_lines:?}; a layer named gone in \
         {gone}"
    );
    assert!(
        first_lines.values().all(|seen| *seen > 0) && gone > 0,
        "every first line and a gone layer were printed and compared: {first_lines:?}, gone {gone}"
    );
    Ok(())
}

/// What a whole-page call with `passed` keeps on `snapshot`, as the tab's store would hold it.
fn page_baseline(snapshot: &Snapshot, passed: &Passed) -> Result<Baseline, String> {
    let page = render(snapshot, Call::Page(passed));
    let kept = page.pending().ok_or("a whole page keeps its text")?;
    Ok(Baseline {
        text: kept.text.to_owned(),
        form: kept.form,
    })
}

/// `onTopDelta` reads each earlier layer with a regular expression, written in Rust as explicit
/// character tests. Every generated text, quotes, backslashes, line terminators and every space
/// among them, is named gone exactly as `onTopDelta`'s own loop and expression name it in QuickJS.
#[test]
fn tests_that_a_gone_layer_is_named_as_javascripts_expression_names_it()
-> Result<(), Box<dyn Error>> {
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let seed = seed()?;
    let mut generator = Generator::new(seed);
    let nothing_on_top = Snapshot::default();
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_LINES)? {
        let line = generator.on_top_line();
        let printed: Vec<String> = on_top_delta(&line, &nothing_on_top)
            .into_iter()
            .map(String::from)
            .collect();
        let todays: Vec<String> = context.with(|ctx| -> Result<Vec<String>, String> {
            ctx.globals()
                .set("line", line.as_str())
                .catch(&ctx)
                .map_err(|caught| caught.to_string())?;
            ctx.eval::<Vec<String>, _>(
                // `onTopDelta` from `snapshot-on-top.ts`, with no layer on top now.
                r#"(() => {
                    const removed = [];
                    for (const each of line.split('\n')) {
                        const match = /^# on top: (\S+ (?:"(?:[^"\\]|\\.)*" )?\[ref=([^\]]+)\])/.exec(each);
                        if (match) removed.push(`# no longer on top: ${match[1]}`);
                    }
                    return removed;
                })()"#,
            )
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
        })?;
        assert_eq!(
            printed, todays,
            "names the layer as JavaScript does; seed {seed:#x}, line {index}: {line:?}"
        );
    }
    Ok(())
}
