//! The zero-conflict check (design 10.1): every printed line with a ref, read back the way an
//! agent reads it and turned into the `getByRole` the agent would write, must select the element
//! its ref names.
//!
//! ```text
//!  SnapshotTree + &mut TabRefTable
//!    └─ tree.aria(ai) ─▶ Snapshot ─┬─ render(Call::Page, no cap) ─▶ Neo text
//!                                  └─ render(Call::Playwright) ──▶ Playwright text
//!    each text ─▶ parse_lines ─▶ PrintedLine (the lines whose key has a [ref=…])
//!    each line ─▶ ByRole::from_line ─▶ chunk ─▶ ChunkPlan ─▶ one tree stage
//!              ─▶ TreeStage::select in the line's document ─▶ Verdict
//! ```
//!
//! Both sides read one tree and one ref table, so a miss is a disagreement on the path between
//! them: the printer's quoting and name rule, the selector's escaping, the attribute parser, name
//! matching, which states print, or the frame a line belongs to. The offline corpus test, the
//! live browser test and the site runner share this pipeline, one entry point each.
//!
//! Two printed roles are the snapshot's own labels, not roles: `generic` for an element with no
//! ARIA role, and `iframe`. No `getByRole` names those elements, in Playwright or here, so such a
//! line is counted apart as a [`Label`] once the tree confirms what the element is; its ref is
//! the only handle. Every other line must be reached.

use std::error::Error;
use std::fmt::{self, Write as _};

use neo_snapshot::{Call, MaxTokens, Passed, Snapshot, Tristate};
use neo_snapshot_apc::AnnotatedPageContent;
use neo_snapshot_apc::refs::{RefLookup, TabRefTable};
use neo_snapshot_apc::select::{
    ChunkPlan, ParsedSelector, PlannedStage, RefResolver, Scope, SelectorBody, SelectorPart,
};
use neo_snapshot_apc::tree::{
    AriaAsk, BuildOptions, DocId, NameCache, NodeKey, SnapshotTree, TreeNode,
};
use serde_json::{Deserializer, Map, Value};

/// The two texts an agent can be shown for one capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Format {
    /// Neo's default format for a page: the short list of what an agent acts on.
    Neo,
    /// Playwright's own text of the same ai-mode tree, every node a line, as
    /// `ariaSnapshot({ mode: 'ai' })` prints it under the conformance switch.
    Playwright,
}

impl Format {
    /// Both formats, in the order [`PageCheck::texts`] holds their texts.
    pub(crate) const ALL: [Self; 2] = [Self::Neo, Self::Playwright];

    /// `snapshot` as this format prints it. Neo's cap is lifted so that every line prints: a cut
    /// line is not shown to the agent, so it cannot conflict.
    fn render(self, snapshot: &Snapshot) -> Result<String, Box<dyn Error>> {
        let passed = Passed {
            max_tokens: Some(MaxTokens::new(1e9).ok_or("1e9 tokens is a valid cap")?),
            ..Passed::default()
        };
        let call = match self {
            Self::Neo => Call::Page(&passed),
            Self::Playwright => Call::Playwright,
        };
        Ok(neo_snapshot::render(snapshot, call).into_text())
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Neo => "neo",
            Self::Playwright => "playwright",
        })
    }
}

/// A two-valued printed state: `[checked]` or `[checked=mixed]`, and likewise `pressed`. A
/// `false` state never prints, so it has no variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mixable {
    /// `[checked]`, `[pressed]`.
    True,
    /// `[checked=mixed]`, `[pressed=mixed]`.
    Mixed,
}

/// `[invalid]`, `[invalid=grammar]` or `[invalid=spelling]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvalidMark {
    /// `[invalid]`.
    True,
    /// `[invalid=grammar]`.
    Grammar,
    /// `[invalid=spelling]`.
    Spelling,
}

