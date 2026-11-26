//! A selector chunk cut into the stages that run it: tree stages Rust evaluates over the
//! [`SnapshotTree`](crate::tree::SnapshotTree), page stages the page's vendored engine evaluates
//! (design 5.4). Planning is pure; neo-playwright's `ChunkRun` carries the current set through
//! the stages.
//!
//! Upstream's `querySelectorAll` folds a chunk's parts left to right over a set of roots, each
//! part reading only that set and the query root (`injectedScript.ts:404-427`). So the parts can
//! be cut into consecutive runs and the set carried from one run to the next, and the answer is
//! the one a single fold would give. The plan cuts maximal runs, and gives every run a page form:
//!
//! ```text
//!  internal:role=row >> internal:has-text="Pro"i >> internal:role=button >> nth=0
//!  └ Tree ─────────┘    └ Page ────────────────┘    └ Tree ─────────────────────┘
//!
//!  page form of a run: each tree leaf replaced by a call-scoped engine and a candidate
//!    css=tr >> internal:has="css=td >> internal:role=button"
//!    css=tr >> internal:has="css=td >> internal:neo-below=0"     candidates[0] = role=button
//! ```
//!
//! A page stage can hold tree leaves inside its combinators (`filter({ has })` over a nested
//! selector that mixes the two sides). Its page form replaces each leaf (`role`,
//! `internal:role`, `aria-ref`) with `internal:neo-below=<i>` or `internal:neo-inside=<i>`, and
//! records the leaf as candidate `i`. `ChunkRun` evaluates every candidate over the frame's whole
//! document and hands the matches to the page, which answers each engine with the candidates
//! inside the root it is called with. That is exact because a role or ref match is a fact of the
//! element alone, so "the matches inside root R" is "the document's matches, inside R". The
//! page's own loop then applies `nth`, `has`, `and`, `or` and `chain` with upstream's roots, in
//! one probe call per stage. A tree run keeps a page form too, for a set whose roots the tree has
//! no node for (design 5.4, path 3).
//!
//! Planning also does what the probe does before it queries anything: it refuses an engine name
//! the page's table lacks, with the probe's words. The call-scoped names are among them, so agent
//! input can never reach the engines a stage registers. The check has to happen here, on the
//! whole chunk: a page stage sees only its own parts, rewritten, and could neither name the
//! chunk in its message nor refuse before an earlier tree stage ran.

use std::mem;

use thiserror::Error;

use super::parsed_selector::{NestedSelectorBody, ParsedSelector, SelectorBody, SelectorPart};
use super::tree_stage::{TreeStage, TreeStageError};

/// The engines of Playwright 1.64.1's `InjectedScript` table (`injectedScript.ts:215-246`), in
/// its order. Neo registers no custom engine (`customEngines: []`) and replaces `aria-ref` under
/// the same name, so this is every name the probe's engine check accepts.
const ENGINE_NAMES: [&str; 32] = [
    "xpath",
    "xpath:light",
    "role",
    "text",
    "text:light",
    "id",
    "id:light",
    "data-testid",
    "data-testid:light",
    "data-test-id",
    "data-test-id:light",
    "data-test",
    "data-test:light",
    "css",
    "nth",
    "visible",
    "internal:control",
    "internal:has",
    "internal:has-not",
    "internal:and",
    "internal:or",
    "internal:chain",
    "internal:label",
    "internal:text",
    "internal:has-text",
    "internal:has-not-text",
    "internal:attr",
    "internal:testid",
    "internal:role",
    "internal:describe",
    "aria-ref",
    "aria-template",
];

/// The call-scoped engine a `role` or `internal:role` leaf becomes: the candidates strictly
/// inside the root, as upstream's role engine queries only below its root.
const NEO_BELOW: &str = "internal:neo-below";

/// The call-scoped engine an `aria-ref` leaf becomes: the candidates inside the root, the root
/// itself included, as Neo's `aria-ref` engine scopes (`isInsideScope`).
const NEO_INSIDE: &str = "internal:neo-inside";

/// One chunk, planned: the chunk as the prelude sent it, and the stages that run it.
#[derive(Clone, Debug)]
pub struct ChunkPlan {
    chunk: ParsedSelector,
    body: PlanBody,
}

