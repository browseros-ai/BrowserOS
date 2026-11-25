//! The size cap on a snapshot's text, ported from the TypeScript renderer's `snapshot-cap.ts`.
//! Neo's default format (`neo/`) cuts its lines with it, and `mode: 'full'` (`full_text.rs`) cuts
//! Playwright's lines with the same function: each format says which of its lines are sections
//! and kept blocks, and this cuts them alike.
//!
//! Whole lines are kept from the top while they fit, then one `# cut:` line says how much was
//! left out, how big the whole snapshot is and which section to snapshot instead, then every kept
//! block (a dialog, an alert) after the cut is printed whole: sites put dialogs at the end of the
//! document, and a cut must never hide one.
//!
//! Every length is JavaScript's, in UTF-16 units, and the limit is `maxTokens * 4` as a double,
//! so a fractional `maxTokens` compares as the TypeScript compares it.

use serde_json::Number;

use crate::js;
use crate::snapshot::{ClosedRoots, Snapshot};

/// A token is counted as four UTF-16 units of the text.
const UNITS_PER_TOKEN: f64 = 4.0;

/// The cap, in tokens, of a text in Neo's default format or `mode: 'full'` when the script passes
/// none. The crate applies it itself (`MaxTokens::default`), so a caller leaves a cap the script
/// left out as `None` and never needs this number to render; the run description's tests compare
/// its prose with it.
pub const DEFAULT_MAX_TOKENS: u32 = 8_000;

/// The call a cut `mode: 'full'` text offers on a section's ref: that section in the same mode,
/// not Neo's short list, so following the offer shows what the cut removed in the same format.
const FULL_CALL: &str = "ariaSnapshot({ mode: 'full' })";

/// A token cap as the prelude lets a script pass it: finite and above zero, fractions included
/// (`2.5` is a limit of 10 units). Part of the interface: a caller builds one with
/// [`MaxTokens::new`] from the number the script passed, and the crate cuts at `tokens * 4`
/// UTF-16 units, a double, as the TypeScript compares it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaxTokens(f64);

impl MaxTokens {
    /// The cap for `tokens`, or `None` for a number the prelude refuses (not finite, or not
    /// above zero). The prelude never sends such a number, so a caller that meets `None` has a
    /// broken producer, not a script error.
    #[must_use]
    pub fn new(tokens: f64) -> Option<Self> {
        (tokens.is_finite() && tokens > 0.0).then_some(Self(tokens))
    }

    /// The number of tokens, as the script passed it: what a stored form writes back.
    #[must_use]
    pub fn tokens(self) -> f64 {
        self.0
    }

    /// The limit in UTF-16 units: `maxTokens * 4`, a double.
    pub(crate) fn units(self) -> f64 {
        self.0 * UNITS_PER_TOKEN
    }
}

/// [`DEFAULT_MAX_TOKENS`]: the cap a call takes when the script passed none, its one owner.
impl Default for MaxTokens {
    fn default() -> Self {
        Self(f64::from(DEFAULT_MAX_TOKENS))
    }
}

/// Which snapshot call the cut line offers on a section's ref: the call of the text being cut, so
/// following the offer gives that section in the format the agent was reading.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Offer {
    /// `ariaSnapshot()`: Neo's default format.
    Neo,
    /// `ariaSnapshot({ mode: 'full' })`.
    Full,
}

impl Offer {
    /// The call as the cut line writes it after `page.getByRef('<ref>').`.
    fn call(self) -> &'static str {
        match self {
            Self::Neo => "ariaSnapshot()",
            Self::Full => FULL_CALL,
        }
    }
}

/// One printed line of a format, indented, with what the cap needs to know of it.
#[derive(Debug)]
pub(crate) struct Printed {
    pub(crate) text: String,
    /// The line prints a `[ref=…]`, by its format's own account.
    pub(crate) has_ref: bool,
    /// A section line with content: what the cut line can offer to snapshot instead.
    pub(crate) section: Option<Section>,
    /// The first line of a kept block: the index after its last line, always past its own.
    pub(crate) kept_end: Option<usize>,
}

/// A section the cut line can offer: `page.getByRef('<ref>').ariaSnapshot()`.
#[derive(Debug)]
pub(crate) struct Section {
    pub(crate) r#ref: String,
    /// Its role, and its quoted name when it has one: `navigation "Footer"`. The cut line says
    /// `the navigation "Footer" section`.
    pub(crate) label: String,
    /// The index after its last line.
    pub(crate) end: usize,
}