/// The state marks of one printed key, in `createKey`'s order. `[cursor=…]` and `[box=…]` say
/// nothing `getByRole` can ask, so they are read and dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent printed mark, as the printer's own node has them"
)]
pub(crate) struct Marks {
    /// `[checked]` or `[checked=mixed]`.
    pub(crate) checked: Option<Mixable>,
    /// `[disabled]`.
    pub(crate) disabled: bool,
    /// `[expanded]`.
    pub(crate) expanded: bool,
    /// `[active]`: focus, which `getByRole` has no option for.
    pub(crate) active: bool,
    /// `[invalid…]`, which `getByRole` has no option for.
    pub(crate) invalid: Option<InvalidMark>,
    /// `[level=N]`.
    pub(crate) level: Option<u32>,
    /// `[pressed]` or `[pressed=mixed]`.
    pub(crate) pressed: Option<Mixable>,
    /// `[selected]`.
    pub(crate) selected: bool,
    /// `[aria-hidden]`: ai mode prints a box-visible element that is hidden for ARIA.
    pub(crate) aria_hidden: bool,
}

/// One printed line that carries a ref, as an agent reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrintedLine {
    /// 1-based line number in its text.
    pub(crate) number: usize,
    /// The line as printed, for reports.
    pub(crate) text: String,
    /// The key's role.
    pub(crate) role: String,
    /// The key's name, decoded from its JSON literal; `None` when the key prints none.
    pub(crate) name: Option<String>,
    /// The key's state marks.
    pub(crate) marks: Marks,
    /// The key's `[ref=…]`.
    pub(crate) r#ref: String,
    /// The `iframe` lines the line is indented under, outermost first: each one's ref, or `None`
    /// for an iframe printed without one (it takes no pointer events, so it gets no ref, while
    /// its document's elements do).
    pub(crate) frames: Vec<Option<String>>,
    /// An enclosing line of the same frame prints `[aria-hidden]`. `aria-hidden` hides a whole
    /// subtree, and the printer marks only the element that carries the attribute, as upstream's
    /// does (`ariaSnapshot.ts:138-139`), so the agent reads it from the nesting. It stops at an
    /// iframe line: hiding never crosses into a child document (`isElementHiddenForAria` walks
    /// one document, and so does the tree's hidden pass).
    pub(crate) under_aria_hidden: bool,
}

/// One list item still open while later, deeper lines are read: what its nesting tells them.
struct OpenItem {
    indent: usize,
    opens: Opens,
}

/// What an open line tells the lines nested under it.
enum Opens {
    /// An `iframe` line opens its child document, named by the iframe's ref if it printed one.
    Frame(Option<String>),
    /// Any other line: whether it prints `[aria-hidden]`, which hides everything under it.
    Element {
        /// The line prints `[aria-hidden]`.
        aria_hidden: bool,
    },
}

