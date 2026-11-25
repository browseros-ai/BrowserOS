//! Seeded random nodes and trees for the differential. They take every value the typed tree
//! admits, not only what a page writes: an explicit `false` in every flag, a `cursor` that is not
//! `pointer`, `""` in every string, a role or a ref with a line break, numbers of every form. Their
//! strings are built from the marks each of upstream's quoting tests looks for, so each mark lands
//! at the start, inside and at the end of a key, a text, a url and a placeholder. A snapshot
//! carries every fact a page can have beside its tree (layers on top, unread closed roots, an
//! unfinished check for clickable elements) and, now and then, more text than Neo's format keeps,
//! and a call passes every combination of Neo's options. A cap is any number the prelude accepts,
//! fractions and the extremes of a double included, and a page for `mode: 'full'` is now and then
//! long enough to be cut at the default cap, with kept blocks and sections past the cut.
//!
//! For the delta, a snapshot can also carry a shelf of plain lines from a few words, so equal
//! lines repeat and the search meets its ties, and a changed copy of a snapshot edits a few places
//! of its tree and of its facts, as a page changes between two snapshots.

use std::env;
use std::error::Error;

use serde_json::{Number, Value};

use crate::aria_node::{AriaChild, AriaNode, Invalid, NodeBox, Tristate};
use crate::cap::MaxTokens;
use crate::form::Passed;
use crate::snapshot::{ClosedRoots, Layer, PartialFrame, Snapshot, Unmarked};
use crate::split_mix64::SplitMix64;

/// The seed of the random runs unless `NEO_SNAPSHOT_DIFFERENTIAL_SEED` names another.
const SEED: u64 = 0x5eed_7a11_0000_0002;

/// What a generated string is made of: zero to three of these, end to end.
const PIECES: &[&str] = &[
    // Plain words and the spaces around them.
    "a",
    "Buy now",
    "x y",
    " ",
    "  ",
    "\u{a0}",
    "\u{3000}",
    "\u{2028}",
    "\u{2029}",
    "\u{feff}",
    "\u{85}",
    "\u{200b}",
    // The separators a text line of Neo's format may hold alone and still print nothing.
    "\u{b7}",
    "\u{2022}",
    "\u{2014}",
    "\u{203a}",
    "\u{ab}",
    // Line breaks and control characters.
    "\t",
    "\n",
    "\r",
    "\r\n",
    "\0",
    "\u{7}",
    "\u{8}",
    "\u{b}",
    "\u{c}",
    "\u{e}",
    "\u{1b}",
    "\u{1f}",
    "\u{7f}",
    "\u{9f}",
    // The marks YAML reads at the start of a value, and those it reads anywhere.
    "-",
    "- ",
    "&",
    "*",
    "]",
    ",",
    "?",
    "!",
    ">",
    "|",
    "@",
    "\"",
    "'",
    "''",
    "#",
    " #",
    "%",
    "[",
    "{",
    "}",
    "`",
    ":",
    ": ",
    ":\n",
    "\\",
    "/",
    "~",
    // What the partial-frame scan and Neo's own lines look for.
    "[ref=e1]",
    "[ref=e2]",
    "# partial:",
    // YAML's words, in several cases.
    "y",
    "N",
    "yes",
    "No",
    "TRUE",
    "false",
    "oN",
    "OFF",
    "null",
    "Null",
    // Strings JavaScript's `Number` reads, and some it does not.
    "0",
    "-1",
    "1.5",
    ".5",
    "5.",
    "1e21",
    "1E-7",
    "0x1F",
    "0b2",
    "0o7",
    "Infinity",
    "-Infinity",
    "infinity",
    "NaN",
    "1_000",
    "+3",
    // Outside ASCII: the two characters that lower into ASCII letters, and wider ones.
    "\u{212a}",
    "\u{130}",
    "\u{e9}",
    "\u{65e5}\u{672c}",
    "\u{1f600}",
];

