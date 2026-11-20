//! The pure halves on synthetic pages: twin groups, borrowed names, batches, and every gate rule.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Write as _;

use neo_jev::gate::{self, Answer, FindGate};
use neo_jev::view::{MAX_BATCH_REFS, View};
use neo_jev::{EarlierPick, FindOutcome, Obstacle};

type TestResult = Result<(), Box<dyn Error>>;

const SHOP: &str = include_str!("fixtures/shop.txt");

fn group_of<'a>(view: &'a View, r: &str) -> Result<&'a neo_jev::view::Group, Box<dyn Error>> {
    let g = view
        .group_of(r)
        .ok_or_else(|| format!("{r} has no group"))?;
    Ok(view.groups().get(g).ok_or("group index out of range")?)
}

fn answer(probs: &[(&str, f32)], none: f32) -> Answer {
    Answer {
        probs: probs.iter().map(|(r, p)| ((*r).to_owned(), *p)).collect(),
        none,
    }
}

fn decided(gate: FindGate) -> Result<gate::FindDecision, Box<dyn Error>> {
    match gate {
        FindGate::Decided(d) => Ok(d),
        FindGate::ReAsk(groups) => Err(format!("unexpected re-ask over {groups:?}").into()),
    }
}

#[test]
fn twins_merge_and_the_visible_control_represents_them() -> TestResult {
    let view = View::new(SHOP);
    let mug = group_of(&view, "e10")?;
    assert_eq!(mug.rep, "e11", "a hidden copy points at its visible twin");
    assert_eq!(
        mug.members,
        vec!["e10", "e11"],
        "the hidden copy's probability still counts"
    );
    assert_eq!(
        mug.offered,
        vec!["e11"],
        "the hidden copy is not offered to Jev"
    );
    let cart = group_of(&view, "e12")?;
    assert_eq!(
        cart.rep, "e13",
        "a clickable wrapper yields to the button beside it"
    );
    assert_eq!(
        cart.offered,
        vec!["e12", "e13"],
        "both stay offered; the gate sums them"
    );
    assert_eq!(
        group_of(&view, "e21")?.rep,
        "e20",
        "a radio's clickable label is its twin"
    );
    assert_ne!(
        view.group_of("e11"),
        view.group_of("e30"),
        "links to different hrefs stay apart"
    );
    assert_eq!(view.elements(), 16, "every ref'd line is an element");
    Ok(())
}

#[test]
fn nameless_controls_borrow_a_name_and_jev_reads_it() -> TestResult {
    let view = View::new(SHOP);
    assert_eq!(
        group_of(&view, "e11")?.label,
        "link \"Blue mug, 350 ml\"",
        "a card link takes the heading after it, not the one before"
    );
    assert_eq!(
        group_of(&view, "e40")?.label,
        "textbox \"City\"",
        "an input takes the label before it"
    );
    assert_eq!(
        group_of(&view, "e41")?.label,
        "combobox \"Select State\"",
        "a placeholder wins"
    );
    assert_eq!(
        group_of(&view, "e31")?.label,
        "link \"₹350 (it's 20% off)\"",
        "a YAML single-quoted line unwraps"
    );
    let text = view.text();
    assert!(
        text.contains("- link \"Blue mug, 350 ml\" [ref=e11]: /p/blue-mug\n"),
        "the borrowed name is written into the text Jev reads: {text}"
    );
    assert!(
        !text.contains("e10"),
        "a merged hidden copy is dropped from the text"
    );
    assert!(
        text.contains("- heading \"Mugs\""),
        "unrelated lines stay verbatim"
    );
    Ok(())
}

#[test]
fn role_filter_and_batches_keep_page_order_and_whole_groups() -> TestResult {
    let view = View::new(SHOP);
    let links: Vec<&str> = view
        .candidates(Some("link"))
        .into_iter()
        .filter_map(|g| view.groups().get(g).map(|group| group.rep.as_str()))
        .collect();
    assert_eq!(
        links,
        vec!["e2", "e11", "e30", "e31"],
        "only links, in page order"
    );

    let mut big = String::new();
    for i in 0..300 {
        let _ = writeln!(big, "- generic [ref=w{i}] [cursor=pointer]: Item {i}");
        let _ = writeln!(big, "- button \"Item {i}\" [ref=b{i}]");
    }
    let view = View::new(&big);
    let candidates = view.candidates(None);
    let batches = view.batches(&candidates);
    assert_eq!(candidates.len(), 300, "each wrapper merged with its button");
    for batch in &batches {
        assert!(
            view.offered(batch).len() <= MAX_BATCH_REFS,
            "a batch fits one Choice with `none`"
        );
    }
    assert_eq!(
        batches.len(),
        3,
        "600 refs in whole pairs need three batches of ≤254"
    );
    assert_eq!(
        view.offered(batches.first().ok_or("no batch")?)
            .first()
            .copied(),
        Some("w0"),
        "batches keep page order"
    );
    Ok(())
}