#[derive(Clone, Debug)]
enum PlanBody {
    Stages(Stages),
    /// A captured chunk (`*`) with a top-level `nth`. Upstream throws when the chunk runs, not
    /// when it is parsed (`injectedScript.ts:378-379`), so the error waits for the run.
    NthWithCapture,
}

/// The stages of a plan, in order. There is always a first, and the kinds alternate: every run
/// is as long as it can be.
#[derive(Clone, Debug)]
pub struct Stages {
    first: PlannedStage,
    rest: Vec<PlannedStage>,
}

/// One maximal run of a chunk's parts.
#[derive(Clone, Debug)]
pub enum PlannedStage {
    /// Parts Rust evaluates over the tree, with the page form a set of roots the tree has no
    /// node for runs them in.
    Tree(TreeRun),
    /// Parts the page evaluates, tree leaves inside them substituted.
    Page(PageForm),
}

/// A run of tree parts: `role`, `internal:role` and `aria-ref` leaves, the combinators whose
/// nested selectors hold only tree parts, and `nth` or `internal:describe` beside them.
#[derive(Clone, Debug)]
pub struct TreeRun {
    stage: TreeStage,
    page_form: PageForm,
}

/// A run as the page runs it, in one `stage` probe call: its parts, every tree leaf at any depth
/// replaced by a call-scoped engine whose body indexes [`Self::candidates`].
#[derive(Clone, Debug)]
pub struct PageForm {
    selector: ParsedSelector,
    candidates: Vec<TreeStage>,
}

/// Why a chunk has no plan: a part names an engine the page's table lacks. Both texts are the
/// probe's own (`checkEngines`, `neo-playwright-js/src/probe/resolve.ts`), which it classifies
/// `selector`: they are the op's message as they stand, with no `Error: ` before them.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ChunkPlanError {
    /// A `ref` part. Agents write it from other tools, so Neo names its two spellings of a ref
    /// instead of upstream's generic text (design D3).
    #[error(
        "Unknown selector engine \"ref\". Use page.getByRef('{reference}') or 'aria-ref={reference}'."
    )]
    RefEngine {
        /// The part's body, trimmed and unquoted as the probe does.
        reference: String,
    },
    /// Any other name upstream's `InjectedScript` has no engine for, the call-scoped engines'
    /// names and the layout names (`left-of`, ...) included.
    #[error("Unknown engine \"{name}\" while parsing selector {selector}")]
    UnknownEngine {
        /// The first such name, in upstream's visiting order.
        name: String,
        /// The whole chunk, as upstream's `stringifySelector` prints it.
        selector: String,
    },
}

impl ChunkPlan {
    /// Plans `chunk`: refuses an unknown engine anywhere in it, rewrites a top-level capture,
    /// then cuts the parts into maximal tree and page runs.
    pub fn of(chunk: &ParsedSelector) -> Result<Self, ChunkPlanError> {
        check_engines(chunk)?;
        // Only the chunk's own parts count: a nested `nth` is its nested selector's business,
        // and nested captures stay for whichever side evaluates them.
        let body = if chunk.capture.is_some() && chunk.parts.iter().any(|part| part.name == "nth") {
            PlanBody::NthWithCapture
        } else {
            PlanBody::Stages(cut(without_capture(chunk)))
        };
        Ok(Self {
            chunk: chunk.clone(),
            body,
        })
    }

    /// The chunk as the prelude sent it: what a strict-mode violation names.
    #[must_use]
    pub fn chunk(&self) -> &ParsedSelector {
        &self.chunk
    }

    /// The stages that run the chunk, or the error upstream throws once it runs.
    pub fn stages(&self) -> Result<&Stages, TreeStageError> {
        match &self.body {
            PlanBody::Stages(stages) => Ok(stages),
            PlanBody::NthWithCapture => Err(TreeStageError::NthWithCapture),
        }
    }
}

impl Stages {
    /// The stage that runs from the chunk's query root.
    #[must_use]
    pub fn first(&self) -> &PlannedStage {
        &self.first
    }

    /// The stages after the first, each starting from the set the one before it produced.
    #[must_use]
    pub fn rest(&self) -> &[PlannedStage] {
        &self.rest
    }

    /// Every stage, in order.
    pub fn iter(&self) -> impl Iterator<Item = &PlannedStage> {
        [&self.first].into_iter().chain(&self.rest)
    }
}