/// Roles a page writes, `text` and `iframe` among them; a generated role is sometimes any string.
const ROLES: &[&str] = &[
    "button",
    "link",
    "heading",
    "generic",
    "list",
    "listitem",
    "paragraph",
    "iframe",
    "text",
    "checkbox",
    "textbox",
    "img",
    "main",
    "dialog",
    "combobox",
    "option",
    "group",
    "row",
    // The roles Neo's format has a rule for.
    "alert",
    "alertdialog",
    "status",
    "banner",
    "navigation",
    "region",
    "form",
    "article",
    "listbox",
    "radio",
    "switch",
    "menuitemcheckbox",
    "searchbox",
    "slider",
    "tab",
    "cell",
];

/// A few refs, so two nodes, or a node and a partial frame, often share one.
const REFS: &[&str] = &["e1", "e2", "e3", "f1e1", "f1e2"];

const CURSORS: &[&str] = &["pointer", "default", "", "Pointer"];

const REASONS: &[&str] = &["timeout after 2000 ms", "frame detached", "target closed"];

/// Caps a script passes, fractions and the extremes among them, in tokens.
const CAPS: &[f64] = &[
    0.1, 1.0, 2.5, 10.0, 25.0, 30.0, 75.0, 100.0, 250.0, 1_000.0, 3_000.0, 1e9,
];

/// Caps at the edges of what the prelude accepts (`Number.isFinite(n) && n > 0`), in tokens.
const EXTREME_CAPS: &[f64] = &[5e-324, f64::MIN_POSITIVE, 1e-9, 1e300, f64::MAX];

/// The deepest level a generated tree reaches, the top level being 0.
const MAX_DEPTH: usize = 4;

/// What a layer's name quoted raw is made of, now and then: quotes, backslashes, and a backslash
/// before each line terminator JavaScript's `.` refuses, the escapes the on-top expression reads.
const ESCAPES: &[&str] = &[
    "a",
    " ",
    "\"",
    "\\",
    "\\\"",
    "\\\\",
    "\\\r",
    "\\\u{2028}",
    "\\\u{2029}",
    "\\\t",
    "\\\u{1f600}",
    "\" ",
    "]",
    "[ref=e1]",
];

/// The few names a shelf's lines take, so the same line comes up many times.
const SHELF_NAMES: &[&str] = &["Red mug", "Blue mug", "Tan mug", "Jug"];

/// What a generated delta found for the tab.
#[derive(Clone, Copy)]
pub(crate) enum EarlierKind {
    /// The stored text was not read.
    NotRead,
    /// Nothing was stored.
    Nothing,
    /// Another page's text: no comparison lists it, and its layers are named gone.
    OtherPage,
    /// The text a whole-page call kept on the snapshot the copy was changed from.
    ThisPage,
}

pub(crate) struct Generator(SplitMix64);

impl Generator {
    pub(crate) fn new(seed: u64) -> Self {
        Self(SplitMix64(seed))
    }

