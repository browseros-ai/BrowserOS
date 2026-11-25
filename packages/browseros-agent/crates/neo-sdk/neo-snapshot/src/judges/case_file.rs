//! One file of snapshot text cases, as neo-playwright-js's case writer writes it
//! (`scripts/snapshot-cases/model.ts` documents the shape): named snapshots, then calls on them
//! with the text today's renderer printed. The committed files and the private sets of saved
//! pages share the shape.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::{env, fs, io};

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::aria_node::{AriaChild, AriaNode};
use crate::call::Call;
use crate::cap::MaxTokens;
use crate::delta::{Baseline, Earlier};
use crate::form::{Form, Passed, TextForm};
use crate::snapshot::{ClosedRoots, Layer, PartialFrame, Snapshot, Unmarked};

use super::expected::Expected;

/// Upstream's printer's own text on trees with no Neo fact; kept for as long as the vendored
/// printer is, and rewritten from it at every engine upgrade.
pub(crate) const PLAYWRIGHT_CASES: &str = include_str!("../../tests/data/cases/playwright.json");

/// Every other case, by today's renderer: Neo's formats, the covered block, and Playwright's
/// format on trees with Neo's facts.
pub(crate) const NEO_CASES: &str = include_str!("../../tests/data/cases/neo.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseFile {
    #[serde(rename = "about")]
    _about: IgnoredAny,
    snapshots: BTreeMap<String, CaseSnapshot>,
    cases: Vec<Case>,
}

/// One call on a named snapshot, and the text it printed (`null` only for a cover with nothing to
/// place).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(crate) name: String,
    pub(crate) snapshot: Option<String>,
    call: CaseCall,
    pub(crate) text: Option<String>,
    /// What a whole page in Neo's format keeps as the tab's pending baseline.
    pub(crate) pending: Option<Pending>,
}

/// Which text, by its one key, with that call's options, decoded as `render` takes them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum CaseCall {
    Playwright {},
    Full(CaseFull),
    /// A locator's snapshot in Neo's format.
    Element(CasePassed),
    /// A whole page in Neo's format.
    Page(CasePassed),
    /// A whole page with `{ delta: true }`.
    Delta(CaseDelta),
    Cover {},
}