/// `cap(head, pinned, lines, limit, call)`: `head`, then `pinned`, then `lines`, within `limit`
/// units. The head lines (the notice) count toward the limit. The pinned lines (the closed-roots
/// line), the cut line and the kept blocks printed after it do not, and print whatever the limit.
/// `offer` is the call the cut line offers on a section's ref, in the caller's own format.
pub(crate) fn cap(
    head: &[String],
    pinned: &[String],
    lines: &[Printed],
    limit: f64,
    offer: Offer,
) -> String {
    let texts = || {
        head.iter()
            .map(String::as_str)
            .chain(lines.iter().map(|line| line.text.as_str()))
    };
    let whole = join(
        head.iter()
            .chain(pinned)
            .map(String::as_str)
            .chain(lines.iter().map(|line| line.text.as_str())),
    );
    // What the limit measures: the whole text without the pinned lines and their newlines.
    let pinned_units: usize = pinned.iter().map(|line| js::units(line) + 1).sum();
    let measured = as_number(js::units(&whole)) - as_number(pinned_units);
    if measured <= limit {
        return whole;
    }
    // Whole lines from the top while they fit; `used` counts the newline before each but the
    // first, so it starts at -1.
    let mut used = -1.0;
    let mut taken = 0;
    for text in texts() {
        let next = used + 1.0 + as_number(js::units(text));
        if next > limit {
            break;
        }
        used = next;
        taken += 1;
    }
    let top = head.iter().take(taken);
    let mut cut = taken.saturating_sub(head.len());
    // A kept block is never split: when the limit falls inside one, the cut goes before it. The
    // first such block is the outermost, since a nested block starts after its parent.
    if let Some(index) = lines
        .iter()
        .take(cut)
        .position(|line| line.kept_end.is_some_and(|end| end > cut))
    {
        cut = index;
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut more = 0;
    let mut with_refs = 0;
    let mut later: Option<&Section> = None;
    let mut index = cut;
    while let Some(line) = lines.get(index) {
        if let Some(end) = line.kept_end {
            // At the left edge: no section is open after the cut line, so its old indentation
            // would point at nothing. A kept block nested in it prints once, inside it.
            let indent = js::units(&line.text) - js::units(js::trim_start(&line.text));
            for kept in lines.get(index..end.min(lines.len())).unwrap_or_default() {
                blocks.push(js::skip_units(&kept.text, indent).into_owned());
            }
            // A block is at least its own line, so the walk always moves on.
            index = end.max(index + 1);
            continue;
        }
        more += 1;
        if line.has_ref {
            with_refs += 1;
        }
        later = later.or(line.section.as_ref());
        index += 1;
    }
    // Every line past the limit sits in a kept block, which prints whole anyway: nothing is cut,
    // so the text stays as it was, each block in its place.
    if more == 0 {
        return whole;
    }
    let size = format!(
        "the whole snapshot is about {} tokens",
        number(about_tokens(measured))
    );
    let call = offer.call();
    let offer = match (open_section(lines, cut, limit), later) {
        (Some(open), _) => format!(
            "page.getByRef('{}').{call} shows all of the {} section. Or pass a larger maxTokens.",
            open.r#ref, open.label
        ),
        (None, Some(later)) => format!(
            "page.getByRef('{}').{call} shows the {} section, which is not shown above. Or pass \
             a larger maxTokens.",
            later.r#ref, later.label
        ),
        (None, None) => "Pass a larger maxTokens.".to_owned(),
    };
    let cut_line = format!("# cut: {more} more lines ({with_refs} with refs); {size}. {offer}");
    join(
        top.chain(pinned)
            .map(String::as_str)
            .chain(lines.iter().take(cut).map(|line| line.text.as_str()))
            .chain([cut_line.as_str()])
            .chain(blocks.iter().map(String::as_str)),
    )
}

/// `openSection`: the outermost section open at the cut whose own lines fit the limit, so its
/// snapshot shows all that the cut removed from it. A section larger than the limit is never
/// offered: its own snapshot would be cut at almost the same place, and the agent would go round
/// in a loop.
fn open_section(lines: &[Printed], cut: usize, limit: f64) -> Option<&Section> {
    lines
        .iter()
        .take(cut)
        .enumerate()
        .find_map(|(index, line)| {
            let section = line.section.as_ref()?;
            if section.end <= cut {
                return None;
            }
            let own = lines.get(index..section.end.min(lines.len()))?;
            (as_number(joined_units(own)) <= limit).then_some(section)
        })
}

/// `partialHead`: the line a text in Neo's default format or `mode: 'full'` pins before its tree
/// when some closed shadow roots may be unread. No `maxTokens` may hide that the tree itself is
/// short.
pub(crate) fn partial_head(snapshot: &Snapshot) -> Option<String> {
    snapshot
        .closed_roots
        .as_ref()
        .map(|ClosedRoots { sent, found }| {
            format!("# partial: closed shadow roots not all read ({sent} of {found})")
        })
}

/// `aboutTokens`: `units` as tokens rounded to the nearest 100, and at least 100, as the notice
/// and the cut line say sizes: "about 0 tokens" would misstate text that is there.
pub(crate) fn about_tokens(units: f64) -> f64 {
    f64::max(
        100.0,
        js::math_round(units / UNITS_PER_TOKEN / 100.0) * 100.0,
    )
}

/// The length of `lines` joined with newlines, in UTF-16 units: `joinedLength`, and
/// `lines.join('\n').length`.
pub(crate) fn joined_units(lines: &[Printed]) -> usize {
    let units: usize = lines.iter().map(|line| js::units(&line.text)).sum();
    units + lines.len().saturating_sub(1)
}

/// A count of units as the double JavaScript holds a string's length in.
#[expect(
    clippy::cast_precision_loss,
    reason = "every count below 2^53 converts exactly, and no text is that long"
)]
pub(crate) fn as_number(units: usize) -> f64 {
    units as f64
}

/// `String(x)` for a number a line prints, such as a token count.
pub(crate) fn number(x: f64) -> String {
    Number::from_f64(x)
        .as_ref()
        .map(js::number_text)
        .unwrap_or_default()
}

/// `parts.join('\n')`.
pub(crate) fn join<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut text = String::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(part);
    }
    text
}
