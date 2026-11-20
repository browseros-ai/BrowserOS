//! Jev's answers -> a verdict. The only home of thresholds. Pure.
//!
//! `find`: probabilities are summed per twin group (`view::Group`). The top group is accepted when
//! it reaches `ACCEPT` and the next group stays under `RIVAL`; `none` leading means not found (the
//! prelude re-looks); anything else is ambiguous at once, since waiting will not settle a split.
//! Several batches are not comparable with each other, so when more than one batch has a leader
//! that beats its own `none`, the gate asks for one re-ask over the finalists.
//!
//! `choose` is clear by the same two lines as `find`; `check` and `find_all` are unsure inside
//! `UNSURE`. The gate only labels: the prelude stops the script on an unclear `choose`, an unsure
//! `check` and any unsure `find_all` element, so a guess never becomes a silent wrong branch.

use std::collections::HashMap;

use crate::view::{self, View};
use crate::{EarlierPick, FindOutcome, Flags, Obstacle, Pick};

/// The top twin group must reach this to be found; the top option, for `choose` to be clear.
pub const ACCEPT: f32 = 0.5;
/// The runner-up group (or option, for `choose`) must stay under this for the top one to be found
/// (or clear). Tuned for `find` on the 93 eval cases (small sample): after twin merging, every
/// right pick's runner-up was ≤ 0.08 recorded and ≤ 0.14 live; the one confident-wrong pick (City
/// dropdown answered with State's `e180`) had a runner-up of 0.21. The synthesis' 0.25 let that
/// pick through.
pub const RIVAL: f32 = 0.2;
/// `check` says yes at or above this.
pub const CHECK_YES: f32 = 0.5;
/// `check` and `find_all` are unsure inside this band, both ends included. The 93 eval cases never
/// came near it: checks scored ≥ 0.88 or ≤ 0.04, `find_all` groups ≥ 0.97 or ≤ 0.08.
pub const UNSURE: (f32, f32) = (0.35, 0.65);
/// `find_all` keeps a candidate at or above this, unless it is unsure.
pub const MATCH: f32 = 0.5;
/// The obstacle question names an obstacle only at or above this.
pub const OBSTACLE: f32 = 0.5;
/// Groups carried into the re-ask when several batches each have a leader.
pub const FINALISTS: usize = 20;
// The re-ask is one Choice: its finalists' offered refs must fit one batch.
const _: () = assert!(
    FINALISTS * view::MAX_GROUP_REFS <= view::MAX_BATCH_REFS,
    "the re-ask's finalists must fit one Choice"
);
/// Lookalikes a verdict reports.
pub const LOOKALIKES: usize = 3;

/// One Choice batch's answer: probability per offered ref, and `none`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answer {
    /// Probability per ref, as Jev returned it (`none` excluded).
    pub probs: Vec<(String, f32)>,
    /// Probability of `none`.
    pub none: f32,
}

/// What `find` decided, before `saw`, `obstacle` and `cost` are attached.
#[derive(Debug, Clone, PartialEq)]
pub struct FindDecision {
    /// Found, not found or ambiguous.
    pub outcome: FindOutcome,
    /// The pick when found.
    pub pick: Option<Pick>,
    /// Top other groups, best first.
    pub lookalikes: Vec<Pick>,
    /// `reused`.
    pub flags: Flags,
}

/// The gate's answer to one round of batches.
#[derive(Debug, Clone, PartialEq)]
pub enum FindGate {
    /// A verdict.
    Decided(FindDecision),
    /// More than one batch has a leader: re-ask one Choice over these groups (page order).
    ReAsk(Vec<usize>),
}

/// A twin group with its summed probability. `group` is `None` for a ref the view never showed.
/// Every rule reads `raw`; `p` is rounded for the verdict only, so 0.495 is never accepted as 0.50.
#[derive(Debug, Clone)]
struct Scored {
    group: Option<usize>,
    r#ref: String,
    label: String,
    raw: f32,
    p: f32,
}