    /// One node without children.
    pub(crate) fn node(&mut self) -> AriaNode {
        let role = if self.chance(85) {
            self.pick(ROLES).to_owned()
        } else {
            self.string()
        };
        let name = self.maybe(60, Self::name);
        let checked = self.tristate();
        let disabled = self.flag();
        let expanded = self.flag();
        let active = self.flag();
        let inert = self.flag();
        let on_top = self.maybe(10, Self::reference);
        let invalid = self.invalid();
        let level = self.maybe(25, Self::number);
        let pressed = self.tristate();
        let selected = self.flag();
        let aria_hidden = self.flag();
        let r#ref = self.maybe(70, Self::reference);
        let cursor = self.maybe(40, |generator| {
            if generator.chance(80) {
                generator.pick(CURSORS).to_owned()
            } else {
                generator.string()
            }
        });
        let r#box = self.maybe(15, |generator| NodeBox {
            x: generator.number(),
            y: generator.number(),
            width: generator.number(),
            height: generator.number(),
        });
        let url = self.maybe(20, Self::string);
        let placeholder = self.maybe(15, Self::string);
        let text = self.maybe(20, Self::string);
        AriaNode {
            role,
            name,
            checked,
            disabled,
            expanded,
            active,
            inert,
            on_top,
            invalid,
            level,
            pressed,
            selected,
            aria_hidden,
            r#ref,
            cursor,
            r#box,
            url,
            placeholder,
            text,
            children: Vec::new(),
        }
    }

    /// A tree of one to four top-level nodes, each with up to three children a level, sometimes
    /// a paragraph of 12,000 units or so after them; up to three frames not read: most under a
    /// ref of the tree, some under none, some two for one; and Neo's facts about the page.
    pub(crate) fn snapshot(&mut self) -> Snapshot {
        let mut tree: Vec<AriaNode> = (0..=self.below(3)).map(|_| self.subtree(0)).collect();
        if self.chance(15) {
            tree.push(self.long_paragraph());
        }
        let mut partial: Vec<PartialFrame> = Vec::new();
        for _ in 0..self.below(4) {
            let iframe = match (self.below(5), partial.last()) {
                (0, Some(last)) => last.r#ref.clone(),
                (1, _) => "e9".to_owned(),
                _ => self.reference(),
            };
            let frame = if self.chance(70) {
                format!("https://pay.example/{}", self.string())
            } else {
                self.string()
            };
            let reason = if self.chance(80) {
                self.pick(REASONS).to_owned()
            } else {
                self.string()
            };
            partial.push(PartialFrame {
                r#ref: iframe,
                frame,
                reason,
            });
        }
        let layers = if self.chance(50) {
            (0..=self.below(3)).map(|_| self.layer()).collect()
        } else {
            Vec::new()
        };
        let closed_roots = self.maybe(20, |generator| ClosedRoots {
            sent: generator.small(),
            found: generator.small(),
        });
        let unmarked = self.maybe(20, |generator| Unmarked {
            reason: if generator.chance(70) {
                generator.pick(&["timeout", "page shape"]).to_owned()
            } else {
                generator.string()
            },
        });
        Snapshot {
            tree,
            partial,
            layers,
            closed_roots,
            unmarked,
        }
    }

    /// A snapshot for the delta: [`Generator::snapshot`], often with a shelf of plain lines from
    /// a few words after it, so a changed copy is mostly equal lines and the search meets ties.
    pub(crate) fn delta_snapshot(&mut self) -> Snapshot {
        let mut snapshot = self.snapshot();
        if self.chance(60) {
            let shelf = self.shelf();
            snapshot.tree.push(shelf);
        }
        snapshot
    }

    /// A copy of `snapshot` with zero to three places of its tree edited (a node added, removed,
    /// doubled, swapped with its neighbour, or renamed, refed, made inert or put on top) and now
    /// and then a layer, a frame not read, the closed roots or the unfinished check gained or
    /// lost.
    pub(crate) fn changed(&mut self, snapshot: &Snapshot) -> Snapshot {
        let mut tree = snapshot.tree.clone();
        for _ in 0..self.below(4) {
            self.change_tree(&mut tree);
        }
        let mut layers: Vec<Layer> = snapshot
            .layers
            .iter()
            .map(|layer| Layer {
                r#ref: layer.r#ref.clone(),
                line: layer.line.clone(),
                whole_view: layer.whole_view,
            })
            .collect();
        match self.below(6) {
            0 if !layers.is_empty() => {
                layers.remove(self.below(layers.len()));
            }
            1 => {
                let layer = self.layer();
                layers.push(layer);
            }
            2 => layers.clear(),
            _ => {}
        }
        let mut partial: Vec<PartialFrame> = snapshot
            .partial
            .iter()
            .map(|frame| PartialFrame {
                r#ref: frame.r#ref.clone(),
                frame: frame.frame.clone(),
                reason: frame.reason.clone(),
            })
            .collect();
        if !partial.is_empty() && self.chance(12) {
            partial.remove(self.below(partial.len()));
        }
        let closed_roots = match (&snapshot.closed_roots, self.chance(12)) {
            (Some(roots), false) => Some(ClosedRoots {
                sent: roots.sent,
                found: roots.found,
            }),
            (None, true) => Some(ClosedRoots {
                sent: self.small(),
                found: self.small(),
            }),
            _ => None,
        };
        let unmarked = match (&snapshot.unmarked, self.chance(12)) {
            (Some(unmarked), false) => Some(Unmarked {
                reason: unmarked.reason.clone(),
            }),
            (None, true) => Some(Unmarked {
                reason: "timeout".to_owned(),
            }),
            _ => None,
        };
        Snapshot {
            tree,
            partial,
            layers,
            closed_roots,
            unmarked,
        }
    }

    /// The options of a delta call on a changed copy: often none, which takes every option from
    /// the stored form, or the options the stored text was kept with, which agree with its form;
    /// otherwise any.
    pub(crate) fn delta_passed(&mut self, kept: Passed) -> Passed {
        match self.below(4) {
            0 | 1 => Passed::default(),
            2 => kept,
            _ => self.passed(),
        }
    }

    /// What a generated delta finds: mostly the text kept on the page it was changed from.
    pub(crate) fn earlier_kind(&mut self) -> EarlierKind {
        match self.below(20) {
            0..3 => EarlierKind::NotRead,
            3..6 => EarlierKind::Nothing,
            6..8 => EarlierKind::OtherPage,
            _ => EarlierKind::ThisPage,
        }
    }

    /// A line an earlier text may hold where a layer's would be: mostly as neo-playwright words a
    /// layer (`action::on_top_line`: the role, the name JSON-quoted or none, the ref, then
    /// `, blank`, `, loading`, `, says "…"`), with names quoted raw too, so quotes and backslashes
    /// meet the expression unescaped, and refs and roles of any string; sometimes any string
    /// after `# on top: `.
    pub(crate) fn on_top_line(&mut self) -> String {
        if self.chance(10) {
            return format!("# on top: {}", self.string());
        }
        let role = if self.chance(80) {
            self.pick(ROLES).to_owned()
        } else {
            self.string()
        };
        let reference = self.reference();
        let name = match self.below(5) {
            0 => None,
            1 => Some(format!("\"{}\"", self.string())),
            2 => {
                let escapes: String = (0..=self.below(5)).map(|_| self.pick(ESCAPES)).collect();
                Some(format!("\"{escapes}\""))
            }
            _ => Some(Value::from(self.string()).to_string()),
        };
        let mut line = match name {
            Some(name) => format!("# on top: {role} {name} [ref={reference}]"),
            None => format!("# on top: {role} [ref={reference}]"),
        };
        match self.below(5) {
            0 => line.push_str(", blank"),
            1 => line.push_str(", loading"),
            2 => {
                line.push_str(", says ");
                line.push_str(&Value::from(self.string()).to_string());
            }
            _ => {}
        }
        line.push_str(self.pick(&[" over the whole view", " over part of the view", ""]));
        line
    }

    /// The options of one call in Neo's default format.
    pub(crate) fn passed(&mut self) -> Passed {
        let max_tokens = self.max_tokens();
        Passed {
            text: self.pick(&[None, Some(false), Some(true)]),
            urls: self.pick(&[None, Some(false), Some(true)]),
            max_tokens,
            inert: self.pick(&[None, Some(false), Some(true)]),
        }
    }

    /// A `maxTokens` as a script may pass it, or `None` for one left out: a round cap, a fraction
    /// in eighths, an extreme double, or any positive finite double at all.
    pub(crate) fn max_tokens(&mut self) -> Option<MaxTokens> {
        match self.below(20) {
            0..=5 => None,
            6..=15 => MaxTokens::new(self.pick(CAPS)),
            16 | 17 => MaxTokens::new(f64::from(self.small()) / 8.0 + 0.125),
            18 => MaxTokens::new(self.pick(EXTREME_CAPS)),
            _ => {
                // A double's bits with the sign cleared, drawn again while not finite or zero.
                let mut tokens = MaxTokens::new(f64::from_bits(self.draw() >> 1));
                while tokens.is_none() {
                    tokens = MaxTokens::new(f64::from_bits(self.draw() >> 1));
                }
                tokens
            }
        }
    }

    /// A snapshot for `mode: 'full'`: [`Generator::snapshot`]'s; now and then a page that prints
    /// no line, whose text is its head lines alone with no blank line after them; and in one of
    /// four of the rest a page long enough to pass the default cap of 32,000 units, three or four
    /// paragraphs of about 12,000 placed among its nodes, so that the cut falls inside it and the
    /// kept blocks and sections after the paragraphs print past the cut or are offered.
    pub(crate) fn full_snapshot(&mut self) -> Snapshot {
        let mut snapshot = self.snapshot();
        if self.chance(6) {
            // Every line the cap holds comes from these three; the layers and the unfinished
            // check stay, and print outside it.
            snapshot.tree.clear();
            snapshot.partial.clear();
            snapshot.closed_roots = None;
        } else if self.chance(25) {
            for _ in 0..3 + self.below(2) {
                let at = self.below(snapshot.tree.len() + 1);
                let paragraph = self.long_paragraph();
                snapshot.tree.insert(at, paragraph);
            }
            for _ in 0..self.below(4) {
                let more = self.subtree(0);
                snapshot.tree.push(more);
            }
        }
        snapshot
    }

    /// A layer on top, worded as neo-playwright words one or as any string. Its ref is often one
    /// the tree's nodes carry as their `onTop` mark.
    fn layer(&mut self) -> Layer {
        let r#ref = self.reference();
        let line = if self.chance(80) {
            let mut line = format!("# on top: dialog \"{}\" [ref={}]", self.string(), r#ref);
            if self.chance(50) {
                line.push_str("\n#   its controls: button \"Accept\" [ref=e2]");
            }
            line
        } else {
            self.string()
        };
        Layer {
            r#ref,
            line,
            whole_view: self.chance(70),
        }
    }

    /// A paragraph whose text pieces join to about 12,000 units, the most Neo's format keeps when
    /// the script does not choose, a two-unit character among them.
    fn long_paragraph(&mut self) -> AriaNode {
        let mut paragraph = self.node();
        paragraph.role = "paragraph".to_owned();
        paragraph.name = None;
        paragraph.text = None;
        paragraph.inert = None;
        paragraph.on_top = None;
        paragraph.r#ref = None;
        for _ in 0..4 {
            let words = "word ".repeat(270 + self.below(60));
            paragraph
                .children
                .push(AriaChild::Text(format!("{words}\u{1f600}{words}")));
        }
        paragraph
    }

    /// A list of 10 to 59 plain lines, links and pictures named from [`SHELF_NAMES`], a few refs
    /// among them and now and then a text, so the same line repeats.
    fn shelf(&mut self) -> AriaNode {
        let mut shelf = plain("list", None, Some("e7".to_owned()));
        for _ in 0..10 + self.below(50) {
            let child = if self.chance(10) {
                AriaChild::Text("In stock".to_owned())
            } else {
                let role = self.pick(&["link", "img", "listitem"]);
                let name = self.pick(SHELF_NAMES).to_owned();
                let r#ref = self.maybe(30, |generator| generator.pick(REFS).to_owned());
                AriaChild::Node(Box::new(plain(role, Some(name), r#ref)))
            };
            shelf.children.push(child);
        }
        shelf
    }

    /// One edit at a random place of `tree`: at its top level, or inside a node.
    fn change_tree(&mut self, tree: &mut Vec<AriaNode>) {
        if tree.is_empty() || self.chance(25) {
            match self.below(5) {
                0 => {
                    let node = self.subtree(1);
                    tree.insert(self.below(tree.len() + 1), node);
                }
                1 if !tree.is_empty() => {
                    tree.remove(self.below(tree.len()));
                }
                2 if !tree.is_empty() => {
                    let at = self.below(tree.len());
                    let copy = tree[at].clone();
                    tree.insert(at, copy);
                }
                3 if tree.len() > 1 => {
                    let at = self.below(tree.len() - 1);
                    tree.swap(at, at + 1);
                }
                _ if !tree.is_empty() => {
                    let at = self.below(tree.len());
                    self.change_node(&mut tree[at]);
                }
                _ => {}
            }
            return;
        }
        let at = self.below(tree.len());
        self.change_inside(&mut tree[at]);
    }

    /// One edit inside `node`: in a node below it, half the time, or among its own children.
    fn change_inside(&mut self, node: &mut AriaNode) {
        let nodes: Vec<usize> = node
            .children
            .iter()
            .enumerate()
            .filter_map(|(index, child)| matches!(child, AriaChild::Node(_)).then_some(index))
            .collect();
        if !nodes.is_empty() && self.chance(50) {
            let at = nodes[self.below(nodes.len())];
            if let AriaChild::Node(child) = &mut node.children[at] {
                self.change_inside(child);
            }
            return;
        }
        let length = node.children.len();
        match self.below(6) {
            0 => {
                let child = if self.chance(30) {
                    AriaChild::Text(self.string())
                } else {
                    AriaChild::Node(Box::new(self.subtree(MAX_DEPTH - 1)))
                };
                node.children.insert(self.below(length + 1), child);
            }
            1 if length > 0 => {
                node.children.remove(self.below(length));
            }
            2 if length > 0 => {
                let at = self.below(length);
                let copy = node.children[at].clone();
                node.children.insert(at, copy);
            }
            3 if length > 1 => {
                let at = self.below(length - 1);
                node.children.swap(at, at + 1);
            }
            4 if length > 0 => {
                let at = self.below(length);
                node.children[at] = AriaChild::Text(self.string());
            }
            _ => self.change_node(node),
        }
    }

    /// One of `node`'s own facts changed: its name, its ref, its inert mark, its on-top mark or
    /// its role.
    fn change_node(&mut self, node: &mut AriaNode) {
        match self.below(5) {
            0 => node.name = self.maybe(70, Self::string),
            1 => node.r#ref = self.maybe(70, Self::reference),
            2 => node.inert = self.flag(),
            3 => node.on_top = self.maybe(50, Self::reference),
            _ => node.role = self.pick(ROLES).to_owned(),
        }
    }

    /// A count below 4,000.
    fn small(&mut self) -> u32 {
        u32::try_from(self.below(4_000)).unwrap_or_default()
    }

    fn subtree(&mut self, depth: usize) -> AriaNode {
        let mut node = self.node();
        // A native select's children are mostly its options and their groups, which Neo's format
        // collects through the groups, a ref or an empty ref among them.
        let select = node.role == "combobox" || node.role == "listbox";
        if depth < MAX_DEPTH {
            for _ in 0..self.below(4) {
                node.children.push(if self.chance(30) {
                    AriaChild::Text(self.string())
                } else {
                    let mut child = self.subtree(depth + 1);
                    if select && self.chance(70) {
                        child.role = self.pick(&["option", "option", "group"]).to_owned();
                    }
                    AriaChild::Node(Box::new(child))
                });
            }
        }
        node
    }

    /// Zero to three pieces end to end, so the empty string comes up too.
    fn string(&mut self) -> String {
        (0..self.below(4)).map(|_| self.pick(PIECES)).collect()
    }

    /// A name, sometimes around upstream's limit of 900 UTF-16 units, an emoji across it.
    fn name(&mut self) -> String {
        match self.below(8) {
            0 => format!("{}\u{1f600}", "a".repeat(898)),
            1 => format!("{}\u{1f600}", "a".repeat(899)),
            2 => "a".repeat(899 + self.below(3)),
            _ => self.string(),
        }
    }

    fn reference(&mut self) -> String {
        match self.below(10) {
            0 => String::new(),
            1 => self.string(),
            _ => self.pick(REFS).to_owned(),
        }
    }

    /// A JSON number of every form a level or a box may take: small and huge integers, `-0`,
    /// fractions, both sides of where JavaScript turns to an exponent, and random doubles.
    fn number(&mut self) -> Number {
        let double = match self.below(14) {
            0 => return Number::from(self.below(7)),
            1 => return Number::from(-3),
            2 => return Number::from((1_u64 << 53) + 1),
            3 => return Number::from(u64::MAX),
            4 => return Number::from(i64::MIN),
            5 => -0.0,
            6 => 2.5,
            7 => 1e21,
            8 => 1e-7,
            9 => 1e-6,
            10 => 1.234_567_890_123_456_8e20,
            11 => 0.1 + 0.2,
            12 => 5e-324,
            _ => f64::from_bits(self.draw()),
        };
        Number::from_f64(double).unwrap_or_else(|| Number::from(0))
    }

    fn flag(&mut self) -> Option<bool> {
        self.pick(&[None, Some(false), Some(true)])
    }

    fn tristate(&mut self) -> Option<Tristate> {
        self.pick(&[
            None,
            Some(Tristate::False),
            Some(Tristate::True),
            Some(Tristate::Mixed),
        ])
    }

    fn invalid(&mut self) -> Option<Invalid> {
        self.pick(&[
            None,
            Some(Invalid::False),
            Some(Invalid::True),
            Some(Invalid::Grammar),
            Some(Invalid::Spelling),
        ])
    }

    /// `make()` with the given chance in a hundred, otherwise `None`.
    fn maybe<T>(&mut self, percent: u64, make: impl FnOnce(&mut Self) -> T) -> Option<T> {
        self.chance(percent).then(|| make(self))
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.draw() % 100 < percent
    }

    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())]
    }

    /// A number from 0 up to, not including, `count`.
    fn below(&mut self, count: usize) -> usize {
        let count = u64::try_from(count).unwrap_or(u64::MAX);
        usize::try_from(self.draw() % count).unwrap_or_default()
    }

    fn draw(&mut self) -> u64 {
        self.0.next().unwrap_or_default()
    }
}