/// Splits `text` into lines and reads back every line whose key carries a ref. Text, `/url`,
/// `/placeholder` and comment lines carry none and are skipped. A line that holds `[ref=` but
/// does not read as a key is an error: skipping it would hide a line the agent was shown. Every
/// other list item is read too where it reads as a key, because its nesting (an iframe, an
/// `[aria-hidden]`) applies to the lines under it.
pub(crate) fn parse_lines(text: &str) -> Result<Vec<PrintedLine>, Box<dyn Error>> {
    let mut printed = Vec::new();
    let mut open: Vec<OpenItem> = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let number = index + 1;
        let body = raw.trim_start_matches(' ');
        let indent = raw.len() - body.len();
        let Some(item) = body.strip_prefix("- ") else {
            if !body.starts_with('#') && raw.contains("[ref=") {
                return Err(format!("line {number} holds a ref but is no list item: {raw}").into());
            }
            continue;
        };
        // A list item at or left of an open item's indentation closes it.
        while open.last().is_some_and(|parent| parent.indent >= indent) {
            open.pop();
        }
        if item.starts_with("text:") || item.starts_with('/') {
            continue;
        }
        let key = match read_key(item) {
            Ok(key) => key,
            Err(reason) if raw.contains("[ref=") => {
                return Err(
                    format!("line {number} does not read as a key ({reason}): {raw}").into(),
                );
            }
            Err(_) => continue,
        };
        let frames: Vec<Option<String>> = open
            .iter()
            .filter_map(|parent| match &parent.opens {
                Opens::Frame(r#ref) => Some(r#ref.clone()),
                Opens::Element { .. } => None,
            })
            .collect();
        let under_aria_hidden = open
            .iter()
            .rev()
            .take_while(|parent| !matches!(parent.opens, Opens::Frame(_)))
            .any(|parent| matches!(parent.opens, Opens::Element { aria_hidden: true }));
        let opens = if key.role == "iframe" {
            Opens::Frame(key.r#ref.clone())
        } else {
            Opens::Element {
                aria_hidden: key.marks.aria_hidden,
            }
        };
        open.push(OpenItem { indent, opens });
        if let Some(r#ref) = key.r#ref {
            printed.push(PrintedLine {
                number,
                text: raw.to_owned(),
                role: key.role,
                name: key.name,
                marks: key.marks,
                r#ref,
                frames,
                under_aria_hidden,
            });
        }
    }
    Ok(printed)
}

/// A key as read back, its ref still optional: an iframe line without one still opens a frame.
struct Key {
    role: String,
    name: Option<String>,
    marks: Marks,
    r#ref: Option<String>,
}

/// Reads the key of a list item (the text after `- `): bare, or YAML single-quoted with each `'`
/// doubled, then optionally `:` or `: VALUE`. A bare key never holds `: ` (the YAML writer quotes
/// a key that does), so the first `: ` ends it.
fn read_key(item: &str) -> Result<Key, String> {
    let key = if let Some(quoted) = item.strip_prefix('\'') {
        let mut key = String::new();
        let mut chars = quoted.char_indices();
        let after = loop {
            match chars.next() {
                Some((at, '\'')) => {
                    if quoted
                        .get(at + 1..)
                        .is_some_and(|rest| rest.starts_with('\''))
                    {
                        key.push('\'');
                        chars.next();
                    } else {
                        break quoted.get(at + 1..).unwrap_or_default();
                    }
                }
                Some((_, character)) => key.push(character),
                None => return Err("the quoted key is never closed".to_owned()),
            }
        };
        if !(after.is_empty() || after == ":" || after.starts_with(": ")) {
            return Err(format!("{after:?} follows the quoted key"));
        }
        key
    } else if let Some((key, _value)) = item.split_once(": ") {
        key.to_owned()
    } else {
        item.strip_suffix(':').unwrap_or(item).to_owned()
    };
    read_key_text(&key)
}

/// Reads `role "name" [mark]…`: the role up to the first space, a JSON string literal, then the
/// marks in any order.
fn read_key_text(key: &str) -> Result<Key, String> {
    let role_end = key.find(' ').unwrap_or(key.len());
    let (role, mut rest) = key.split_at(role_end);
    if role.is_empty() {
        return Err("the key has no role".to_owned());
    }
    let mut name = None;
    if let Some(literal) = rest
        .strip_prefix(' ')
        .filter(|literal| literal.starts_with('"'))
    {
        let mut stream = Deserializer::from_str(literal).into_iter::<String>();
        let decoded = stream
            .next()
            .ok_or("the name literal is empty")?
            .map_err(|error| format!("the name is no JSON string: {error}"))?;
        name = Some(decoded);
        rest = literal.get(stream.byte_offset()..).unwrap_or_default();
    }
    let mut marks = Marks::default();
    let mut r#ref = None;
    while !rest.is_empty() {
        let open = rest
            .strip_prefix(" [")
            .ok_or_else(|| format!("{rest:?} is no mark"))?;
        let (mark, after) = open
            .split_once(']')
            .ok_or_else(|| format!("the mark {open:?} is never closed"))?;
        rest = after;
        match mark.split_once('=') {
            None => match mark {
                "checked" => marks.checked = Some(Mixable::True),
                "disabled" => marks.disabled = true,
                "expanded" => marks.expanded = true,
                "active" => marks.active = true,
                "invalid" => marks.invalid = Some(InvalidMark::True),
                "pressed" => marks.pressed = Some(Mixable::True),
                "selected" => marks.selected = true,
                "aria-hidden" => marks.aria_hidden = true,
                _ => return Err(format!("unknown mark [{mark}]")),
            },
            Some(("checked", "mixed")) => marks.checked = Some(Mixable::Mixed),
            Some(("pressed", "mixed")) => marks.pressed = Some(Mixable::Mixed),
            Some(("invalid", "grammar")) => marks.invalid = Some(InvalidMark::Grammar),
            Some(("invalid", "spelling")) => marks.invalid = Some(InvalidMark::Spelling),
            Some(("level", level)) => {
                marks.level = Some(
                    level
                        .parse()
                        .map_err(|error| format!("[level={level}]: {error}"))?,
                );
            }
            Some(("ref", value)) if !value.is_empty() => r#ref = Some(value.to_owned()),
            Some(("cursor" | "box", _)) => {}
            Some(_) => return Err(format!("unknown mark [{mark}]")),
        }
    }
    Ok(Key {
        role: role.to_owned(),
        name,
        marks,
        r#ref,
    })
}

/// `getByRole`'s options, field for field as Playwright 1.64.1's `ByRoleOptions` has them
/// (`isomorphic/locatorUtils.ts`), without `description`. `None` is an option the script leaves
/// out; `checked` and `pressed` take `true`, `false` or `'mixed'`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ByRoleOptions {
    /// `checked`.
    pub(crate) checked: Option<Tristate>,
    /// `disabled`.
    pub(crate) disabled: Option<bool>,
    /// `selected`.
    pub(crate) selected: Option<bool>,
    /// `expanded`.
    pub(crate) expanded: Option<bool>,
    /// `includeHidden`.
    pub(crate) include_hidden: Option<bool>,
    /// `level`.
    pub(crate) level: Option<u32>,
    /// `name`, a string; a regular expression is not what a printed line gives.
    pub(crate) name: Option<String>,
    /// `exact`.
    pub(crate) exact: Option<bool>,
    /// `pressed`.
    pub(crate) pressed: Option<Tristate>,
}