fn score(view: &View, answer: &Answer) -> Vec<Scored> {
    let mut by_group: HashMap<usize, f32> = HashMap::new();
    let mut strays: Vec<Scored> = Vec::new();
    for (r, p) in &answer.probs {
        match view.group_of(r) {
            Some(g) => *by_group.entry(g).or_default() += p,
            None => strays.push(Scored {
                group: None,
                r#ref: r.clone(),
                label: "?".into(),
                raw: *p,
                p: round2(*p),
            }),
        }
    }
    let mut scored: Vec<Scored> = by_group
        .into_iter()
        .filter_map(|(g, p)| {
            let group = view.groups().get(g)?;
            Some(Scored {
                group: Some(g),
                r#ref: group.rep.clone(),
                label: group.label.clone(),
                raw: p,
                p: round2(p),
            })
        })
        .chain(strays)
        .collect();
    scored.sort_by(by_score);
    scored
}

/// Best first; ties break by page order, so the verdict is the same on every run.
fn by_score(a: &Scored, b: &Scored) -> std::cmp::Ordering {
    b.raw.total_cmp(&a.raw).then(a.group.cmp(&b.group))
}

fn round2(p: f32) -> f32 {
    (p * 100.0).round() / 100.0
}

fn pick_of(s: &Scored) -> Pick {
    Pick {
        r#ref: s.r#ref.clone(),
        label: s.label.clone(),
        p: s.p,
    }
}

