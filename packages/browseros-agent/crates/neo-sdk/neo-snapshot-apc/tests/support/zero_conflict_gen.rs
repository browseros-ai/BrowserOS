//! The zero-conflict fuzzer's seeded page generator (design 10.1): one seed, one
//! `AnnotatedPageContent` shaped as Chromium reports it under contract v6, leaning on what breaks
//! quoting and name matching.
//!
//! ```text
//!  seed ─▶ SplitMix64 ─▶ Generator::document (main, then each iframe's child, ≤ 4 deep)
//!                          │  per element: a KINDS row, one of its name sources, a name drawn
//!                          │  from PALETTE, its states, a visibility
//!                          ▼
//!               NodeBuilder tree ─▶ ApcBuilder ─▶ GeneratedPage { apc, facts }
//!                                                                    │
//!          PageCheck (zero_conflict.rs) + facts ─▶ Coverage: what the printed lines exercised
//! ```
//!
//! Each shape copies a recorded one (`tests/fixtures/*.pb`): a `<button>` is a `ButtonButton`
//! form control tagged `button` with its text as children, a `role=checkbox` `<div>` is an
//! untagged `InputCheckbox` control, a value sits in a user-agent shadow text node. Inherited
//! facts are written where Chromium writes them: `aria_hidden` on every node below the element
//! that sets it, text included; `aria_disabled`, `inert` and `pointer_events_none` on every
//! element below; and `inert` alone across an iframe into its document. Document tokens, frame
//! tokens and per-document DOM ids are all distinct.

use std::collections::HashMap;

use neo_snapshot_apc::AnnotatedPageContent;
use neo_snapshot_apc::proto::{
    AxRole, BrowserOsRef, BrowserOsTriState, ClickabilityReason, FormControlType, SelectOption,
};
use neo_snapshot_apc::tree::{ApcBuilder, DocToken, DomNodeId, FrameBuilder, NodeBuilder, NodeKey};

use crate::zero_conflict::{InvalidMark, Mixable, PageCheck};

/// The first seed of the default run; tree `i` is seed `BASE_SEED + i`.
pub(crate) const BASE_SEED: u64 = 0x5EED_0000_0000_0000;

/// The deepest child document: the main document is depth 0.
const MAX_FRAME_DEPTH: usize = 4;

/// The viewport every page is captured in, in CSS pixels; a box below its bottom is below the
/// fold.
const VIEWPORT_HEIGHT: i32 = 600;

/// Sebastiano Vigna's `SplitMix64`: a 64-bit state stepped by the golden ratio and mixed, so every
/// seed, consecutive ones included, starts an independent stream.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`, or 0 when `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).unwrap_or(u64::MAX).max(1);
        usize::try_from(self.next() % n).unwrap_or(0)
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: usize, high: usize) -> usize {
        low + self.below(high.saturating_sub(low) + 1)
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    /// One item of a table, which is never empty; `None` only if it were.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        items.get(self.below(items.len()))
    }
}

/// Whether a palette entry's element prints a name when its line prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Printed {
    /// The name prints: the entry survived into the line.
    Name,
    /// No name prints (empty, whitespace only, over 900 UTF-16 units): the entry survived when
    /// its element's line prints with a ref and no name.
    NoName,
}

