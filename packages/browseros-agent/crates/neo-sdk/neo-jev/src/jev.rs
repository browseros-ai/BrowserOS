//! The Jev client handle the server holds: the four verbs over `view`, `client` and `gate`.
//!
//! Each verb builds a fresh `View` from the page text, asks Jev (one request; `find` may add one
//! re-ask, `find_all` sends its requests in parallel), and hands the answers to `gate`.

use std::sync::Arc;
use std::time::Instant;

use futures_util::future;

use crate::client::{Client, OBSTACLE_ID, Ordered, Question, RULES, Reply, State};
use crate::gate::{self, Answer, FindGate};
use crate::view::View;
use crate::{
    CheckVerdict, ChooseVerdict, Cost, FindAllVerdict, FindOpts, FindOutcome, FindVerdict, Flags,
    JevConfig, JevFault, Obstacle, PageText, Saw,
};

/// Nouls per `find_all` request (the harness measured 100 per request at ≈300 ms).
const FIND_ALL_BATCH: usize = 100;

/// A cheap-to-clone Jev client. One per server; every run shares its connection pool.
#[derive(Clone, Debug)]
pub struct Jev {
    client: Arc<Client>,
}

/// Accumulates what a verb's requests cost.
struct Meter {
    started: Instant,
    cost: Cost,
    long_context: bool,
    obstacle: Option<Obstacle>,
}

impl Meter {
    fn new() -> Meter {
        Meter {
            started: Instant::now(),
            cost: Cost::default(),
            long_context: false,
            obstacle: None,
        }
    }

    fn add(&mut self, reply: &Reply) {
        self.cost.calls += reply.calls;
        self.cost.input_tokens += reply.input_tokens;
        self.long_context |= reply.long_context;
        // The first request's obstacle answer stands; a re-ask reads the same page.
        if self.obstacle.is_none() {
            self.obstacle = Some(gate::obstacle(&reply.answer(OBSTACLE_ID).probabilities));
        }
    }

    fn finish(&mut self, saw: &mut Saw) -> (Obstacle, Cost) {
        saw.long_context = self.long_context;
        let ms = self.started.elapsed().as_millis();
        self.cost.ms = u32::try_from(ms).unwrap_or(u32::MAX);
        (self.obstacle.unwrap_or_default(), self.cost)
    }
}

fn state<'a>(page: &'a PageText, text: &'a str) -> State<'a> {
    State {
        url: &page.url,
        title: &page.title,
        page: text,
    }
}

fn pick_question(view: &View, batch: &[usize], ask: &str) -> Question {
    let mut options: Vec<(String, Option<String>)> = view
        .offered(batch)
        .into_iter()
        .map(|r| (r.to_owned(), None))
        .collect();
    options.push((
        "none".into(),
        Some("None of the offered elements matches.".into()),
    ));
    Question::choice(
        serde_json::json!({
            "ask": ask,
            "question": "Which offered element is `ask`? Options are element refs from `page`; choose the one element a person would click or type into for `ask`. Choose `none` when no offered element matches.",
            "rules": RULES,
        }),
        options,
    )
}

fn answer_of(reply: &Reply, id: &str) -> Answer {
    let raw = reply.answer(id);
    let none = raw.probabilities.get("none").copied().unwrap_or(0.0);
    let probs = raw
        .probabilities
        .into_iter()
        .filter(|(r, _)| r != "none")
        .collect();
    Answer { probs, none }
}

impl Jev {
    /// Builds a client for `config`. Does no I/O.
    #[must_use]
    pub fn new(config: JevConfig) -> Jev {
        Jev {
            client: Arc::new(Client::new(config)),
        }
    }

    /// Which element matches `ask`.
    ///
    /// # Errors
    /// Only transport faults; a miss is a `FindVerdict`.
    pub async fn find(
        &self,
        page: &PageText,
        ask: &str,
        opts: &FindOpts,
    ) -> Result<FindVerdict, JevFault> {
        let mut meter = Meter::new();
        let view = View::new(&page.snapshot);
        let candidates = view.candidates(opts.role.as_deref());
        let batches = view.batches(&candidates);
        let mut saw = Saw {
            elements: view.elements(),
            candidates: candidates.len(),
            text: view.has_text(),
            batches: batches.len(),
            long_context: false,
            cut: None,
        };
        let st = state(page, view.text());

        let decision = if batches.is_empty() {
            // Nothing of that role on the page: not found without asking.
            gate::FindDecision {
                outcome: FindOutcome::NotFound,
                pick: None,
                lookalikes: Vec::new(),
                flags: Flags::default(),
            }
        } else {
            let questions = Ordered(
                batches
                    .iter()
                    .enumerate()
                    .map(|(i, batch)| (format!("pick_{i}"), pick_question(&view, batch, ask)))
                    .collect(),
            );
            let reply = self.client.ask(&st, questions).await?;
            meter.add(&reply);
            let answers: Vec<Answer> = (0..batches.len())
                .map(|i| answer_of(&reply, &format!("pick_{i}")))
                .collect();
            match gate::find(&view, &answers, ask, &opts.picked) {
                FindGate::Decided(decision) => decision,
                FindGate::ReAsk(finalists) => {
                    let questions =
                        Ordered(vec![("pick".into(), pick_question(&view, &finalists, ask))]);
                    let reply = self.client.ask(&st, questions).await?;
                    meter.add(&reply);
                    match gate::find(&view, &[answer_of(&reply, "pick")], ask, &opts.picked) {
                        FindGate::Decided(decision) => decision,
                        // One batch cannot have two leaders; kept total rather than trusted.
                        FindGate::ReAsk(_) => gate::FindDecision {
                            outcome: FindOutcome::Ambiguous,
                            pick: None,
                            lookalikes: Vec::new(),
                            flags: Flags::default(),
                        },
                    }
                }
            }
        };
        let (obstacle, cost) = meter.finish(&mut saw);
        Ok(FindVerdict {
            outcome: decision.outcome,
            pick: decision.pick,
            lookalikes: decision.lookalikes,
            flags: decision.flags,
            saw,
            obstacle,
            cost,
        })
    }