/// Decides `find` over one round of batch answers (one per batch, in batch order).
#[must_use]
pub fn find(view: &View, answers: &[Answer], ask: &str, picked: &[EarlierPick]) -> FindGate {
    let per_batch: Vec<Vec<Scored>> = answers.iter().map(|a| score(view, a)).collect();
    let leaders: Vec<usize> = (0..answers.len())
        .filter(|&b| {
            let top = per_batch
                .get(b)
                .and_then(|s| s.first())
                .map_or(0.0, |s| s.raw);
            let none = answers.get(b).map_or(1.0, |a| a.none);
            top > 0.0 && top > none
        })
        .collect();

    if leaders.len() > 1 {
        let mut finalists: Vec<&Scored> = per_batch.iter().flatten().collect();
        finalists.sort_by(|a, b| by_score(a, b));
        let mut groups: Vec<usize> = finalists
            .iter()
            .filter_map(|s| s.group)
            .take(FINALISTS)
            .collect();
        groups.sort_unstable();
        groups.dedup();
        return FindGate::ReAsk(groups);
    }

    // The deciding batch: the one leader, else the batch whose best group came closest.
    let deciding = leaders.first().copied().or_else(|| {
        (0..per_batch.len()).max_by(|&a, &b| {
            let pa = per_batch
                .get(a)
                .and_then(|s| s.first())
                .map_or(0.0, |s| s.raw);
            let pb = per_batch
                .get(b)
                .and_then(|s| s.first())
                .map_or(0.0, |s| s.raw);
            pa.total_cmp(&pb)
        })
    });
    let ranked: &[Scored] = deciding
        .and_then(|b| per_batch.get(b))
        .map_or(&[], Vec::as_slice);
    let none = deciding
        .and_then(|b| answers.get(b))
        .map_or(1.0, |a| a.none);
    let top = ranked.first();
    let top_p = top.map_or(0.0, |s| s.raw);
    let next_p = ranked.get(1).map_or(0.0, |s| s.raw);

    // Beating `none` matters only off the usual shape: a batch's probabilities sum to 1, so a top
    // at ACCEPT already leads `none`, save a tie.
    let clear = top_p >= ACCEPT && next_p < RIVAL && top_p > none;
    // A clear winner already picked this run under another ask is the one confident-wrong shape
    // the eval found (City answered with State's dropdown, 6 of 6 samples; never a right pick).
    // It stops as ambiguous so the script decides; `reused` names the earlier ask.
    let reused = top.filter(|_| clear).and_then(|t| {
        let group = view.group_of(&t.r#ref);
        picked
            .iter()
            .find(|earlier| {
                earlier.ask != ask
                    && (earlier.r#ref == t.r#ref
                        || (group.is_some() && view.group_of(&earlier.r#ref) == group))
            })
            .map(|earlier| earlier.ask.clone())
    });
    let outcome = if clear && reused.is_none() {
        FindOutcome::Found
    } else if !clear && none >= top_p {
        FindOutcome::NotFound
    } else {
        FindOutcome::Ambiguous
    };
    let pick = match outcome {
        FindOutcome::Found => top.map(pick_of),
        FindOutcome::NotFound | FindOutcome::Ambiguous => None,
    };

    // Lookalikes come from every batch: a miss should show the best of the whole page.
    let mut others: Vec<&Scored> = per_batch.iter().flatten().filter(|s| s.raw > 0.0).collect();
    others.sort_by(|a, b| by_score(a, b));
    let lookalikes: Vec<Pick> = others
        .into_iter()
        .filter(|s| pick.as_ref().is_none_or(|p| p.r#ref != s.r#ref))
        .take(LOOKALIKES)
        .map(pick_of)
        .collect();

    FindGate::Decided(FindDecision {
        outcome,
        pick,
        lookalikes,
        flags: Flags { reused },
    })
}

/// The line a miss report carries when `reused` stopped a pick.
#[must_use]
pub fn reused_hint(earlier_ask: &str) -> String {
    format!("already picked for {earlier_ask:?}; reuse that locator if you meant the same element")
}

/// What `find_all` decided, before `saw`, `obstacle` and `cost` are attached.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FindAllDecision {
    /// Groups at or above `MATCH` and outside `UNSURE`, page order.
    pub picks: Vec<Pick>,
    /// Groups scored inside `UNSURE`, page order. Any here stops the script, so `picks` is used
    /// only when this is empty.
    pub unsure: Vec<Pick>,
}

/// Decides `find_all`. `scores` holds one probability per candidate group (index into
/// `view.groups()`); both lists come back in page order, not score order.
#[must_use]
pub fn find_all(view: &View, scores: &[(usize, f32)]) -> FindAllDecision {
    let mut sorted: Vec<(usize, f32)> = scores.to_vec();
    sorted.sort_by_key(|&(g, _)| g);
    let mut decision = FindAllDecision::default();
    for (g, p) in sorted {
        let Some(group) = view.groups().get(g) else {
            continue;
        };
        let list = if in_unsure(p) {
            &mut decision.unsure
        } else if p >= MATCH {
            &mut decision.picks
        } else {
            continue;
        };
        list.push(Pick {
            r#ref: group.rep.clone(),
            label: group.label.clone(),
            p,
        });
    }
    decision
}

fn in_unsure(p: f32) -> bool {
    (UNSURE.0..=UNSURE.1).contains(&p)
}

/// `check`'s decision over one Noul probability.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckDecision {
    /// The claim holds.
    pub yes: bool,
    /// The probability sits inside `UNSURE`.
    pub unsure: bool,
}

/// Decides `check`.
#[must_use]
pub fn check(p: f32) -> CheckDecision {
    CheckDecision {
        yes: p >= CHECK_YES,
        unsure: in_unsure(p),
    }
}

/// What `choose` decided, before `saw`, `obstacle` and `cost` are attached.
#[derive(Debug, Clone, PartialEq)]
pub struct ChooseDecision {
    /// Every option with its probability, best first (ties in script order).
    pub ranked: Vec<(String, f32)>,
    /// The top option reaches `ACCEPT` and the next stays under `RIVAL`. Not clear stops the
    /// script: `choose` has no `none`, so a split means the page does not answer the question.
    pub clear: bool,
}

/// Decides `choose` over one probability per option, in the script's order.
#[must_use]
pub fn choose(options: &[String], probs: &[f32]) -> ChooseDecision {
    let mut order: Vec<(usize, f32)> = probs.iter().copied().enumerate().collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let ranked: Vec<(String, f32)> = order
        .into_iter()
        .filter_map(|(i, p)| Some((options.get(i)?.clone(), p)))
        .collect();
    let top = ranked.first().map_or(0.0, |(_, p)| *p);
    let next = ranked.get(1).map_or(0.0, |(_, p)| *p);
    ChooseDecision {
        clear: top >= ACCEPT && next < RIVAL,
        ranked,
    }
}

/// The obstacle from the obstacle Choice's probabilities, keyed by `Obstacle`'s snake_case name.
#[must_use]
pub fn obstacle(probs: &HashMap<String, f32>) -> Obstacle {
    let best = probs
        .iter()
        .filter(|(_, p)| **p >= OBSTACLE)
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(name, _)| name.as_str());
    match best {
        Some("captcha") => Obstacle::Captcha,
        Some("login_wall") => Obstacle::LoginWall,
        Some("cookie_banner") => Obstacle::CookieBanner,
        Some("error_page") => Obstacle::ErrorPage,
        Some("modal") => Obstacle::Modal,
        _ => Obstacle::None,
    }
}
