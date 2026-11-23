//! Seeded random nodes and trees for the differential. They take every value the typed tree
//! admits, not only what a page writes: an explicit `false` in every flag, a `cursor` that is not
//! `pointer`, `""` in every string, a role or a ref with a line break, numbers of every form. Their
//! strings are built from the marks each of upstream's quoting tests looks for, so each mark lands
//! at the start, inside and at the end of a key, a text, a url and a placeholder. A snapshot
//! carries every fact a page can have beside its tree (layers on top, unread closed roots, an
//! unfinished check for clickable elements) and, now and then, more text than Neo's format keeps,
//! and a call passes every combination of Neo's options.

use std::env;
use std::error::Error;

use serde_json::Number;

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

/// The deepest level a generated tree reaches, the top level being 0.
const MAX_DEPTH: usize = 4;

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

    /// The options of one call in Neo's default format.
    pub(crate) fn passed(&mut self) -> Passed {
        let max_tokens = if self.chance(30) {
            None
        } else if self.chance(80) {
            MaxTokens::new(self.pick(CAPS))
        } else {
            MaxTokens::new(f64::from(self.small()) / 8.0 + 0.125)
        };
        Passed {
            text: self.pick(&[None, Some(false), Some(true)]),
            urls: self.pick(&[None, Some(false), Some(true)]),
            max_tokens,
            inert: self.pick(&[None, Some(false), Some(true)]),
        }
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