/// `page.getByRole(role, options)`: the query an agent writes for a line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ByRole {
    /// The role, as the line prints it.
    pub(crate) role: String,
    /// The options.
    pub(crate) options: ByRoleOptions,
}

impl ByRole {
    /// The query for `line`: its role, its printed name matched exactly, and each printed state
    /// `getByRole` can ask for. `[aria-hidden]` on the line or on a line it is nested under in the
    /// same frame asks `includeHidden`, since `getByRole` skips an element hidden for ARIA by
    /// default and ai mode prints such an element when its box shows. `[active]` and `[invalid]`
    /// have no option, and an unprinted state is never asked as `false`: the agent cannot tell an
    /// absent state from a false one.
    pub(crate) fn from_line(line: &PrintedLine) -> Self {
        let tristate = |mark: Mixable| match mark {
            Mixable::True => Tristate::True,
            Mixable::Mixed => Tristate::Mixed,
        };
        let marks = &line.marks;
        Self {
            role: line.role.clone(),
            options: ByRoleOptions {
                checked: marks.checked.map(tristate),
                disabled: marks.disabled.then_some(true),
                selected: marks.selected.then_some(true),
                expanded: marks.expanded.then_some(true),
                include_hidden: (marks.aria_hidden || line.under_aria_hidden).then_some(true),
                level: marks.level,
                exact: line.name.as_ref().map(|_| true),
                name: line.name.clone(),
                pressed: marks.pressed.map(tristate),
            },
        }
    }

    /// `getByRoleSelector(role, options)` (`isomorphic/locatorUtils.ts:69-90`), props in its
    /// order, each value as `String(value)` prints it and the name through
    /// `escapeForAttributeSelector`.
    pub(crate) fn selector(&self) -> String {
        let options = &self.options;
        let mut selector = format!("internal:role={}", self.role);
        let mut prop = |name: &str, value: Option<String>| {
            if let Some(value) = value {
                // Writing to a `String` cannot fail.
                let _ = write!(selector, "[{name}={value}]");
            }
        };
        prop("checked", options.checked.map(tristate_text));
        prop("disabled", options.disabled.map(|value| value.to_string()));
        prop("selected", options.selected.map(|value| value.to_string()));
        prop("expanded", options.expanded.map(|value| value.to_string()));
        prop(
            "include-hidden",
            options.include_hidden.map(|value| value.to_string()),
        );
        prop("level", options.level.map(|value| value.to_string()));
        prop(
            "name",
            options
                .name
                .as_deref()
                .map(|name| escape_for_attribute_selector(name, options.exact == Some(true))),
        );
        prop("pressed", options.pressed.map(tristate_text));
        selector
    }

