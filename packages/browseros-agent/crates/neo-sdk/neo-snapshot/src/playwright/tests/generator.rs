//! Seeded random nodes for the differential. They take every value the typed tree
//! admits, not only what a page writes: an explicit `false` in every flag, a `cursor` that is not
//! `pointer`, `""` in every string, a role or a ref with a line break, numbers of every form. Their
//! strings are built from the marks each of upstream's quoting tests looks for, so each mark lands
//! at the start, inside and at the end of a key, a text, a url and a placeholder.

use serde_json::Number;

use crate::aria_node::{AriaNode, Invalid, NodeBox, Tristate};
use crate::split_mix64::SplitMix64;

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
];

/// A few refs, so two nodes often share one.
const REFS: &[&str] = &["e1", "e2", "e3", "f1e1", "f1e2"];

const CURSORS: &[&str] = &["pointer", "default", "", "Pointer"];

pub(super) struct Generator(SplitMix64);

impl Generator {
    pub(super) fn new(seed: u64) -> Self {
        Self(SplitMix64(seed))
    }

    /// One node without children.
    pub(super) fn node(&mut self) -> AriaNode {
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