    /// Every element matching `ask`, in page order.
    ///
    /// # Errors
    /// Only transport faults.
    pub async fn find_all(
        &self,
        page: &PageText,
        ask: &str,
        opts: &FindOpts,
    ) -> Result<FindAllVerdict, JevFault> {
        let mut meter = Meter::new();
        let view = View::new(&page.snapshot);
        let candidates = view.candidates(opts.role.as_deref());
        let chunks: Vec<&[usize]> = candidates.chunks(FIND_ALL_BATCH).collect();
        let mut saw = Saw {
            elements: view.elements(),
            candidates: candidates.len(),
            text: view.has_text(),
            batches: chunks.len(),
            long_context: false,
            cut: None,
        };
        let st = state(page, view.text());
        let requests = chunks.iter().map(|chunk| {
            let questions = Ordered(
                chunk
                    .iter()
                    .filter_map(|&g| {
                        let group = view.groups().get(g)?;
                        let r = &group.rep;
                        Some((
                            r.clone(),
                            Question::noul(format!(
                                "Element {r} in `page` is: {}. Is element {r} {ask}? {RULES}",
                                group.label
                            )),
                        ))
                    })
                    .collect(),
            );
            self.client.ask(&st, questions)
        });
        // Requests run in parallel; results come back in chunk order, so page order holds.
        let replies = future::join_all(requests).await;
        let mut scores: Vec<(usize, f32)> = Vec::with_capacity(candidates.len());
        for (chunk, reply) in chunks.iter().zip(replies) {
            let reply = reply?;
            meter.add(&reply);
            for &g in *chunk {
                let rep = view
                    .groups()
                    .get(g)
                    .map(|group| group.rep.as_str())
                    .unwrap_or_default();
                scores.push((g, reply.answer(rep).noul.unwrap_or(0.0)));
            }
        }
        let gate::FindAllDecision { picks, unsure } = gate::find_all(&view, &scores);
        let (obstacle, cost) = meter.finish(&mut saw);
        Ok(FindAllVerdict {
            picks,
            unsure,
            saw,
            obstacle,
            cost,
        })
    }

    /// Whether `claim` holds on the page.
    ///
    /// # Errors
    /// Only transport faults.
    pub async fn check(&self, page: &PageText, claim: &str) -> Result<CheckVerdict, JevFault> {
        let mut meter = Meter::new();
        let view = View::new(&page.snapshot);
        let mut saw = Saw {
            elements: view.elements(),
            candidates: 0,
            text: view.has_text(),
            batches: 1,
            long_context: false,
            cut: None,
        };
        let questions = Ordered(vec![(
            "claim".into(),
            Question::noul(format!(
                "Is this true of the page as shown: \"{claim}\"? {RULES}"
            )),
        )]);
        let reply = self
            .client
            .ask(&state(page, &page.snapshot), questions)
            .await?;
        meter.add(&reply);
        let p = reply.answer("claim").noul.unwrap_or(0.0);
        let decision = gate::check(p);
        let (obstacle, cost) = meter.finish(&mut saw);
        Ok(CheckVerdict {
            yes: decision.yes,
            p,
            unsure: decision.unsure,
            saw,
            obstacle,
            cost,
        })
    }

    /// Which of the script's `options` answers `question`.
    ///
    /// # Errors
    /// Only transport faults (Jev refuses an empty list or more than 255 options as `Refused`).
    pub async fn choose(
        &self,
        page: &PageText,
        question: &str,
        options: &[String],
    ) -> Result<ChooseVerdict, JevFault> {
        let mut meter = Meter::new();
        let view = View::new(&page.snapshot);
        let mut saw = Saw {
            elements: view.elements(),
            candidates: options.len(),
            text: view.has_text(),
            batches: 1,
            long_context: false,
            cut: None,
        };
        // Ids, not the strings: the script owns the strings and may repeat or format them freely.
        let criteria = options
            .iter()
            .enumerate()
            .map(|(i, option)| (format!("o{i}"), Some(option.clone())))
            .collect();
        let questions = Ordered(vec![(
            "choose".into(),
            Question::choice(
                serde_json::json!({
                    "ask": question,
                    "question": "Which option answers `ask` for the page as shown?",
                    "rules": RULES,
                }),
                criteria,
            ),
        )]);
        let reply = self
            .client
            .ask(&state(page, &page.snapshot), questions)
            .await?;
        meter.add(&reply);
        let raw = reply.answer("choose");
        let probs: Vec<f32> = (0..options.len())
            .map(|i| {
                raw.probabilities
                    .get(&format!("o{i}"))
                    .copied()
                    .unwrap_or(0.0)
            })
            .collect();
        let gate::ChooseDecision { ranked, clear } = gate::choose(options, &probs);
        let (choice, p) = ranked.first().cloned().unwrap_or_default();
        let (obstacle, cost) = meter.finish(&mut saw);
        Ok(ChooseVerdict {
            choice,
            p,
            ranked,
            clear,
            saw,
            obstacle,
            cost,
        })
    }
}