impl TreeRun {
    /// The run as one tree stage.
    #[must_use]
    pub fn stage(&self) -> &TreeStage {
        &self.stage
    }

    /// The run as the page runs it, when a root of its set is not a node of the tree, or the
    /// chunk's query root is an element the tree has no node for.
    #[must_use]
    pub fn page_form(&self) -> &PageForm {
        &self.page_form
    }
}

impl PageForm {
    /// Substitutes every tree leaf of `run`, at any depth, in the order upstream's visitor meets
    /// them, so candidate indexes start at 0 for each run.
    fn of(run: &ParsedSelector) -> Self {
        let mut candidates = Vec::new();
        let selector = substitute(run, &mut candidates);
        Self {
            selector,
            candidates,
        }
    }

    /// The selector the `stage` call sends.
    #[must_use]
    pub fn selector(&self) -> &ParsedSelector {
        &self.selector
    }

    /// Candidate `i` is the tree leaf that `internal:neo-below=<i>` or `internal:neo-inside=<i>`
    /// stands for, as a one-part stage that the run evaluates over the frame's whole document.
    #[must_use]
    pub fn candidates(&self) -> &[TreeStage] {
        &self.candidates
    }
}

/// Upstream's engine check, run before anything is queried (`checkEngines` in the probe's
/// `resolve.ts`, through `visitAllSelectorParts`): every part, then its nested parts, depth
/// first, and the first unknown name fails the chunk.
fn check_engines(chunk: &ParsedSelector) -> Result<(), ChunkPlanError> {
    let mut pending: Vec<&SelectorPart> = chunk.parts.iter().rev().collect();
    while let Some(part) = pending.pop() {
        if !ENGINE_NAMES.contains(&part.name.as_str()) {
            return Err(match (part.name.as_str(), &part.body) {
                ("ref", SelectorBody::Text(body)) => ChunkPlanError::RefEngine {
                    reference: unquote(js_trim(body)).to_owned(),
                },
                _ => ChunkPlanError::UnknownEngine {
                    name: part.name.clone(),
                    selector: stringify(chunk),
                },
            });
        }
        if let SelectorBody::Nested(nested) = &part.body {
            pending.extend(nested.parsed.parts.iter().rev());
        }
    }
    Ok(())
}

/// Upstream's `stringifySelector` (`isomorphic/selectorParser.ts:157-171`): each part as
/// `name=source`, a `css` part and an `xpath` part that reads as a path as their source alone,
/// and the captured part marked `*`.
fn stringify(selector: &ParsedSelector) -> String {
    let parts: Vec<String> = selector
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            if selector.capture == Some(index) {
                return format!("*{}={}", part.name, part.source);
            }
            let bare = part.name == "css"
                || (part.name == "xpath"
                    && (part.source.starts_with("//") || part.source.starts_with("..")));
            if bare {
                part.source.clone()
            } else {
                format!("{}={}", part.name, part.source)
            }
        })
        .collect();
    parts.join(" >> ")
}

/// ECMAScript's `String.prototype.trim`, which differs from Rust's on two characters: it strips
/// U+FEFF and keeps U+0085.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// The probe's `unquote`: one pair of matching quotes around the text, removed.
fn unquote(text: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|quote| text.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(text)
}

/// `*a >> b >> c` as upstream runs it, `a >> internal:has="b >> c"`: the captured part's
/// elements, kept when the rest matches inside them (`injectedScript.ts:377-387`).
fn without_capture(chunk: &ParsedSelector) -> Vec<SelectorPart> {
    let Some(capture) = chunk.capture else {
        return chunk.parts.clone();
    };
    let kept = capture.saturating_add(1);
    let mut parts: Vec<SelectorPart> = chunk.parts.iter().take(kept).cloned().collect();
    let rest: Vec<SelectorPart> = chunk.parts.iter().skip(kept).cloned().collect();
    if !rest.is_empty() {
        let parsed = ParsedSelector {
            parts: rest,
            capture: None,
        };
        let source = stringify(&parsed);
        parts.push(SelectorPart {
            name: "internal:has".to_owned(),
            body: SelectorBody::Nested(NestedSelectorBody {
                parsed,
                distance: None,
            }),
            source,
        });
    }
    parts
}