/// A node with a role, maybe a name and a ref, and nothing else.
fn plain(role: &str, name: Option<String>, r#ref: Option<String>) -> AriaNode {
    AriaNode {
        role: role.to_owned(),
        name,
        checked: None,
        disabled: None,
        expanded: None,
        active: None,
        inert: None,
        on_top: None,
        invalid: None,
        level: None,
        pressed: None,
        selected: None,
        aria_hidden: None,
        r#ref,
        cursor: None,
        r#box: None,
        url: None,
        placeholder: None,
        text: None,
        children: Vec::new(),
    }
}

/// The random runs' seed: `NEO_SNAPSHOT_DIFFERENTIAL_SEED` (decimal, or hex after `0x`), else
/// the gate's.
pub(crate) fn seed() -> Result<u64, Box<dyn Error>> {
    let Ok(text) = env::var("NEO_SNAPSHOT_DIFFERENTIAL_SEED") else {
        return Ok(SEED);
    };
    Ok(match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16)?,
        None => text.parse()?,
    })
}

/// How many a random run draws: the number `variable` names, else the gate's `gate`.
pub(crate) fn count(variable: &str, gate: usize) -> Result<usize, Box<dyn Error>> {
    match env::var(variable) {
        Ok(text) => Ok(text.parse()?),
        Err(_) => Ok(gate),
    }
}