    /// The chunk `splitSelectorByFrame` sends for [`Self::selector`]: one `internal:role` part
    /// whose body and source are both the text after `internal:role=`, as upstream's
    /// `parseSelector` keeps a plain part's source (`isomorphic/selectorParser.ts:93`).
    pub(crate) fn chunk(&self) -> ParsedSelector {
        let selector = self.selector();
        let body = selector
            .strip_prefix("internal:role=")
            .unwrap_or(&selector)
            .to_owned();
        ParsedSelector {
            parts: vec![SelectorPart {
                name: "internal:role".to_owned(),
                source: body.clone(),
                body: SelectorBody::Text(body),
            }],
            capture: None,
        }
    }

    /// The options object exactly as a script passes it to `getByRole`, the keys left out that
    /// the script leaves out.
    pub(crate) fn script_options(&self) -> Value {
        let options = &self.options;
        let tristate = |state: Tristate| match state {
            Tristate::False => Value::Bool(false),
            Tristate::True => Value::Bool(true),
            Tristate::Mixed => Value::from("mixed"),
        };
        let mut object = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                object.insert(key.to_owned(), value);
            }
        };
        put("checked", options.checked.map(tristate));
        put("disabled", options.disabled.map(Value::Bool));
        put("selected", options.selected.map(Value::Bool));
        put("expanded", options.expanded.map(Value::Bool));
        put("includeHidden", options.include_hidden.map(Value::Bool));
        put("level", options.level.map(Value::from));
        put("name", options.name.clone().map(Value::String));
        put("exact", options.exact.map(Value::Bool));
        put("pressed", options.pressed.map(tristate));
        Value::Object(object)
    }
}

/// `String(value)` for `checked` and `pressed`.
fn tristate_text(state: Tristate) -> String {
    match state {
        Tristate::False => "false",
        Tristate::True => "true",
        Tristate::Mixed => "mixed",
    }
    .to_owned()
}

/// `escapeForAttributeSelector` for a string (`isomorphic/stringUtils.ts:118-126`): `\` then `"`
/// backslash-escaped inside double quotes, then `s` for exact or `i`.
fn escape_for_attribute_selector(value: &str, exact: bool) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"{}", if exact { 's' } else { 'i' })
}

/// A printed role that is the snapshot's own label for an element no `getByRole` names, in
/// Playwright as here. The ref is the agent's only handle on such a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    /// `generic` for an element with no ARIA role: ai mode prints `getAriaRole(element) ??
    /// 'generic'` (`ariaSnapshot.ts:249-250`), while the role engine matches `generic` only as
    /// an element's role (an explicit `role="generic"`), which such a line is checked as.
    Generic,
    /// `iframe` for a frame element, which has no role of that name.
    Iframe,
}

/// What a line's query came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The query's matches include the line's element.
    Reached {
        /// How many elements the query matched.
        matches: usize,
    },
    /// The line's role is the snapshot's label for an element no role query can name, and the
    /// tree confirms the element is that kind. Neither reached nor a miss.
    Label(Label),
    /// The query ran in the right document and its matches lack the line's element.
    Missed {
        /// How many elements the query matched instead.
        matches: usize,
    },
    /// The line's element is not in the document its indentation puts it in.
    WrongFrame,
    /// The line's ref names no live element in the table that printed it.
    RefUnresolved,
    /// An enclosing iframe line's ref names no element with a child document.
    FrameUnresolved,
    /// The query did not plan as exactly one tree stage.
    NotOneTreeStage(String),
    /// The query planned, then failed as it ran.
    StageError(String),
}

impl Verdict {
    /// Whether the line's element was reached.
    pub(crate) fn reached(&self) -> bool {
        matches!(self, Self::Reached { .. })
    }

    /// Whether the line breaks the invariant: anything but reached or a confirmed label.
    pub(crate) fn is_miss(&self) -> bool {
        !matches!(self, Self::Reached { .. } | Self::Label(_))
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reached { matches } => write!(f, "reached ({matches} matches)"),
            Self::Label(Label::Generic) => f.write_str("label: generic, no ARIA role"),
            Self::Label(Label::Iframe) => f.write_str("label: iframe"),
            Self::Missed { matches } => write!(f, "missed ({matches} other matches)"),
            Self::WrongFrame => f.write_str("wrong frame"),
            Self::RefUnresolved => f.write_str("ref unresolved"),
            Self::FrameUnresolved => f.write_str("frame unresolved"),
            Self::NotOneTreeStage(why) => write!(f, "not one tree stage: {why}"),
            Self::StageError(error) => write!(f, "stage error: {error}"),
        }
    }
}