#[test]
fn gate_sums_twins_and_applies_the_thresholds() -> TestResult {
    let view = View::new(SHOP);
    let found = decided(gate::find(
        &view,
        &[answer(&[("e20", 0.81), ("e21", 0.19)], 0.0)],
        "Female",
        &[],
    ))?;
    assert_eq!(
        found.outcome,
        FindOutcome::Found,
        "0.81 + 0.19 is one element at 1.0"
    );
    assert_eq!(
        found.pick.as_ref().map(|p| (p.r#ref.as_str(), p.p)),
        Some(("e20", 1.0)),
        "the control, summed"
    );

    let rival = decided(gate::find(
        &view,
        &[answer(&[("e11", 0.71), ("e30", 0.21)], 0.08)],
        "mug",
        &[],
    ))?;
    assert_eq!(
        rival.outcome,
        FindOutcome::Ambiguous,
        "a runner-up at 0.21 is a real rival"
    );
    assert!(rival.pick.is_none(), "ambiguous names no pick");
    assert_eq!(
        rival.lookalikes.len(),
        2,
        "both groups are shown to the script"
    );

    let narrow = decided(gate::find(
        &view,
        &[answer(&[("e11", 0.6), ("e30", 0.15)], 0.25)],
        "mug",
        &[],
    ))?;
    assert_eq!(
        narrow.outcome,
        FindOutcome::Found,
        "0.15 is under the rival line"
    );
    Ok(())
}

#[test]
fn gate_reports_not_found_and_reused() -> TestResult {
    let view = View::new(SHOP);
    let missing = decided(gate::find(
        &view,
        &[answer(&[("e11", 0.02)], 0.98)],
        "the Delhi option",
        &[],
    ))?;
    assert_eq!(
        missing.outcome,
        FindOutcome::NotFound,
        "`none` leading means not on the page yet"
    );
    let split = decided(gate::find(
        &view,
        &[answer(&[("e11", 0.3), ("e30", 0.14)], 0.56)],
        "mug",
        &[],
    ))?;
    assert_eq!(
        split.outcome,
        FindOutcome::NotFound,
        "none at 0.56 leads a 0.30 group"
    );

    let earlier = [EarlierPick {
        r#ref: "e41".into(),
        ask: "the Select State dropdown".into(),
    }];
    let again = decided(gate::find(
        &view,
        &[answer(&[("e41", 0.9)], 0.05)],
        "the Select City dropdown",
        &earlier,
    ))?;
    assert_eq!(
        again.flags.reused.as_deref(),
        Some("the Select State dropdown"),
        "a ref picked for another ask is flagged with that ask"
    );
    assert_eq!(
        again.outcome,
        FindOutcome::Ambiguous,
        "a reused clear winner stops instead of acting"
    );
    assert!(again.pick.is_none(), "no pick when stopped");
    assert_eq!(
        again.lookalikes.first().map(|p| p.r#ref.as_str()),
        Some("e41"),
        "the reused element still leads the lookalikes"
    );
    assert_eq!(
        gate::reused_hint("the Select State dropdown"),
        "already picked for \"the Select State dropdown\"; reuse that locator if you meant the same element",
        "the miss report's wording"
    );
    let same = decided(gate::find(
        &view,
        &[answer(&[("e41", 0.9)], 0.05)],
        "the Select State dropdown",
        &earlier,
    ))?;
    assert!(
        same.flags.reused.is_none() && same.outcome == FindOutcome::Found,
        "the same ask picking the same ref is a re-look, not a reuse"
    );
    let twin = [EarlierPick {
        r#ref: "e21".into(),
        ask: "the Female label".into(),
    }];
    let via_twin = decided(gate::find(
        &view,
        &[answer(&[("e20", 0.95)], 0.0)],
        "the Female radio",
        &twin,
    ))?;
    assert!(
        via_twin.flags.reused.is_some() && via_twin.outcome == FindOutcome::Ambiguous,
        "a twin of an earlier pick is the same element"
    );
    Ok(())
}

#[test]
fn gate_re_asks_only_when_several_batches_lead() -> TestResult {
    let view = View::new(SHOP);
    let one = gate::find(
        &view,
        &[answer(&[("e11", 0.9)], 0.1), answer(&[("e30", 0.05)], 0.95)],
        "mug",
        &[],
    );
    assert!(
        matches!(&one, FindGate::Decided(d) if d.outcome == FindOutcome::Found),
        "the other batches saying `none` settles it without a re-ask: {one:?}"
    );
    let two = gate::find(
        &view,
        &[answer(&[("e11", 0.6)], 0.4), answer(&[("e30", 0.7)], 0.3)],
        "mug",
        &[],
    );
    let FindGate::ReAsk(groups) = two else {
        return Err(format!("two leaders must re-ask: {two:?}").into());
    };
    let reps: Vec<&str> = groups
        .iter()
        .filter_map(|&g| view.groups().get(g).map(|x| x.rep.as_str()))
        .collect();
    assert_eq!(reps, vec!["e11", "e30"], "finalists go back in page order");
    Ok(())
}

#[test]
fn check_choose_find_all_and_obstacle_rules() -> TestResult {
    assert!(
        gate::check(0.5).yes && gate::check(0.5).unsure,
        "0.5 is yes, and unsure"
    );
    assert!(
        !gate::check(0.34).yes && !gate::check(0.34).unsure,
        "0.34 is a clear no"
    );
    assert!(
        gate::check(0.66).yes && !gate::check(0.66).unsure,
        "0.66 is a clear yes"
    );

    let options = vec!["$128".to_owned(), "$173".to_owned(), "$178".to_owned()];
    let chosen = gate::choose(&options, &[0.03, 0.97, 0.0]);
    assert_eq!(
        chosen.ranked.first().map(|(c, _)| c.as_str()),
        Some("$173"),
        "the script's own string wins"
    );
    assert_eq!(chosen.ranked.len(), 3, "every option is ranked");
    assert!(
        chosen.clear,
        "0.97 over 0.03 is the eval's one choose, and clear"
    );

    let view = View::new(SHOP);
    let scores: Vec<(usize, f32)> = ["e30", "e2", "e11"]
        .iter()
        .zip([0.9, 0.1, 0.8])
        .filter_map(|(r, p)| Some((view.group_of(r)?, p)))
        .collect();
    let all = gate::find_all(&view, &scores);
    let picks: Vec<String> = all.picks.into_iter().map(|p| p.r#ref).collect();
    assert_eq!(
        picks,
        vec!["e11", "e30"],
        "matches come back in page order, not score order"
    );
    assert!(
        all.unsure.is_empty(),
        "0.9, 0.8 and 0.1 are all outside the band"
    );

    let probs: HashMap<String, f32> =
        [("none".to_owned(), 0.2), ("captcha".to_owned(), 0.8)].into();
    assert_eq!(
        gate::obstacle(&probs),
        Obstacle::Captcha,
        "a confident obstacle is named"
    );
    let unsure: HashMap<String, f32> =
        [("none".to_owned(), 0.45), ("modal".to_owned(), 0.4)].into();
    assert_eq!(
        gate::obstacle(&unsure),
        Obstacle::None,
        "no obstacle reaches 0.5"
    );
    Ok(())
}

#[test]
fn choose_is_clear_only_by_find_s_two_lines() -> TestResult {
    let options = vec![
        "sign in".to_owned(),
        "pick a size".to_owned(),
        "other".to_owned(),
    ];
    let clear = |probs: &[f32]| gate::choose(&options, probs).clear;
    assert!(
        clear(&[0.1, 0.8, 0.1]),
        "0.80 leads and 0.10 is under RIVAL"
    );
    assert!(
        clear(&[0.0, 0.5, 0.19]),
        "ACCEPT and RIVAL are the same lines as find's"
    );
    assert!(!clear(&[0.0, 0.49, 0.1]), "a top under 0.5 is not clear");
    assert!(
        !clear(&[0.2, 0.7, 0.1]),
        "a runner-up at 0.20 is a real rival"
    );
    assert!(!clear(&[0.4, 0.4, 0.2]), "a tie is not clear");
    let tie = gate::choose(&options, &[0.4, 0.4, 0.2]);
    assert_eq!(
        tie.ranked.first().map(|(c, _)| c.as_str()),
        Some("sign in"),
        "ties keep the script's order, so the stop's detail is stable"
    );
    assert!(
        !gate::choose(&[], &[]).clear,
        "no options is never clear (Jev refuses it before this)"
    );
    Ok(())
}

#[test]
fn find_all_puts_scores_inside_the_band_in_unsure_page_order() -> TestResult {
    let view = View::new(SHOP);
    let scores: Vec<(usize, f32)> = [
        ("e31", 0.35),
        ("e30", 0.97),
        ("e11", 0.65),
        ("e2", 0.66),
        ("e13", 0.34),
    ]
    .iter()
    .filter_map(|(r, p)| Some((view.group_of(r)?, *p)))
    .collect();
    assert_eq!(scores.len(), 5, "every fixture ref has a group");
    let all = gate::find_all(&view, &scores);
    let refs = |picks: &[neo_jev::Pick]| -> Vec<String> {
        picks.iter().map(|p| p.r#ref.clone()).collect()
    };
    assert_eq!(
        refs(&all.picks),
        vec!["e2", "e30"],
        "0.66 and 0.97 are picks: at or over MATCH and outside the band"
    );
    assert_eq!(
        refs(&all.unsure),
        vec!["e11", "e31"],
        "0.65 and 0.35 sit on the band's ends, which are included; page order"
    );
    assert!(
        !all.picks
            .iter()
            .chain(&all.unsure)
            .any(|p| p.r#ref == "e13"),
        "0.34 is a clear no, in neither list"
    );
    Ok(())
}