/// What a palette entry contributes to a name.
#[derive(Clone, Copy, Debug)]
enum Text {
    /// A fragment, combined with up to two others.
    Fragment(&'static str),
    /// A whole name, used alone.
    Whole(&'static str),
    /// A whole name of this many UTF-16 units: ASCII letters, the last two units an emoji when
    /// `emoji`.
    Long { units: usize, emoji: bool },
    /// An earlier name of the document with its ASCII case flipped (`Pay` beside `pay`).
    CaseOf,
    /// An earlier name of the document with words added (`Save`, `Save all`).
    PrefixOf,
    /// An earlier element of the document again: same kind, same source, same name.
    Duplicate,
}

/// One entry of the name palette.
struct Entry {
    label: &'static str,
    text: Text,
    printed: Printed,
}

const fn fragment(label: &'static str, text: &'static str) -> Entry {
    Entry {
        label,
        text: Text::Fragment(text),
        printed: Printed::Name,
    }
}

/// The names a page draws from: characters that quoting, escaping, whitespace handling or the
/// selector parser can get wrong, and the relations (case, prefix, duplicate) that exact matching
/// must keep apart.
const PALETTE: &[Entry] = &[
    fragment("double quote", "\""),
    fragment("single quote", "'"),
    fragment("backslash", "\\"),
    fragment("backslash quote", "\\\""),
    fragment("ends \"s", "Pay\"s"),
    fragment("ends \"i", "Pay\"i"),
    fragment("newline", "two\nlines"),
    fragment("tab", "a\tb"),
    fragment("carriage return", "a\rb"),
    fragment("leading spaces", "  lead"),
    fragment("trailing spaces", "trail  "),
    fragment("inner spaces", "in    ner"),
    fragment("nbsp leading", "\u{a0}nb"),
    fragment("nbsp trailing", "nb\u{a0}"),
    fragment("nbsp inner", "n\u{a0}b"),
    fragment("U+200B", "zero\u{200b}width"),
    fragment("U+200C", "non\u{200c}joiner"),
    fragment("U+200D", "joi\u{200d}ner"),
    fragment("U+FEFF", "b\u{feff}om"),
    fragment("U+00AD", "soft\u{ad}hyphen"),
    fragment("U+2028", "line\u{2028}sep"),
    fragment("U+2029", "para\u{2029}sep"),
    fragment("U+0085", "next\u{85}line"),
    fragment("U+3000", "ideo\u{3000}space"),
    fragment("U+2007", "fig\u{2007}space"),
    fragment("emoji", "\u{1f600}"),
    fragment("skin tone", "\u{1f44d}\u{1f3fd}"),
    fragment("zwj family", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"),
    fragment("flag", "\u{1f1ef}\u{1f1f5}"),
    fragment("cjk", "\u{652f}\u{6255}\u{3046}"),
    fragment("arabic", "\u{627}\u{62f}\u{641}\u{639}"),
    fragment("hebrew", "\u{5e9}\u{5dc}\u{5dd}"),
    fragment("combining mark", "cafe\u{301}"),
    fragment(">>", ">>"),
    fragment(">> nth=0", ">> nth=0"),
    fragment("[", "["),
    fragment("]", "]"),
    fragment("=", "="),
    fragment("[name=\"x\"s]", "[name=\"x\"s]"),
    fragment("*", "*"),
    fragment("#x", "#x"),
    fragment("- x", "- x"),
    fragment("x: y", "x: y"),
    fragment("x:", "x:"),
    fragment("yes", "yes"),
    fragment("null", "null"),
    fragment("True", "True"),
    fragment("123", "123"),
    fragment("1e3", "1e3"),
    fragment("0x10", "0x10"),
    fragment("/sub.*mit/i", "/sub.*mit/i"),
    fragment("backtick", "`"),
    fragment("{}", "{}"),
    fragment("U+0001", "\u{1}"),
    fragment("U+007F", "\u{7f}"),
    fragment("Pay", "Pay"),
    fragment("Save", "Save"),
    fragment("plain", "Continue"),
    Entry {
        label: "empty",
        text: Text::Whole(""),
        printed: Printed::NoName,
    },
    Entry {
        label: "whitespace only",
        text: Text::Whole(" \t "),
        printed: Printed::NoName,
    },
    Entry {
        label: "899 units",
        text: Text::Long {
            units: 899,
            emoji: false,
        },
        printed: Printed::Name,
    },
    Entry {
        label: "900 units",
        text: Text::Long {
            units: 900,
            emoji: false,
        },
        printed: Printed::Name,
    },
    Entry {
        label: "900 units, emoji last",
        text: Text::Long {
            units: 900,
            emoji: true,
        },
        printed: Printed::Name,
    },
    Entry {
        label: "901 units",
        text: Text::Long {
            units: 901,
            emoji: false,
        },
        printed: Printed::NoName,
    },
    Entry {
        label: "901 units, emoji last",
        text: Text::Long {
            units: 901,
            emoji: true,
        },
        printed: Printed::NoName,
    },
    Entry {
        label: "case pair",
        text: Text::CaseOf,
        printed: Printed::Name,
    },
    Entry {
        label: "prefix family",
        text: Text::PrefixOf,
        printed: Printed::Name,
    },
    Entry {
        label: "exact duplicate",
        text: Text::Duplicate,
        printed: Printed::Name,
    },
];

/// Every palette text a name can be built from, each relation shown by an example, for the
/// escaping test.
pub(crate) fn palette_names() -> Vec<String> {
    let mut names = Vec::new();
    for entry in PALETTE {
        match entry.text {
            Text::Fragment(text) | Text::Whole(text) => names.push(text.to_owned()),
            Text::Long { units, emoji } => names.push(long_name(units, emoji)),
            Text::CaseOf => names.extend(["pay".to_owned(), "PAY".to_owned()]),
            Text::PrefixOf => names.extend(["Save all".to_owned(), "Save all changes".to_owned()]),
            Text::Duplicate => {}
        }
    }
    names
}

fn long_name(units: usize, emoji: bool) -> String {
    let letters = if emoji {
        units.saturating_sub(2)
    } else {
        units
    };
    let mut name: String = (b'a'..=b'z')
        .cycle()
        .take(letters)
        .map(char::from)
        .collect();
    if emoji {
        name.push('\u{1f600}');
    }
    name
}

/// Where an element's name comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// `aria-label`.
    AriaLabel,
    /// Its text: one to three text nodes, some inside inline elements, some after a break.
    Content,
    /// `title`.
    Title,
    /// `alt`.
    Alt,
    /// A `<label for>` before it.
    LabelFor,
    /// `aria-labelledby` naming one or two spans elsewhere in the document.
    Labelledby,
    /// `placeholder`.
    Placeholder,
    /// A button input's `value`.
    Value,
}

/// A state an element can be given, through the contract field Chromium writes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateKind {
    /// `checked` true or false.
    Checked,
    /// `checked` true, false or mixed.
    CheckedOrMixed,
    /// `aria-pressed` true, false or mixed.
    Pressed,
    /// `aria-expanded` true or false.
    Expanded,
    /// `aria-selected="true"`.
    Selected,
    /// `aria-level` 1 to 6.
    Level,
    /// A natively disabled control.
    Disabled,
    /// `aria-disabled="true"`.
    AriaDisabled,
    /// `aria-invalid`.
    Invalid,
    /// The document's focused element.
    Focus,
}

/// Every state, for states put on kinds that do not take them.
const ALL_STATES: [StateKind; 10] = [
    Checked,
    CheckedOrMixed,
    Pressed,
    Expanded,
    Selected,
    Level,
    Disabled,
    AriaDisabled,
    Invalid,
    Focus,
];

/// How a kind's node is built.
#[derive(Clone, Copy, Debug)]
enum Base {
    /// `<button>`.
    Button,
    /// `<input type=button>`.
    InputButton,
    /// `<a href>`.
    Link,
    /// A native `<input>` toggle, `role=switch` on it when `role` is set.
    NativeToggle {
        control: FormControlType,
        role: Option<&'static str>,
    },
    /// A `<div>` with an ARIA widget role; Chromium reports a toggle role as a form control.
    Aria {
        role: &'static str,
        ax: AxRole,
        control: Option<FormControlType>,
    },
    /// `<input type=text>`.
    TextInput,
    /// `<select>`: a combobox.
    Select,
    /// `<select multiple>`: a list box with option nodes.
    SelectListbox,
    /// A `role=listbox` `<div>` with `role=option` children.
    AriaListbox,
    /// `<h1>` to `<h6>`.
    Heading,
    /// `<img>`.
    Image,
    /// A plain `<div>` with `cursor: pointer`.
    ClickableDiv,
    /// An element whose role prohibits a name.
    Prohibited {
        tag: Option<&'static str>,
        role: Option<&'static str>,
        paragraph: bool,
    },
    /// `<iframe>` with a child document.
    Iframe,
}

/// One element kind: how it is built, where its name can come from, which states it takes.
struct Kind {
    label: &'static str,
    base: Base,
    sources: &'static [Source],
    states: &'static [StateKind],
    /// Whether its role lets a name print.
    nameable: bool,
}

use Source::{Alt, AriaLabel, Content, LabelFor, Labelledby, Placeholder, Title, Value};
use StateKind::{
    AriaDisabled, Checked, CheckedOrMixed, Disabled, Expanded, Focus, Invalid, Level, Pressed,
    Selected,
};

/// The element kinds, one row each. A source listed twice is drawn twice as often.
const KINDS: &[Kind] = &[
    Kind {
        label: "button",
        base: Base::Button,
        sources: &[Content, Content, AriaLabel, Title, LabelFor, Labelledby],
        states: &[Pressed, Expanded, Disabled, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "input type=button",
        base: Base::InputButton,
        sources: &[Value, Value, AriaLabel, Title, Labelledby],
        states: &[Pressed, Disabled, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=button",
        base: Base::Aria {
            role: "button",
            ax: AxRole::Button,
            control: None,
        },
        sources: &[Content, AriaLabel],
        states: &[Pressed, Expanded, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "link",
        base: Base::Link,
        sources: &[Content, Content, AriaLabel, Title, Labelledby],
        states: &[Expanded, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "checkbox",
        base: Base::NativeToggle {
            control: FormControlType::InputCheckbox,
            role: None,
        },
        sources: &[LabelFor, LabelFor, AriaLabel, Title, Labelledby],
        states: &[CheckedOrMixed, Disabled, AriaDisabled, Invalid, Focus],
        nameable: true,
    },
    Kind {
        label: "radio",
        base: Base::NativeToggle {
            control: FormControlType::InputRadio,
            role: None,
        },
        sources: &[LabelFor, AriaLabel, Labelledby],
        states: &[Checked, Disabled, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "input role=switch",
        base: Base::NativeToggle {
            control: FormControlType::InputCheckbox,
            role: Some("switch"),
        },
        sources: &[LabelFor, AriaLabel],
        states: &[Checked, Disabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=checkbox",
        base: Base::Aria {
            role: "checkbox",
            ax: AxRole::CheckBox,
            control: Some(FormControlType::InputCheckbox),
        },
        sources: &[Content, AriaLabel, Labelledby],
        states: &[CheckedOrMixed, AriaDisabled, Invalid, Focus],
        nameable: true,
    },
    Kind {
        label: "role=radio",
        base: Base::Aria {
            role: "radio",
            ax: AxRole::RadioButton,
            control: Some(FormControlType::InputRadio),
        },
        sources: &[Content, AriaLabel],
        states: &[Checked, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=switch",
        base: Base::Aria {
            role: "switch",
            ax: AxRole::Switch,
            control: Some(FormControlType::InputCheckbox),
        },
        sources: &[Content, AriaLabel],
        states: &[Checked, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "textbox",
        base: Base::TextInput,
        sources: &[LabelFor, AriaLabel, Placeholder, Title, Labelledby],
        states: &[Disabled, AriaDisabled, Invalid, Focus],
        nameable: true,
    },
    Kind {
        label: "select",
        base: Base::Select,
        sources: &[LabelFor, AriaLabel, Title, Labelledby],
        states: &[Expanded, Disabled, AriaDisabled, Invalid, Focus],
        nameable: true,
    },
    Kind {
        label: "select multiple",
        base: Base::SelectListbox,
        sources: &[LabelFor, AriaLabel],
        states: &[Disabled, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=listbox",
        base: Base::AriaListbox,
        sources: &[AriaLabel, Labelledby],
        states: &[AriaDisabled],
        nameable: true,
    },
    Kind {
        label: "role=tab",
        base: Base::Aria {
            role: "tab",
            ax: AxRole::Tab,
            control: None,
        },
        sources: &[Content, AriaLabel],
        states: &[Selected, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=menuitem",
        base: Base::Aria {
            role: "menuitem",
            ax: AxRole::MenuItem,
            control: None,
        },
        sources: &[Content, AriaLabel],
        states: &[Expanded, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=menuitemcheckbox",
        base: Base::Aria {
            role: "menuitemcheckbox",
            ax: AxRole::MenuItemCheckBox,
            control: Some(FormControlType::InputCheckbox),
        },
        sources: &[Content, AriaLabel],
        states: &[CheckedOrMixed, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "role=treeitem",
        base: Base::Aria {
            role: "treeitem",
            ax: AxRole::TreeItem,
            control: None,
        },
        sources: &[Content, AriaLabel],
        states: &[Expanded, Selected, Level, AriaDisabled, Focus],
        nameable: true,
    },
    Kind {
        label: "h1-h6",
        base: Base::Heading,
        sources: &[Content, Content, AriaLabel, Labelledby],
        states: &[],
        nameable: true,
    },
    Kind {
        label: "role=heading",
        base: Base::Aria {
            role: "heading",
            ax: AxRole::Heading,
            control: None,
        },
        sources: &[Content, AriaLabel],
        states: &[Level],
        nameable: true,
    },
    Kind {
        label: "img",
        base: Base::Image,
        sources: &[Alt, Alt, AriaLabel, Title, Labelledby],
        states: &[],
        nameable: true,
    },
    Kind {
        label: "clickable div",
        base: Base::ClickableDiv,
        sources: &[Content],
        states: &[],
        nameable: false,
    },
    Kind {
        label: "role=generic",
        base: Base::Prohibited {
            tag: None,
            role: Some("generic"),
            paragraph: false,
        },
        sources: &[AriaLabel, Title, Content],
        states: &[],
        nameable: false,
    },
    Kind {
        label: "p",
        base: Base::Prohibited {
            tag: Some("p"),
            role: None,
            paragraph: true,
        },
        sources: &[AriaLabel, Content],
        states: &[],
        nameable: false,
    },
    Kind {
        label: "code",
        base: Base::Prohibited {
            tag: Some("code"),
            role: None,
            paragraph: false,
        },
        sources: &[AriaLabel, Content],
        states: &[],
        nameable: false,
    },
    Kind {
        label: "strong",
        base: Base::Prohibited {
            tag: Some("strong"),
            role: None,
            paragraph: false,
        },
        sources: &[AriaLabel],
        states: &[],
        nameable: false,
    },
    Kind {
        label: "iframe",
        base: Base::Iframe,
        sources: &[Title, AriaLabel],
        states: &[],
        nameable: true,
    },
];

/// How an element is shown, each copying a CSS or attribute effect Chromium reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visibility {
    /// An ordinary box.
    Shown,
    /// `aria-hidden="true"` on a visible box: ai mode prints it with `[aria-hidden]`.
    AriaHidden,
    /// `visibility: hidden` on the element, its text in a `visibility: visible` span.
    CssHidden,
    /// A zero-size box.
    ZeroSize,
    /// `pointer-events: none`.
    PointerEventsNone,
    /// `inert`: no clickability or focus left.
    Inert,
    /// Far below the viewport.
    BelowFold,
}

/// Facts inherited from an ancestor, written on every node Chromium writes them on.
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one contract field Chromium writes down a subtree, set independently"
)]
struct Inherit {
    aria_hidden: bool,
    aria_disabled: bool,
    inert: bool,
    pointer_events_none: bool,
}

/// What the generator knows of one element it named, keyed by its `NodeKey`.
#[derive(Clone, Debug)]
struct ElementFacts {
    /// Its row in `KINDS`, or `None` for an option inside a list box.
    kind: Option<usize>,
    /// The palette entries its name was drawn from.
    entries: Vec<usize>,
    /// Its role lets a name print.
    nameable: bool,
    /// Its name is over 900 UTF-16 units.
    over_900: bool,
}

/// What a page's printed lines can be compared with: each named element's facts and each
/// document's frame depth.
#[derive(Clone, Debug, Default)]
pub(crate) struct PageFacts {
    elements: HashMap<NodeKey, ElementFacts>,
    depths: HashMap<DocToken, usize>,
}

/// One generated page.
#[derive(Debug)]
pub(crate) struct GeneratedPage {
    /// The page as Chromium would send it.
    pub(crate) apc: AnnotatedPageContent,
    /// What the generator put into it.
    pub(crate) facts: PageFacts,
}

/// The page for `seed`: the same seed always gives the same page.
pub(crate) fn generate(seed: u64) -> Result<GeneratedPage, String> {
    let mut generator = Generator {
        rng: SplitMix64(seed),
        seed,
        docs: 0,
        facts: PageFacts::default(),
    };
    let main = generator.document(0, 0, false)?;
    let mut builder = ApcBuilder::new(&main.token)
        .contract_version(6)
        .viewport(0, 0, 800, VIEWPORT_HEIGHT)
        .main_frame_token(&main.frame_token)
        .main_url("https://zero-conflict.test/")
        .root(main.root);
    if let Some(focus) = main.focus {
        builder = builder.main_focused_node_id(focus);
    }
    Ok(GeneratedPage {
        apc: builder.build(),
        facts: generator.facts,
    })
}

struct Generator {
    rng: SplitMix64,
    seed: u64,
    docs: usize,
    facts: PageFacts,
}

/// A document under construction.
struct Doc {
    token: DocToken,
    depth: usize,
    next_id: i32,
    /// The top of the next box, in main-frame CSS pixels.
    y: i32,
    /// Its named elements so far, for duplicates, case pairs and prefixes.
    named: Vec<Named>,
    focus: Option<i32>,
    /// Spans `aria-labelledby` points at, appended to the body at the end.
    referents: Vec<NodeBuilder>,
    /// The document is inside an inert iframe, which makes all of it inert.
    inert: bool,
}

#[derive(Clone)]
struct Named {
    kind: usize,
    source: Source,
    name: String,
}

impl Doc {
    /// What every element of the document inherits from outside it.
    fn inherit(&self) -> Inherit {
        Inherit {
            inert: self.inert,
            ..Inherit::default()
        }
    }

    fn id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A box of `height` at the next position, zero-size when `zero`.
    fn slot(&mut self, height: i32, zero: bool) -> Rect {
        let rect = Rect {
            x: 8,
            y: self.y,
            width: if zero { 0 } else { 160 },
            height: if zero { 0 } else { height },
        };
        self.y += height + 4;
        rect
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    /// The box from this one's top down to `bottom`, keeping a zero-size box zero-size.
    fn down_to(self, bottom: i32) -> Self {
        if self.width == 0 {
            return self;
        }
        Self {
            height: (bottom - self.y).max(self.height),
            ..self
        }
    }
}

/// `node` with `rect` as its outer and visible box.
fn at(node: NodeBuilder, rect: Rect) -> NodeBuilder {
    node.bounding_box(rect.x, rect.y, rect.width, rect.height)
}

/// A built document: its root and what its frame records.
struct BuiltDoc {
    token: String,
    frame_token: String,
    root: NodeBuilder,
    focus: Option<i32>,
    bottom: i32,
}

/// A drawn name and the palette entries it came from.
struct Drawn {
    name: String,
    entries: Vec<usize>,
}

/// Everything drawn for one element before its nodes are built.
struct Plan {
    kind: usize,
    source: Source,
    drawn: Drawn,
    visibility: Visibility,
    /// Each state with the number that picks its value.
    states: Vec<(StateKind, usize)>,
}

impl Generator {
    /// A document at frame `depth` whose boxes start at `top`, all of it inert when `inert`.
    fn document(&mut self, depth: usize, top: i32, inert: bool) -> Result<BuiltDoc, String> {
        let index = self.docs;
        self.docs += 1;
        let token = format!("doc-{:016x}-{index}", self.seed);
        let frame_token = format!("frame-{:016x}-{index}", self.seed);
        let doc_token = DocToken::new(&token).ok_or("a generated token is never empty")?;
        self.facts.depths.insert(doc_token.clone(), depth);
        let mut doc = Doc {
            token: doc_token,
            depth,
            next_id: 1,
            y: top + 8,
            named: Vec::new(),
            focus: None,
            referents: Vec::new(),
            inert,
        };
        let root_id = doc.id();
        let html_id = doc.id();
        let body_id = doc.id();
        let count = if depth == 0 {
            self.rng.between(4, 14)
        } else {
            self.rng.between(2, 6)
        };
        let mut body_nodes = Vec::new();
        let mut made = 0;
        while made < count {
            let group = self.rng.between(1, 3).min(count - made);
            made += group;
            body_nodes.extend(self.group(&mut doc, group)?);
        }
        // One kind in 28 is an iframe, too rare to nest four deep; this gives each document an
        // even chance of a child.
        if depth < MAX_FRAME_DEPTH && self.rng.chance(50) {
            let iframe = KINDS
                .iter()
                .position(|kind| matches!(kind.base, Base::Iframe))
                .ok_or("KINDS has an iframe row")?;
            body_nodes.extend(self.element(&mut doc, Some(iframe))?);
        }
        body_nodes.append(&mut doc.referents);
        let height = (doc.y - top).max(VIEWPORT_HEIGHT);
        let page = Rect {
            x: 0,
            y: top,
            width: 800,
            height,
        };
        let inherit = doc.inherit();
        let body = mark(
            at(NodeBuilder::element("body").dom_id(body_id), page),
            inherit,
        );
        let html = mark(
            at(NodeBuilder::element("html").dom_id(html_id), page),
            inherit,
        );
        let html = html.child(body.children(body_nodes));
        let root = at(NodeBuilder::root().dom_id(root_id), page).child(html);
        Ok(BuiltDoc {
            token,
            frame_token,
            root,
            focus: doc.focus,
            bottom: doc.y,
        })
    }

    /// `count` elements, alone or inside a wrapper: a `<div>`, a list, or a landmark.
    fn group(&mut self, doc: &mut Doc, count: usize) -> Result<Vec<NodeBuilder>, String> {
        let top = doc.y;
        let wrapper = self.rng.below(8);
        let mut elements = Vec::new();
        for _ in 0..count {
            let nodes = self.element(doc, None)?;
            if wrapper == 1 {
                let id = doc.id();
                let item = mark(NodeBuilder::element("li").dom_id(id), doc.inherit());
                elements.push(item.children(nodes));
            } else {
                elements.extend(nodes);
            }
        }
        let rect = Rect {
            x: 0,
            y: top,
            width: 800,
            height: doc.y - top,
        };
        let wrapped = match wrapper {
            0 => NodeBuilder::container(),
            1 => NodeBuilder::element("ul"),
            2 => NodeBuilder::element("main"),
            3 => NodeBuilder::element("nav"),
            4 => {
                let label = self.draw_fragments()?;
                NodeBuilder::element("section").aria_label(&label.name)
            }
            _ => return Ok(elements),
        };
        let id = doc.id();
        let wrapped = mark(at(wrapped.dom_id(id), rect), doc.inherit());
        Ok(vec![wrapped.children(elements)])
    }

    /// One element, of `forced` kind when given, and the nodes that come with it (its
    /// `<label for>`), as children of the element's parent.
    fn element(
        &mut self,
        doc: &mut Doc,
        forced: Option<usize>,
    ) -> Result<Vec<NodeBuilder>, String> {
        let plan = self.plan(doc, forced)?;
        let kind = KINDS.get(plan.kind).ok_or("kind index in range")?;
        let mut inherit = doc.inherit();
        match plan.visibility {
            Visibility::AriaHidden => inherit.aria_hidden = true,
            Visibility::Inert => inherit.inert = true,
            Visibility::PointerEventsNone => inherit.pointer_events_none = true,
            Visibility::BelowFold => doc.y += 1_600,
            Visibility::Shown | Visibility::CssHidden | Visibility::ZeroSize => {}
        }
        if plan.states.iter().any(|&(state, _)| state == AriaDisabled) {
            inherit.aria_disabled = true;
        }
        let id = doc.id();
        let mut nodes = Vec::new();
        if plan.source == LabelFor {
            let label_id = doc.id();
            let rect = doc.slot(16, false);
            let content = self.content(doc, &plan.drawn.name, rect, doc.inherit());
            let label = mark(
                at(NodeBuilder::element("label").dom_id(label_id), rect),
                doc.inherit(),
            );
            nodes.push(label.label_for(id).children(content));
        }
        let rect = doc.slot(20, plan.visibility == Visibility::ZeroSize);
        let (mut node, rect) = self.base(doc, kind, &plan, rect, inherit)?;
        node = self.name_source(doc, node, plan.source, &plan.drawn.name);
        for &(state, pick) in &plan.states {
            node = apply_state(doc, node, kind, state, pick, id, inherit);
        }
        node = mark(at(node.dom_id(id), rect), inherit);
        if plan.visibility == Visibility::CssHidden {
            node = node.visibility_hidden(true);
        }
        let over_900 = plan.drawn.entries.iter().any(|&entry| {
            matches!(
                PALETTE.get(entry).map(|entry| entry.text),
                Some(Text::Long { units, .. }) if units > 900
            )
        });
        self.facts.elements.insert(
            NodeKey::new(doc.token.clone(), DomNodeId::new(id)),
            ElementFacts {
                kind: Some(plan.kind),
                entries: plan.drawn.entries,
                nameable: kind.nameable,
                over_900,
            },
        );
        doc.named.push(Named {
            kind: plan.kind,
            source: plan.source,
            name: plan.drawn.name,
        });
        nodes.push(node);
        Ok(nodes)
    }

    /// The draws for one element: a duplicate of an earlier one now and then, else a kind
    /// (never an iframe past the deepest frame), a source, a name, a visibility and states.
    fn plan(&mut self, doc: &Doc, forced: Option<usize>) -> Result<Plan, String> {
        let duplicate = if forced.is_none() && self.rng.chance(6) {
            self.rng.pick(&doc.named).cloned()
        } else {
            None
        };
        let (kind_index, source, drawn) = if let Some(named) = duplicate {
            let entry = entry_index(|text| matches!(text, Text::Duplicate))?;
            let drawn = Drawn {
                name: named.name,
                entries: vec![entry],
            };
            (named.kind, named.source, drawn)
        } else {
            let mut kind_index = forced.unwrap_or_else(|| self.rng.below(KINDS.len()));
            if matches!(
                KINDS.get(kind_index).map(|kind| kind.base),
                Some(Base::Iframe)
            ) && doc.depth >= MAX_FRAME_DEPTH
            {
                kind_index = 0;
            }
            let kind = KINDS.get(kind_index).ok_or("kind index in range")?;
            let source = *self
                .rng
                .pick(kind.sources)
                .ok_or("every kind has a source")?;
            (kind_index, source, self.draw_name(doc)?)
        };
        let kind = KINDS.get(kind_index).ok_or("kind index in range")?;
        let visibility = self.visibility(kind, source, &drawn.name);
        let states = self.states(kind);
        Ok(Plan {
            kind: kind_index,
            source,
            drawn,
            visibility,
            states,
        })
    }

    fn visibility(&mut self, kind: &Kind, source: Source, name: &str) -> Visibility {
        let roll = self.rng.below(100);
        let visibility = match roll {
            0..=4 => Visibility::AriaHidden,
            5..=8 => Visibility::CssHidden,
            9..=12 => Visibility::ZeroSize,
            13..=15 => Visibility::PointerEventsNone,
            16..=19 => Visibility::Inert,
            20..=22 => Visibility::BelowFold,
            _ => Visibility::Shown,
        };
        // Chromium reports a `visibility: hidden` element only for a visible descendant, and
        // only content can be one. A zero-size iframe would lay its document out in a zero-size
        // viewport, which these boxes do not model, so an iframe keeps its size.
        let iframe = matches!(kind.base, Base::Iframe);
        let hides_content = source == Content && !name.is_empty() && !iframe;
        match visibility {
            Visibility::CssHidden if !hides_content => Visibility::Shown,
            Visibility::ZeroSize if iframe => Visibility::Shown,
            visibility => visibility,
        }
    }

    /// The states drawn for `kind`: each of its own a third of the time, and now and then one it
    /// does not take, as a page can write any ARIA attribute on any element.
    fn states(&mut self, kind: &Kind) -> Vec<(StateKind, usize)> {
        let mut states = Vec::new();
        for &state in kind.states {
            if self.rng.chance(35) {
                states.push((state, self.rng.below(60)));
            }
        }
        if self.rng.chance(10)
            && let Some(&stray) = self.rng.pick(&ALL_STATES)
            && !kind.states.contains(&stray)
        {
            states.push((stray, self.rng.below(60)));
        }
        states
    }

    /// The node of `kind`, with its children, before its name source and states, and its box:
    /// `rect` grown to cover what it holds (an iframe's document, a list box's options).
    fn base(
        &mut self,
        doc: &mut Doc,
        kind: &Kind,
        plan: &Plan,
        rect: Rect,
        inherit: Inherit,
    ) -> Result<(NodeBuilder, Rect), String> {
        let name = plan.drawn.name.as_str();
        // Content that names the element, or, under a label that wins over content, decoy text.
        let content = match plan.source {
            Content => Some(name.to_owned()),
            AriaLabel | Labelledby if self.rng.chance(30) => Some(self.draw_fragments()?.name),
            _ => None,
        };
        let mut children = content
            .map(|text| self.content(doc, &text, rect, inherit))
            .unwrap_or_default();
        if plan.visibility == Visibility::CssHidden {
            let id = doc.id();
            let span = mark(at(NodeBuilder::container().dom_id(id), rect), inherit);
            children = vec![span.children(children)];
        }
        let clickable = &[ClickabilityReason::ClickableControl];
        let node = match kind.base {
            Base::Button => {
                let node = NodeBuilder::form_control(FormControlType::ButtonButton).tag("button");
                interactive(node, clickable, inherit).children(children)
            }
            Base::InputButton => {
                let node = NodeBuilder::form_control(FormControlType::InputButton)
                    .tag("input")
                    .break_before(true);
                let node = interactive(node, clickable, inherit);
                if plan.source == Value {
                    with_value(doc, node, name, rect, inherit)
                } else {
                    node
                }
            }
            Base::Link => {
                let url = format!("https://zero-conflict.test/{}", doc.next_id);
                let node = NodeBuilder::anchor(&url).tag("a").href(&url);
                interactive(node, &[ClickabilityReason::CursorPointer], inherit).children(children)
            }
            Base::NativeToggle { control, role } => {
                let mut node = NodeBuilder::form_control(control)
                    .tag("input")
                    .break_before(true)
                    .checked_state(BrowserOsTriState::False)
                    .checked(false);
                if let Some(role) = role {
                    node = node.role(role);
                }
                interactive(node, clickable, inherit)
            }
            Base::Aria { role, ax, control } => {
                let node = control.map_or_else(NodeBuilder::container, NodeBuilder::form_control);
                let reasons = [ClickabilityReason::AriaRole, ClickabilityReason::TabIndex];
                interactive(node.role(role).aria_role(ax), &reasons, inherit).children(children)
            }
            Base::TextInput => self.text_input(doc, rect, inherit)?,
            Base::Select => self.select(doc, rect, inherit)?,
            Base::SelectListbox => {
                let node = NodeBuilder::form_control(FormControlType::SelectMultiple)
                    .tag("select")
                    .list_box(true);
                let node = interactive(node, clickable, inherit);
                self.list_box(doc, node, Options::Native, inherit)?
            }
            Base::AriaListbox => {
                let node = NodeBuilder::container()
                    .role("listbox")
                    .aria_role(AxRole::ListBox)
                    .clickability_reason(ClickabilityReason::AriaRole);
                self.list_box(doc, node, Options::Aria, inherit)?
            }
            Base::Heading => {
                let level = self.rng.between(1, 6);
                let tag = format!("h{level}");
                NodeBuilder::heading().tag(&tag).children(children)
            }
            Base::Image => NodeBuilder::image().tag("img"),
            Base::ClickableDiv => NodeBuilder::container()
                .clickability_reason(ClickabilityReason::CursorPointer)
                .children(children),
            Base::Prohibited {
                tag,
                role,
                paragraph,
            } => {
                let mut node = if paragraph {
                    NodeBuilder::paragraph()
                } else {
                    NodeBuilder::container()
                };
                if let Some(tag) = tag {
                    node = node.tag(tag);
                }
                if let Some(role) = role {
                    node = node.role(role);
                }
                node.children(children)
            }
            Base::Iframe => self.iframe(doc, rect, inherit)?,
        };
        Ok((node, rect.down_to(doc.y - 4)))
    }

    /// `<input type=text>`: its editor is a user-agent shadow container, which holds the value
    /// a third of the time.
    fn text_input(
        &mut self,
        doc: &mut Doc,
        rect: Rect,
        inherit: Inherit,
    ) -> Result<NodeBuilder, String> {
        let shadow_id = doc.id();
        let mut shadow = at(NodeBuilder::container().dom_id(shadow_id), rect)
            .ua_shadow(true)
            .clickability_reason(ClickabilityReason::Editable);
        let node = NodeBuilder::form_control(FormControlType::InputText)
            .tag("input")
            .break_before(true);
        let mut node = interactive(node, &[ClickabilityReason::ClickableControl], inherit);
        if self.rng.chance(30) {
            let value = self.draw_fragments()?.name;
            if !value.is_empty() {
                let id = doc.id();
                let text = mark_text(NodeBuilder::text(&value).ua_shadow(true), inherit);
                shadow = shadow.child(at(text.dom_id(id), rect));
            }
            node = node.field_value(&value);
        }
        Ok(node.child(shadow))
    }

    /// `<select>`: its options as form-control data, the selected one's text in a user-agent
    /// shadow container.
    fn select(
        &mut self,
        doc: &mut Doc,
        rect: Rect,
        inherit: Inherit,
    ) -> Result<NodeBuilder, String> {
        let options = self.options()?;
        let node = NodeBuilder::form_control(FormControlType::SelectOne).tag("select");
        let mut node = interactive(node, &[ClickabilityReason::ClickableControl], inherit);
        let mut selected = String::new();
        for (drawn, is_selected) in &options {
            node = node.select_option(select_option(&drawn.name, *is_selected));
            if *is_selected {
                selected.clone_from(&drawn.name);
            }
        }
        let shadow_id = doc.id();
        let text_id = doc.id();
        let text = mark_text(NodeBuilder::text(&selected).ua_shadow(true), inherit);
        let shadow = at(NodeBuilder::container().dom_id(shadow_id), rect).ua_shadow(true);
        Ok(node.child(shadow.child(at(text.dom_id(text_id), rect))))
    }

    /// A list box `node` with option children, one of them selected. A native list box also
    /// lists its options as form-control data.
    fn list_box(
        &mut self,
        doc: &mut Doc,
        node: NodeBuilder,
        options: Options,
        inherit: Inherit,
    ) -> Result<NodeBuilder, String> {
        let mut node = node;
        for (drawn, is_selected) in self.options()? {
            let mut option = match options {
                Options::Native => {
                    node = node.select_option(select_option(&drawn.name, is_selected));
                    NodeBuilder::element("option")
                }
                Options::Aria => NodeBuilder::container()
                    .role("option")
                    .aria_role(AxRole::ListBoxOption),
            };
            if is_selected {
                option = option.selected(true);
            }
            node = node.child(self.option(doc, option, drawn, inherit));
        }
        Ok(node)
    }

    /// `<iframe>` and its document one frame deeper, which pushes the parent's next box below it.
    fn iframe(
        &mut self,
        doc: &mut Doc,
        rect: Rect,
        inherit: Inherit,
    ) -> Result<NodeBuilder, String> {
        // Inertness reaches into the frame's document; CSS and ARIA inheritance do not.
        let child = self.document(doc.depth + 1, rect.y, inherit.inert)?;
        let url = format!("https://frame-{}.zero-conflict.test/", child.token);
        let mut frame = FrameBuilder::new(&child.token)
            .frame_token(&child.frame_token)
            .url(&url)
            .root(child.root);
        if let Some(focus) = child.focus {
            frame = frame.focused_node_id(focus);
        }
        doc.y = doc.y.max(child.bottom + 4);
        Ok(NodeBuilder::iframe(frame)
            .tag("iframe")
            .focusable(!inherit.inert))
    }

    /// Writes `name` into `node` as `source` carries it. Content, a label and a value were
    /// written with the node's children.
    fn name_source(
        &mut self,
        doc: &mut Doc,
        node: NodeBuilder,
        source: Source,
        name: &str,
    ) -> NodeBuilder {
        match source {
            AriaLabel => node.aria_label(name),
            // `title` is written only when it is not blank.
            Title if name.trim_matches(HTML_SPACE).is_empty() => node,
            Title => node.title(name),
            Alt => node.alt(name),
            Placeholder => node.placeholder(name),
            Labelledby => {
                let refs = self.referents(doc, name);
                node.labelledby(refs)
            }
            Content | LabelFor | Value => node,
        }
    }

    /// The `aria-labelledby` targets that spell `name`: one or two spans appended to the body,
    /// or, for a referent that is not rendered, the text Chromium carries instead of a node.
    fn referents(&mut self, doc: &mut Doc, name: &str) -> Vec<BrowserOsRef> {
        if self.rng.chance(15) {
            return vec![BrowserOsRef {
                dom_node_id: None,
                unrendered_text: Some(name.to_owned()),
            }];
        }
        let pieces = if self.rng.chance(30) {
            split_at_char(name, self.rng.below(name.chars().count() + 1))
        } else {
            vec![name.to_owned()]
        };
        let mut refs = Vec::new();
        for piece in pieces {
            let id = doc.id();
            let rect = doc.slot(16, false);
            let content = self.content(doc, &piece, rect, doc.inherit());
            let span = mark(at(NodeBuilder::container().dom_id(id), rect), doc.inherit());
            doc.referents.push(span.children(content));
            refs.push(BrowserOsRef {
                dom_node_id: Some(id),
                unrendered_text: None,
            });
        }
        refs
    }

    /// `text` as an element's content: one to three text nodes, some wrapped in an inline
    /// element, some after a block break. Empty text renders no node.
    fn content(
        &mut self,
        doc: &mut Doc,
        text: &str,
        rect: Rect,
        inherit: Inherit,
    ) -> Vec<NodeBuilder> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut pieces = vec![text.to_owned()];
        if self.rng.chance(40) {
            let chars = text.chars().count();
            pieces = split_at_char(text, self.rng.below(chars + 1));
            if self.rng.chance(40)
                && let Some(last) = pieces.pop()
            {
                let at = self.rng.below(last.chars().count() + 1);
                pieces.extend(split_at_char(&last, at));
            }
        }
        let mut nodes = Vec::new();
        for piece in pieces.into_iter().filter(|piece| !piece.is_empty()) {
            let text_id = doc.id();
            let mut node = at(
                mark_text(NodeBuilder::text(&piece), inherit).dom_id(text_id),
                rect,
            );
            if self.rng.chance(25) {
                node = node.break_before(true);
            }
            if self.rng.chance(25) {
                let id = doc.id();
                let wrapper = if self.rng.chance(50) {
                    NodeBuilder::container()
                } else {
                    NodeBuilder::element("b")
                };
                node = mark(at(wrapper.dom_id(id), rect), inherit).child(node);
            }
            nodes.push(node);
        }
        nodes
    }

    /// Two to four option names, one of them selected.
    fn options(&mut self) -> Result<Vec<(Drawn, bool)>, String> {
        let count = self.rng.between(2, 4);
        let selected = self.rng.below(count);
        let mut options = Vec::new();
        for index in 0..count {
            options.push((self.draw_fragments()?, index == selected));
        }
        Ok(options)
    }

    /// An option node named by its text content, recorded as a named element.
    fn option(
        &mut self,
        doc: &mut Doc,
        node: NodeBuilder,
        drawn: Drawn,
        inherit: Inherit,
    ) -> NodeBuilder {
        let id = doc.id();
        let rect = doc.slot(16, false);
        let content = self.content(doc, &drawn.name, rect, inherit);
        self.facts.elements.insert(
            NodeKey::new(doc.token.clone(), DomNodeId::new(id)),
            ElementFacts {
                kind: None,
                entries: drawn.entries,
                nameable: true,
                over_900: false,
            },
        );
        mark(at(node.dom_id(id), rect), inherit).children(content)
    }

    /// A name: a whole entry, a relation to an earlier name, or one to three fragments.
    fn draw_name(&mut self, doc: &Doc) -> Result<Drawn, String> {
        let roll = self.rng.below(100);
        if roll < 10 {
            let wholes: Vec<usize> = (0..PALETTE.len())
                .filter(|&index| {
                    matches!(
                        PALETTE.get(index).map(|entry| entry.text),
                        Some(Text::Whole(_) | Text::Long { .. })
                    )
                })
                .collect();
            let index = *self
                .rng
                .pick(&wholes)
                .ok_or("the palette has whole names")?;
            let name = match PALETTE.get(index).map(|entry| entry.text) {
                Some(Text::Whole(text)) => text.to_owned(),
                Some(Text::Long { units, emoji }) => long_name(units, emoji),
                _ => String::new(),
            };
            return Ok(Drawn {
                name,
                entries: vec![index],
            });
        }
        let earlier: Vec<&Named> = doc
            .named
            .iter()
            .filter(|named| named.name.bytes().any(|byte| byte.is_ascii_alphabetic()))
            .filter(|named| named.name.len() < 200)
            .collect();
        if roll < 18
            && let Some(named) = self.rng.pick(&earlier)
        {
            let name = named
                .name
                .chars()
                .map(|character| {
                    if character.is_ascii_uppercase() {
                        character.to_ascii_lowercase()
                    } else {
                        character.to_ascii_uppercase()
                    }
                })
                .collect();
            return Ok(Drawn {
                name,
                entries: vec![entry_index(|text| matches!(text, Text::CaseOf))?],
            });
        }
        if roll < 26
            && let Some(named) = self.rng.pick(&earlier)
        {
            let words = if self.rng.chance(50) {
                " all"
            } else {
                " all changes"
            };
            return Ok(Drawn {
                name: format!("{}{words}", named.name),
                entries: vec![entry_index(|text| matches!(text, Text::PrefixOf))?],
            });
        }
        self.draw_fragments()
    }

    /// One to three fragments, joined with nothing or a space.
    fn draw_fragments(&mut self) -> Result<Drawn, String> {
        let fragments: Vec<usize> = (0..PALETTE.len())
            .filter(|&index| {
                matches!(
                    PALETTE.get(index).map(|entry| entry.text),
                    Some(Text::Fragment(_))
                )
            })
            .collect();
        let count = self.rng.between(1, 3);
        let joiner = if self.rng.chance(50) { "" } else { " " };
        let mut name = String::new();
        let mut entries = Vec::new();
        for part in 0..count {
            let index = *self
                .rng
                .pick(&fragments)
                .ok_or("the palette has fragments")?;
            if let Some(Text::Fragment(text)) = PALETTE.get(index).map(|entry| entry.text) {
                if part > 0 {
                    name.push_str(joiner);
                }
                name.push_str(text);
                entries.push(index);
            }
        }
        Ok(Drawn { name, entries })
    }
}

/// HTML's ASCII whitespace, the set Blink trims a `title` with.
const HTML_SPACE: &[char] = &[' ', '\t', '\n', '\u{c}', '\r'];

/// `text` cut before its `at`-th character.
fn split_at_char(text: &str, at: usize) -> Vec<String> {
    let cut = text
        .char_indices()
        .nth(at)
        .map_or(text.len(), |(index, _)| index);
    let (head, tail) = text.split_at(cut);
    vec![head.to_owned(), tail.to_owned()]
}

fn entry_index(test: impl Fn(Text) -> bool) -> Result<usize, String> {
    PALETTE
        .iter()
        .position(|entry| test(entry.text))
        .ok_or_else(|| "the palette lacks a relation entry".to_owned())
}

fn select_option(text: &str, selected: bool) -> SelectOption {
    SelectOption {
        value: Some(text.to_owned()),
        text: Some(text.to_owned()),
        is_selected: Some(selected),
        is_disabled: Some(false),
    }
}

/// Writes the inherited facts Chromium puts on an element node.
fn mark(node: NodeBuilder, inherit: Inherit) -> NodeBuilder {
    let mut node = node;
    if inherit.aria_hidden {
        node = node.aria_hidden(true);
    }
    if inherit.aria_disabled {
        node = node.aria_disabled(true);
    }
    if inherit.inert {
        node = node.inert(true);
    }
    if inherit.pointer_events_none {
        node = node.pointer_events_none(true);
    }
    node
}

/// Writes the inherited facts Chromium puts on a text node: only `aria_hidden`.
fn mark_text(node: NodeBuilder, inherit: Inherit) -> NodeBuilder {
    if inherit.aria_hidden {
        node.aria_hidden(true)
    } else {
        node
    }
}

/// Which options a list box holds.
#[derive(Clone, Copy, Debug)]
enum Options {
    /// `<option>` elements of a `<select multiple>`.
    Native,
    /// `role=option` elements of a `role=listbox`.
    Aria,
}

/// A control that takes clicks and focus, unless it is inert, which takes both away.
fn interactive(node: NodeBuilder, reasons: &[ClickabilityReason], inherit: Inherit) -> NodeBuilder {
    if inherit.inert {
        return node;
    }
    let mut node = node.focusable(true);
    for &reason in reasons {
        node = node.clickability_reason(reason);
    }
    node
}

/// A button input's `value`: in its form-control data, and as the user-agent shadow text it
/// renders, which no empty value has.
fn with_value(
    doc: &mut Doc,
    node: NodeBuilder,
    value: &str,
    rect: Rect,
    inherit: Inherit,
) -> NodeBuilder {
    let node = node.field_value(value);
    if value.is_empty() {
        return node;
    }
    let id = doc.id();
    let text = mark_text(NodeBuilder::text(value).ua_shadow(true), inherit);
    node.child(at(text.dom_id(id), rect))
}

/// Writes `state`, its value chosen by `pick`, as Chromium writes it.
fn apply_state(
    doc: &mut Doc,
    node: NodeBuilder,
    kind: &Kind,
    state: StateKind,
    pick: usize,
    id: i32,
    inherit: Inherit,
) -> NodeBuilder {
    let tri = |choices: &[BrowserOsTriState]| {
        choices
            .get(pick % choices.len().max(1))
            .copied()
            .unwrap_or(BrowserOsTriState::True)
    };
    let control = matches!(
        kind.base,
        Base::NativeToggle { .. }
            | Base::Aria {
                control: Some(_),
                ..
            }
    );
    match state {
        Checked | CheckedOrMixed => {
            let choices: &[BrowserOsTriState] = if state == Checked {
                &[BrowserOsTriState::True, BrowserOsTriState::False]
            } else {
                &[
                    BrowserOsTriState::True,
                    BrowserOsTriState::Mixed,
                    BrowserOsTriState::False,
                ]
            };
            let value = tri(choices);
            let node = node.checked_state(value);
            if control {
                node.checked(value == BrowserOsTriState::True)
            } else {
                node
            }
        }
        Pressed => node.pressed_state(tri(&[
            BrowserOsTriState::True,
            BrowserOsTriState::Mixed,
            BrowserOsTriState::False,
        ])),
        Expanded => node.expanded_state(tri(&[BrowserOsTriState::True, BrowserOsTriState::False])),
        Selected => node.selected(true),
        Level => node.level(u32::try_from(pick % 6 + 1).unwrap_or(1)),
        Disabled => node.contract_disabled(true).disabled(true),
        AriaDisabled => node.aria_disabled(true),
        Invalid => {
            let values = ["true", "grammar", "spelling", "false", ""];
            node.aria_invalid(values.get(pick % values.len()).copied().unwrap_or("true"))
        }
        Focus => {
            if doc.focus.is_none() && !inherit.inert {
                doc.focus = Some(id);
            }
            node
        }
    }
}

/// The marks a printed line can carry, as coverage counts them: each state mark, `[level=1]` to
/// `[level=6]` apart, and `[cursor=pointer]`.
const MARKS: [&str; 19] = [
    "checked",
    "checked=mixed",
    "disabled",
    "expanded",
    "active",
    "invalid",
    "invalid=grammar",
    "invalid=spelling",
    "level=1",
    "level=2",
    "level=3",
    "level=4",
    "level=5",
    "level=6",
    "pressed",
    "pressed=mixed",
    "selected",
    "aria-hidden",
    "cursor=pointer",
];

/// What the printed ref lines of a run exercised, so a generator that silently stopped making a
/// shape fails the gate instead of passing it vacuously.
#[derive(Clone, Debug)]
pub(crate) struct Coverage {
    marks: [usize; MARKS.len()],
    depths: [usize; MAX_FRAME_DEPTH + 1],
    over_900: usize,
    quoted_keys: usize,
    kinds: Vec<usize>,
    palette: Vec<usize>,
}

impl Default for Coverage {
    fn default() -> Self {
        Self {
            marks: [0; MARKS.len()],
            depths: [0; MAX_FRAME_DEPTH + 1],
            over_900: 0,
            quoted_keys: 0,
            kinds: vec![0; KINDS.len()],
            palette: vec![0; PALETTE.len()],
        }
    }
}

impl Coverage {
    /// Counts every printed ref line of `check` against what `facts` says the generator put
    /// there.
    pub(crate) fn count(&mut self, facts: &PageFacts, check: &PageCheck) {
        for line_check in &check.lines {
            let line = &line_check.line;
            let marks = &line.marks;
            let level = marks.level.map(|level| format!("level={level}"));
            let present = [
                marks.checked == Some(Mixable::True),
                marks.checked == Some(Mixable::Mixed),
                marks.disabled,
                marks.expanded,
                marks.active,
                marks.invalid == Some(InvalidMark::True),
                marks.invalid == Some(InvalidMark::Grammar),
                marks.invalid == Some(InvalidMark::Spelling),
                level.as_deref() == Some("level=1"),
                level.as_deref() == Some("level=2"),
                level.as_deref() == Some("level=3"),
                level.as_deref() == Some("level=4"),
                level.as_deref() == Some("level=5"),
                level.as_deref() == Some("level=6"),
                marks.pressed == Some(Mixable::True),
                marks.pressed == Some(Mixable::Mixed),
                marks.selected,
                marks.aria_hidden,
                line.text.contains("[cursor=pointer]"),
            ];
            for (count, present) in self.marks.iter_mut().zip(present) {
                *count += usize::from(present);
            }
            if line.text.trim_start_matches(' ').starts_with("- '") {
                self.quoted_keys += 1;
            }
            let Some(target) = &line_check.target else {
                continue;
            };
            if let Some(&depth) = facts.depths.get(&target.doc)
                && let Some(count) = self.depths.get_mut(depth)
            {
                *count += 1;
            }
            let Some(element) = facts.elements.get(target) else {
                continue;
            };
            if let Some(count) = element.kind.and_then(|kind| self.kinds.get_mut(kind)) {
                *count += 1;
            }
            if element.over_900 && element.nameable && line.name.is_none() {
                self.over_900 += 1;
            }
            for &entry in &element.entries {
                let survived = match PALETTE.get(entry).map(|entry| entry.printed) {
                    Some(Printed::Name) => line.name.is_some(),
                    Some(Printed::NoName) => element.nameable && line.name.is_none(),
                    None => false,
                };
                if survived && let Some(count) = self.palette.get_mut(entry) {
                    *count += 1;
                }
            }
        }
    }

    /// Adds `other`'s counts to these.
    pub(crate) fn merge(&mut self, other: &Self) {
        for (count, more) in self.marks.iter_mut().zip(other.marks) {
            *count += more;
        }
        for (count, more) in self.depths.iter_mut().zip(other.depths) {
            *count += more;
        }
        self.over_900 += other.over_900;
        self.quoted_keys += other.quoted_keys;
        for (count, more) in self.kinds.iter_mut().zip(&other.kinds) {
            *count += more;
        }
        for (count, more) in self.palette.iter_mut().zip(&other.palette) {
            *count += more;
        }
    }

    /// Every counter, labelled, in a fixed order.
    pub(crate) fn counters(&self) -> Vec<(String, usize)> {
        let mut counters: Vec<(String, usize)> = MARKS
            .iter()
            .zip(self.marks)
            .map(|(mark, count)| (format!("[{mark}]"), count))
            .collect();
        counters.extend(
            self.depths
                .iter()
                .enumerate()
                .map(|(depth, &count)| (format!("frame depth {depth}"), count)),
        );
        counters.push(("name over 900 units, unprinted".to_owned(), self.over_900));
        counters.push(("quoted YAML key".to_owned(), self.quoted_keys));
        counters.extend(
            KINDS
                .iter()
                .zip(&self.kinds)
                .map(|(kind, &count)| (format!("kind {:?}", kind.label), count)),
        );
        counters.extend(
            PALETTE
                .iter()
                .zip(&self.palette)
                .map(|(entry, &count)| (format!("palette {:?}", entry.label), count)),
        );
        counters
    }
}