/// One printed line, its query, and what the query came to.
#[derive(Clone, Debug)]
pub(crate) struct LineCheck {
    /// The text the line was printed in.
    pub(crate) format: Format,
    /// The line as read back.
    pub(crate) line: PrintedLine,
    /// The query's selector, `internal:role=…`.
    pub(crate) query: String,
    /// The element the line's ref resolves to, which the live test hydrates; `None` when the
    /// ref did not resolve.
    pub(crate) target: Option<NodeKey>,
    /// What the query came to.
    pub(crate) verdict: Verdict,
}

/// Every checked line of one capture, and the two texts they were read from.
#[derive(Clone, Debug)]
pub(crate) struct PageCheck {
    /// The page's name in reports: a fixture's file stem, a seed, a site.
    pub(crate) page: String,
    /// The Neo and Playwright texts, in [`Format::ALL`]'s order. The live test compares two
    /// captures of one page by them.
    pub(crate) texts: [String; 2],
    /// Each line with a ref, Neo's first, each text's in line order.
    pub(crate) lines: Vec<LineCheck>,
}

impl PageCheck {
    /// How many lines were reached.
    pub(crate) fn reached(&self) -> usize {
        self.lines
            .iter()
            .filter(|check| check.verdict.reached())
            .count()
    }

    /// How many lines were checked.
    pub(crate) fn total(&self) -> usize {
        self.lines.len()
    }

    /// How many lines print a label no role query can name.
    pub(crate) fn labels(&self) -> usize {
        self.lines
            .iter()
            .filter(|check| matches!(check.verdict, Verdict::Label(_)))
            .count()
    }

    /// The lines that break the invariant.
    pub(crate) fn misses(&self) -> impl Iterator<Item = &LineCheck> {
        self.lines.iter().filter(|check| check.verdict.is_miss())
    }
}

/// The tab's ref table as a tree stage reads it, for `aria-ref` parts (none in a `getByRole`
/// chunk, but the stage takes a resolver whatever its parts).
struct TableRefs<'a>(&'a TabRefTable);

impl RefResolver for TableRefs<'_> {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        self.0
            .resolve_str(body)
            .as_key()
            .map(|target| target.key.clone())
    }
}

/// Prints `tree` in ai mode into `refs`, renders both formats, and checks every line with a ref.
/// The live test calls it twice on one table to compare two captures of an unchanged page.
pub(crate) fn check_tree(
    page: &str,
    tree: &SnapshotTree,
    refs: &mut TabRefTable,
) -> Result<PageCheck, Box<dyn Error>> {
    let snapshot = tree.aria(AriaAsk::ai(refs))?.into_snapshot();
    let texts = [
        Format::Neo.render(&snapshot)?,
        Format::Playwright.render(&snapshot)?,
    ];
    let names = tree.name_cache();
    let resolver = TableRefs(refs);
    let query = Query {
        tree,
        names: &names,
        resolver: &resolver,
    };
    let mut lines = Vec::new();
    for (format, text) in Format::ALL.into_iter().zip(&texts) {
        let printed =
            parse_lines(text).map_err(|error| format!("{page}, {format} text: {error}"))?;
        for line in printed {
            let by_role = ByRole::from_line(&line);
            let target = match refs.resolve_str(&line.r#ref) {
                RefLookup::Key(target) => Some(target.key),
                RefLookup::Retired | RefLookup::Unknown => None,
            };
            let verdict = match &target {
                None => Verdict::RefUnresolved,
                Some(target) => query.verdict(&line, &by_role, target),
            };
            lines.push(LineCheck {
                format,
                query: by_role.selector(),
                line,
                target,
                verdict,
            });
        }
    }
    Ok(PageCheck {
        page: page.to_owned(),
        texts,
        lines,
    })
}

/// Builds `apc` as a fresh tab would and checks it.
pub(crate) fn check_apc(
    page: &str,
    apc: AnnotatedPageContent,
) -> Result<PageCheck, Box<dyn Error>> {
    let tree = SnapshotTree::build(apc, BuildOptions::new())?;
    check_tree(page, &tree, &mut TabRefTable::new())
}

/// One capture's tree and the two seams a tree stage reads, shared by every line's query.
struct Query<'a> {
    tree: &'a SnapshotTree,
    names: &'a NameCache<'a>,
    resolver: &'a TableRefs<'a>,
}