/// The side that evaluates a run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Tree,
    Page,
}

/// The side that evaluates `part`, or `None` for `nth` and `internal:describe`, which read only
/// the set and so join whichever run they follow. A combinator is a tree part when its nested
/// selector is, at every depth; any other part is the page's, layout parts with nested bodies
/// included.
fn side(part: &SelectorPart) -> Option<Side> {
    let tree = match (part.name.as_str(), &part.body) {
        ("nth" | "internal:describe", _) => return None,
        ("role" | "internal:role" | "aria-ref", SelectorBody::Text(_)) => true,
        (
            "internal:has" | "internal:has-not" | "internal:and" | "internal:or" | "internal:chain",
            SelectorBody::Nested(nested),
        ) => TreeStage::new(&nested.parsed).is_ok(),
        _ => false,
    };
    Some(if tree { Side::Tree } else { Side::Page })
}

/// Cuts `parts` into maximal runs. Neutral parts before the first sided part join the first
/// run; a chunk with no sided part (only `nth` and `internal:describe`, or none at all) is one
/// tree run.
fn cut(parts: Vec<SelectorPart>) -> Stages {
    let mut runs: Vec<(Side, Vec<SelectorPart>)> = Vec::new();
    let mut leading = Vec::new();
    for part in parts {
        let side = side(&part);
        match runs.last_mut() {
            Some((last, run)) if side.is_none_or(|side| side == *last) => run.push(part),
            _ => match side {
                None => leading.push(part),
                Some(side) => {
                    let mut run = mem::take(&mut leading);
                    run.push(part);
                    runs.push((side, run));
                }
            },
        }
    }
    let mut stages = runs.into_iter().map(|(side, parts)| stage(side, parts));
    let first = stages.next().unwrap_or_else(|| stage(Side::Tree, leading));
    Stages {
        first,
        rest: stages.collect(),
    }
}

fn stage(side: Side, parts: Vec<SelectorPart>) -> PlannedStage {
    let run = ParsedSelector {
        parts,
        capture: None,
    };
    let page_form = PageForm::of(&run);
    match side {
        Side::Tree => match TreeStage::new(&run) {
            Ok(stage) => PlannedStage::Tree(TreeRun { stage, page_form }),
            // `side` accepts only parts the tree stage plans, so this is a hand-built part of an
            // odd shape (an `nth` with a nested body). The page form runs any parts exactly, so
            // falling back costs speed, never an answer.
            Err(_) => PlannedStage::Page(page_form),
        },
        Side::Page => PlannedStage::Page(page_form),
    }
}

fn substitute(selector: &ParsedSelector, candidates: &mut Vec<TreeStage>) -> ParsedSelector {
    ParsedSelector {
        parts: selector
            .parts
            .iter()
            .map(|part| substitute_part(part, candidates))
            .collect(),
        // A leaf becomes one part, so a nested selector's capture still points at its part.
        capture: selector.capture,
    }
}

fn substitute_part(part: &SelectorPart, candidates: &mut Vec<TreeStage>) -> SelectorPart {
    let engine = match (part.name.as_str(), &part.body) {
        ("role" | "internal:role", SelectorBody::Text(_)) => Some(NEO_BELOW),
        ("aria-ref", SelectorBody::Text(_)) => Some(NEO_INSIDE),
        _ => None,
    };
    let leaf = ParsedSelector {
        parts: vec![part.clone()],
        capture: None,
    };
    if let Some(engine) = engine
        && let Ok(candidate) = TreeStage::new(&leaf)
    {
        let index = candidates.len();
        candidates.push(candidate);
        return SelectorPart {
            name: engine.to_owned(),
            body: SelectorBody::Text(index.to_string()),
            source: part.source.clone(),
        };
    }
    match &part.body {
        // Combinators and layout parts stay, so the page's loop evaluates them with upstream's
        // roots; only the leaves under them change.
        SelectorBody::Nested(nested) => SelectorPart {
            name: part.name.clone(),
            body: SelectorBody::Nested(NestedSelectorBody {
                parsed: substitute(&nested.parsed, candidates),
                distance: nested.distance,
            }),
            source: part.source.clone(),
        },
        SelectorBody::Text(_) | SelectorBody::Css(_) => part.clone(),
    }
}