/// `mode: 'full'`'s one option: the cap, `None` when the case left it out. A `maxTokens` the
/// prelude refuses is not a case, and fails the file's decode.
#[derive(Deserialize)]
#[serde(try_from = "FullJson")]
struct CaseFull(Option<MaxTokens>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FullJson {
    max_tokens: Option<f64>,
}

impl TryFrom<FullJson> for CaseFull {
    type Error = String;

    fn try_from(full: FullJson) -> Result<Self, String> {
        Ok(Self(full.max_tokens.map(max_tokens).transpose()?))
    }
}

/// The options a default-format call passed, each `None` when left out.
#[derive(Deserialize)]
#[serde(try_from = "PassedJson")]
struct CasePassed(Passed);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PassedJson {
    text: Option<bool>,
    urls: Option<bool>,
    max_tokens: Option<f64>,
    inert: Option<bool>,
}

impl TryFrom<PassedJson> for CasePassed {
    type Error = String;

    fn try_from(passed: PassedJson) -> Result<Self, String> {
        Ok(Self(Passed {
            text: passed.text,
            urls: passed.urls,
            max_tokens: passed.max_tokens.map(max_tokens).transpose()?,
            inert: passed.inert,
        }))
    }
}

/// A delta call: the options it passed and what it found for the tab, decoded as `render` takes
/// them, the stored text owned so the call can borrow it.
#[derive(Deserialize)]
#[serde(try_from = "DeltaJson")]
struct CaseDelta {
    passed: Passed,
    earlier: Found,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaJson {
    passed: CasePassed,
    earlier: EarlierJson,
}

/// `'notRead'`, `'nothing'`, or the stored text and its form.
#[derive(Deserialize)]
#[serde(untagged)]
enum EarlierJson {
    Word(EarlierWord),
    Text(Pending),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum EarlierWord {
    NotRead,
    Nothing,
}

impl TryFrom<DeltaJson> for CaseDelta {
    type Error = String;

    fn try_from(delta: DeltaJson) -> Result<Self, String> {
        let CasePassed(passed) = delta.passed;
        let earlier = match delta.earlier {
            EarlierJson::Word(EarlierWord::NotRead) => Found::NotRead,
            EarlierJson::Word(EarlierWord::Nothing) => Found::Nothing,
            EarlierJson::Text(stored) => Found::Text(stored.baseline()?),
        };
        Ok(Self { passed, earlier })
    }
}

/// What a delta found for the tab, owning the stored text.
enum Found {
    NotRead,
    Nothing,
    Text(Baseline),
}

impl Found {
    /// As `Call::Delta` takes it.
    fn earlier(&self) -> Earlier<'_> {
        match self {
            Self::NotRead => Earlier::NotRead,
            Self::Nothing => Earlier::Nothing,
            Self::Text(stored) => Earlier::Text(stored),
        }
    }
}

/// The cap as the renderer takes it, for a number the prelude accepts.
fn max_tokens(tokens: f64) -> Result<MaxTokens, String> {
    MaxTokens::new(tokens).ok_or_else(|| format!("maxTokens {tokens} is refused"))
}

/// A text and the form it was rendered in, as a baseline holds them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pending {
    pub(crate) text: String,
    form: CaseForm,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaseForm {
    text: CaseTextForm,
    urls: bool,
    max_tokens: f64,
    inert: Option<bool>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CaseTextForm {
    Kept,
    LeftOut,
    Always,
    Never,
}

/// A snapshot as the file holds it: the tree as JSON, or as a chain built here because it nests
/// deeper than a JSON file may, and Neo's facts about the page.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaseSnapshot {
    json: Option<Vec<AriaNode>>,
    chain: Option<Chain>,
    partial: Vec<CasePartial>,
    on_top: Option<Vec<CaseLayer>>,
    closed_roots: Option<CaseClosedRoots>,
    unmarked: Option<CaseUnmarked>,
}

/// `depth` nodes, each of the first `depth - 1` a copy of `link` whose one child is the next, the
/// last one `leaf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chain {
    depth: usize,
    link: AriaNode,
    leaf: AriaNode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CasePartial {
    r#ref: String,
    frame: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaseLayer {
    r#ref: String,
    line: String,
    whole_view: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseClosedRoots {
    sent: u32,
    found: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseUnmarked {
    reason: String,
}

impl CaseFile {
    /// The private sets' case files: each `*.json` file in the folders `folders` names, joined
    /// with the platform's path separator, a folder's files in name order.
    pub(crate) fn private_files(folders: &OsStr) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for folder in env::split_paths(folders) {
            let mut in_folder: Vec<PathBuf> = Vec::new();
            for entry in fs::read_dir(&folder)? {
                let path = entry?.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    in_folder.push(path);
                }
            }
            in_folder.sort();
            files.extend(in_folder);
        }
        Ok(files)
    }

    /// The cases [`render`](crate::render) prints whose call `wanted` picks, each with its call
    /// and its tree's name.
    pub(crate) fn rendered_cases(
        &self,
        wanted: impl Fn(&Call<'_>) -> bool,
    ) -> impl Iterator<Item = Result<(&Case, Call<'_>, &str), String>> {
        self.cases.iter().filter_map(move |case| {
            let call = case.call().filter(|call| wanted(call))?;
            Some(
                case.snapshot
                    .as_deref()
                    .map(|name| (case, call, name))
                    .ok_or_else(|| format!("{} names no snapshot", case.name)),
            )
        })
    }

    /// The cases that ask for a covered click's block.
    pub(crate) fn cover_cases(&self) -> impl Iterator<Item = &Case> {
        self.cases
            .iter()
            .filter(|case| matches!(case.call, CaseCall::Cover {}))
    }

    /// Every snapshot's name.
    pub(crate) fn snapshot_names(&self) -> impl Iterator<Item = &str> {
        self.snapshots.keys().map(String::as_str)
    }

    /// The snapshot named `name`, built as the renderer is handed it.
    pub(crate) fn snapshot(&self, name: &str) -> Result<Snapshot, String> {
        let found = self
            .snapshots
            .get(name)
            .ok_or_else(|| format!("no snapshot is named {name}"))?;
        let tree = match (&found.json, &found.chain) {
            (Some(json), None) => json.clone(),
            (None, Some(chain)) => vec![chain.built()],
            _ => return Err(format!("{name} holds neither a tree nor a chain, or both")),
        };
        let partial = found
            .partial
            .iter()
            .map(|frame| PartialFrame {
                r#ref: frame.r#ref.clone(),
                frame: frame.frame.clone(),
                reason: frame.reason.clone(),
            })
            .collect();
        let layers = found
            .on_top
            .iter()
            .flatten()
            .map(|layer| Layer {
                r#ref: layer.r#ref.clone(),
                line: layer.line.clone(),
                whole_view: layer.whole_view,
            })
            .collect();
        Ok(Snapshot {
            tree,
            partial,
            layers,
            closed_roots: found.closed_roots.as_ref().map(|roots| ClosedRoots {
                sent: roots.sent,
                found: roots.found,
            }),
            unmarked: found.unmarked.as_ref().map(|unmarked| Unmarked {
                reason: unmarked.reason.clone(),
            }),
        })
    }
}

impl Case {
    /// The case's call as `render` takes it; `None` for a cover, which `cover_block` prints.
    pub(crate) fn call(&self) -> Option<Call<'_>> {
        match &self.call {
            CaseCall::Playwright {} => Some(Call::Playwright),
            CaseCall::Full(CaseFull(max_tokens)) => Some(Call::Full {
                max_tokens: *max_tokens,
            }),
            CaseCall::Element(CasePassed(passed)) => Some(Call::Element(passed)),
            CaseCall::Page(CasePassed(passed)) => Some(Call::Page(passed)),
            CaseCall::Delta(CaseDelta { passed, earlier }) => Some(Call::Delta {
                passed,
                earlier: earlier.earlier(),
            }),
            CaseCall::Cover {} => None,
        }
    }

    /// What today's renderer printed and kept for this case.
    pub(crate) fn expected(&self) -> Expected<'_> {
        Expected {
            text: self.text.as_deref(),
            pending: self.pending.as_ref(),
        }
    }
}

impl Pending {
    /// The text and its form, as a tab's store keeps them.
    fn baseline(&self) -> Result<Baseline, String> {
        Ok(Baseline {
            text: self.text.clone(),
            form: self.form()?,
        })
    }

    /// The form the baseline was rendered in.
    pub(crate) fn form(&self) -> Result<Form, String> {
        let CaseForm {
            text,
            urls,
            max_tokens,
            inert,
        } = &self.form;
        Ok(Form {
            text: match text {
                CaseTextForm::Kept => TextForm::Kept,
                CaseTextForm::LeftOut => TextForm::LeftOut,
                CaseTextForm::Always => TextForm::Always,
                CaseTextForm::Never => TextForm::Never,
            },
            urls: *urls,
            max_tokens: self::max_tokens(*max_tokens)?,
            // The TypeScript writes the key only when it is true.
            inert: *inert == Some(true),
        })
    }
}

impl Chain {
    fn built(&self) -> AriaNode {
        let mut node = self.leaf.clone();
        for _ in 1..self.depth {
            let mut link = self.link.clone();
            link.children = vec![AriaChild::Node(Box::new(node))];
            node = link;
        }
        node
    }
}
