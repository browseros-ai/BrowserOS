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
use crate::cap::MaxTokens;
use crate::form::{Form, Passed, TextForm};
use crate::snapshot::{ClosedRoots, Layer, PartialFrame, Snapshot, Unmarked};

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
    pub(crate) call: CaseCall,
    pub(crate) text: Option<String>,
    /// What a whole page in Neo's format keeps as the tab's pending baseline.
    pub(crate) pending: Option<Pending>,
}

/// Which text, by its one key, with that call's options. The options of `mode: 'full'` and of a
/// delta are read by the pieces that print them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaseCall {
    Playwright {},
    Full {},
    /// A locator's snapshot in Neo's format.
    Element(CasePassed),
    /// A whole page in Neo's format.
    Page(CasePassed),
    Delta {},
    Cover {},
}

/// The options a default-format call passed, each absent when left out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CasePassed {
    text: Option<bool>,
    urls: Option<bool>,
    max_tokens: Option<f64>,
    inert: Option<bool>,
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

    /// The cases that ask for Playwright's format.
    pub(crate) fn playwright_cases(&self) -> impl Iterator<Item = &Case> {
        self.cases
            .iter()
            .filter(|case| matches!(case.call, CaseCall::Playwright {}))
    }

    /// The cases in Neo's default format, a locator's or a whole page's, with the options passed.
    pub(crate) fn neo_cases(&self) -> impl Iterator<Item = (&Case, &CasePassed)> {
        self.cases.iter().filter_map(|case| match &case.call {
            CaseCall::Element(passed) | CaseCall::Page(passed) => Some((case, passed)),
            CaseCall::Playwright {}
            | CaseCall::Full {}
            | CaseCall::Delta {}
            | CaseCall::Cover {} => None,
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
    /// Whether the call keeps a pending baseline: a whole page in Neo's format does.
    pub(crate) fn keeps_pending(&self) -> bool {
        matches!(self.call, CaseCall::Page(_))
    }
}

impl CasePassed {
    /// The options as the renderer takes them. A `maxTokens` the prelude refuses is not a case.
    pub(crate) fn passed(&self) -> Result<Passed, String> {
        let max_tokens = match self.max_tokens {
            Some(tokens) => Some(
                MaxTokens::new(tokens).ok_or_else(|| format!("maxTokens {tokens} is refused"))?,
            ),
            None => None,
        };
        Ok(Passed {
            text: self.text,
            urls: self.urls,
            max_tokens,
            inert: self.inert,
        })
    }
}

impl Pending {
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
            max_tokens: MaxTokens::new(*max_tokens)
                .ok_or_else(|| format!("maxTokens {max_tokens} is refused"))?,
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