/// What one planned query selected.
struct Selected {
    /// Every match, a synthesized option without a DOM node included.
    count: usize,
    /// The matches that have a `NodeKey`.
    keys: Vec<NodeKey>,
}

impl Query<'_> {
    /// Runs `by_role` in `target`'s own document, the element's frame, once the line's printed
    /// nesting is checked against it, and compares its matches with `target`.
    fn verdict(&self, line: &PrintedLine, by_role: &ByRole, target: &NodeKey) -> Verdict {
        let doc = match frame_of(self.tree, self.resolver.0, line, target) {
            Ok(doc) => doc,
            Err(verdict) => return verdict,
        };
        if let Some(label) = label_of(self.tree, line, target) {
            return Verdict::Label(label);
        }
        let Some(scope) = Scope::document(self.tree, doc) else {
            return Verdict::FrameUnresolved;
        };
        let selected = match self.select(by_role, scope) {
            Ok(selected) => selected,
            Err(verdict) => return verdict,
        };
        if selected.keys.contains(target) {
            Verdict::Reached {
                matches: selected.count,
            }
        } else {
            Verdict::Missed {
                matches: selected.count,
            }
        }
    }

    /// What `by_role` selects from `scope`, planned by `ChunkPlan` as exactly one tree stage.
    fn select(&self, by_role: &ByRole, scope: Scope<'_>) -> Result<Selected, Verdict> {
        let plan = ChunkPlan::of(&by_role.chunk())
            .map_err(|error| Verdict::NotOneTreeStage(error.to_string()))?;
        let stages = plan
            .stages()
            .map_err(|error| Verdict::StageError(error.to_string()))?;
        let (PlannedStage::Tree(run), []) = (stages.first(), stages.rest()) else {
            return Err(Verdict::NotOneTreeStage(format!(
                "{} stages",
                stages.iter().count()
            )));
        };
        let matches = run
            .stage()
            .select(self.tree, scope, self.names, self.resolver)
            .map_err(|error| Verdict::StageError(error.to_string()))?;
        Ok(Selected {
            count: matches.len(),
            keys: matches.keys(self.tree).flatten().collect(),
        })
    }
}

/// `target`'s document, once `line`'s iframe nesting is checked against that document's chain
/// of parent documents: one iframe line per level, and each iframe line that printed a ref
/// naming that level's document. A level whose iframe printed no ref is checked by depth alone.
fn frame_of(
    tree: &SnapshotTree,
    refs: &TabRefTable,
    line: &PrintedLine,
    target: &NodeKey,
) -> Result<DocId, Verdict> {
    let mut chain = Vec::new();
    let mut document = tree
        .document_by_token(&target.doc)
        .ok_or(Verdict::WrongFrame)?;
    loop {
        chain.push(document.id());
        let Some(parent) = document.parent_doc() else {
            break;
        };
        document = tree.document(parent).ok_or(Verdict::WrongFrame)?;
    }
    chain.reverse();
    if chain.first() != Some(&DocId::MAIN) || chain.len() != line.frames.len() + 1 {
        return Err(Verdict::WrongFrame);
    }
    for (iframe, &level) in line.frames.iter().zip(chain.iter().skip(1)) {
        let Some(iframe) = iframe else {
            continue;
        };
        let named = refs
            .resolve_str(iframe)
            .as_key()
            .and_then(|frame| tree.node_by_key(&frame.key))
            .and_then(TreeNode::child_doc)
            .ok_or(Verdict::FrameUnresolved)?;
        if named != level {
            return Err(Verdict::WrongFrame);
        }
    }
    chain.last().copied().ok_or(Verdict::WrongFrame)
}

/// The label `line` prints for `target` when its role is one of the snapshot's own labels and
/// the tree confirms the element is that kind. A `generic` line for an element that does have
/// a role, or an `iframe` line for one that is no frame, is checked like any other.
fn label_of(tree: &SnapshotTree, line: &PrintedLine, target: &NodeKey) -> Option<Label> {
    let node = tree.node_by_key(target)?;
    match line.role.as_str() {
        "generic" if node.role().is_none() => Some(Label::Generic),
        "iframe" if node.is_iframe_boundary() => Some(Label::Iframe),
        _ => None,
    }
}
