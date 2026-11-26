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
use crate::snapshot::ClosedRoots;

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

/// Position of a line pushed to a [`LineSink`], passed back when its section or kept block closes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LineMark {
    pub(crate) index: usize,
    text_units_before: usize,
    in_kept: bool,
}

/// Per-line flags passed to [`LineSink::push_line`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LineMeta {
    pub(crate) has_ref: bool,
    pub(crate) starts_kept: bool,
}

/// Streaming target for finalized lines from `neo/` and `full_text.rs`.
///
/// Printers push each finalized line once in document order and notify the sink when an open kept
/// block or section ends. Implementations either cap and retain only the lines needed for output
/// ([`LineCap`]), measure total UTF-16 length ([`UnitsSink`]), or count lines and refs for inert
/// omission notices ([`CountSink`]).
pub(crate) trait LineSink {
    /// Whether this sink inspects the formatted line text passed to [`Self::push_line`]. Returning
    /// `false` lets the printer skip indentation and YAML escaping for lines that are only counted.
    fn needs_text(&self) -> bool {
        true
    }

    /// Appends one finalized line; `meta.starts_kept` opens a kept block covering this line and
    /// every line until the matching [`Self::end_kept`].
    fn push_line(&mut self, text: &str, meta: LineMeta) -> LineMark;

    /// Closes the kept block opened at `mark`.
    fn end_kept(&mut self, mark: LineMark);

    /// Closes the section opened at `mark` with its ref, role, and optional name.
    fn end_section(&mut self, mark: LineMark, r#ref: &str, role: &str, name: Option<&str>);
}

#[derive(Debug)]
struct StreamLine {
    text: String,
    has_ref: bool,
    section: Option<StreamSection>,
    kept_end: Option<usize>,
}

#[derive(Debug)]
struct StreamSection {
    r#ref: String,
    label: String,
    end: usize,
    own_units: usize,
}

/// Streaming equivalent of [`cap`]: retains only lines up to `limit` plus kept blocks (`dialog`,
/// `alertdialog`, `alert`, `status`), discarding non-kept lines past the cut while maintaining
/// exact UTF-16 counts, ref counts, and section offers.
///
/// When `abandon_above` is set (Neo's default length probe when the script left `text` unset),
/// crossing that unit budget drops retention and switches to pure length counting so a large page
/// that will fall back to interactive mode never buffers beyond the probe budget.
#[derive(Debug)]
pub(crate) struct LineCap {
    limit: f64,
    abandon_above: Option<usize>,
    abandoned: bool,
    used: f64,
    overflowed: bool,
    prefix: Vec<StreamLine>,
    kept_depth: usize,
    in_crossing_kept: bool,
    crossing_kept_tail: Vec<String>,
    overflow_kept_blocks: Vec<Vec<String>>,
    overflow_more: usize,
    overflow_with_refs: usize,
    overflow_later: Option<(usize, StreamSection)>,
    total_lines: usize,
    total_text_units: usize,
}

impl LineCap {
    pub(crate) fn new(limit: f64) -> Self {
        Self::with_abandon(limit, None)
    }

    pub(crate) fn with_abandon(limit: f64, abandon_above: Option<usize>) -> Self {
        Self {
            limit,
            abandon_above,
            abandoned: false,
            used: -1.0,
            overflowed: false,
            prefix: Vec::new(),
            kept_depth: 0,
            in_crossing_kept: false,
            crossing_kept_tail: Vec::new(),
            overflow_kept_blocks: Vec::new(),
            overflow_more: 0,
            overflow_with_refs: 0,
            overflow_later: None,
            total_lines: 0,
            total_text_units: 0,
        }
    }

    /// Number of finalized lines pushed so far.
    pub(crate) fn line_count(&self) -> usize {
        self.total_lines
    }

    /// Length of all pushed lines joined with `\n`, in UTF-16 units.
    pub(crate) fn joined_units(&self) -> usize {
        self.total_text_units + self.total_lines.saturating_sub(1)
    }

    /// Finalizes the capped text with `head` (counted toward `limit`), `pinned` (never counted),
    /// and `offer` (the cut line's suggested call).
    pub(crate) fn finish(self, head: &[String], pinned: &[String], offer: Offer) -> String {
        let non_pinned_count = head.len() + self.total_lines;
        if non_pinned_count == 0 {
            return join(pinned.iter().map(String::as_str));
        }
        let head_text_units: usize = head.iter().map(|line| js::units(line)).sum();
        let measured = as_number(head_text_units + self.total_text_units + (non_pinned_count - 1));
        if measured <= self.limit {
            return join(
                head.iter()
                    .chain(pinned)
                    .map(String::as_str)
                    .chain(self.prefix.iter().map(|line| line.text.as_str())),
            );
        }
        let mut used = -1.0;
        let mut taken = 0;
        for text in head
            .iter()
            .map(String::as_str)
            .chain(self.prefix.iter().map(|line| line.text.as_str()))
        {
            let next = used + 1.0 + as_number(js::units(text));
            if next > self.limit {
                break;
            }
            used = next;
            taken += 1;
        }
        let top = head.iter().take(taken);
        let mut cut = taken.saturating_sub(head.len());
        if let Some(index) = self
            .prefix
            .iter()
            .take(cut)
            .position(|line| line.kept_end.is_some_and(|end| end > cut))
        {
            cut = index;
        }
        let CutTail {
            more,
            with_refs,
            later,
            blocks,
        } = self.collect_cut_tail(cut);
        if more == 0 {
            return self.join_whole(head, pinned);
        }
        let open = self.prefix.iter().take(cut).find_map(|line| {
            let section = line.section.as_ref()?;
            (section.end > cut && as_number(section.own_units) <= self.limit).then_some(section)
        });
        let size = format!(
            "the whole snapshot is about {} tokens",
            number(about_tokens(measured))
        );
        let call = offer.call();
        let offer_text = match (open, later) {
            (Some(open), _) => format!(
                "page.getByRef('{}').{call} shows all of the {} section. Or pass a larger \
                 maxTokens.",
                open.r#ref, open.label
            ),
            (None, Some(later)) => format!(
                "page.getByRef('{}').{call} shows the {} section, which is not shown above. Or \
                 pass a larger maxTokens.",
                later.r#ref, later.label
            ),
            (None, None) => "Pass a larger maxTokens.".to_owned(),
        };
        let cut_line =
            format!("# cut: {more} more lines ({with_refs} with refs); {size}. {offer_text}");
        join(
            top.chain(pinned)
                .map(String::as_str)
                .chain(self.prefix.iter().take(cut).map(|line| line.text.as_str()))
                .chain([cut_line.as_str()])
                .chain(blocks.iter().map(String::as_str)),
        )
    }

    fn collect_cut_tail(&self, cut: usize) -> CutTail<'_> {
        let prefix_len = self.prefix.len();
        let mut blocks: Vec<String> = Vec::new();
        let mut more = 0;
        let mut with_refs = 0;
        let mut later: Option<&StreamSection> = None;
        let mut index = cut;
        while let Some(line) = self.prefix.get(index) {
            if let Some(end) = line.kept_end {
                let indent = js::units(&line.text) - js::units(js::trim_start(&line.text));
                for kept in self
                    .prefix
                    .get(index..end.min(prefix_len))
                    .unwrap_or_default()
                {
                    blocks.push(js::skip_units(&kept.text, indent).into_owned());
                }
                if end > prefix_len {
                    for kept in &self.crossing_kept_tail {
                        blocks.push(js::skip_units(kept, indent).into_owned());
                    }
                }
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
        if self.overflowed {
            more += self.overflow_more;
            with_refs += self.overflow_with_refs;
            if later.is_none() {
                later = self.overflow_later.as_ref().map(|(_, section)| section);
            }
            for block in &self.overflow_kept_blocks {
                if let Some(first) = block.first() {
                    let indent = js::units(first) - js::units(js::trim_start(first));
                    for kept in block {
                        blocks.push(js::skip_units(kept, indent).into_owned());
                    }
                }
            }
        }
        CutTail {
            more,
            with_refs,
            later,
            blocks,
        }
    }

    fn join_whole(&self, head: &[String], pinned: &[String]) -> String {
        join(
            head.iter()
                .chain(pinned)
                .map(String::as_str)
                .chain(self.prefix.iter().map(|line| line.text.as_str()))
                .chain(self.crossing_kept_tail.iter().map(String::as_str))
                .chain(
                    self.overflow_kept_blocks
                        .iter()
                        .flat_map(|block| block.iter().map(String::as_str)),
                ),
        )
    }
}

struct CutTail<'a> {
    more: usize,
    with_refs: usize,
    later: Option<&'a StreamSection>,
    blocks: Vec<String>,
}

impl LineSink for LineCap {
    fn push_line(&mut self, text: &str, meta: LineMeta) -> LineMark {
        let LineMeta {
            has_ref,
            starts_kept,
        } = meta;
        let index = self.total_lines;
        let text_units_before = self.total_text_units;
        let in_kept = self.kept_depth > 0;
        let units = js::units(text);
        self.total_lines += 1;
        self.total_text_units += units;
        if self.abandoned {
            return LineMark {
                index,
                text_units_before,
                in_kept,
            };
        }
        if let Some(budget) = self.abandon_above
            && self.joined_units() > budget
        {
            self.abandoned = true;
            return LineMark {
                index,
                text_units_before,
                in_kept,
            };
        }
        if !self.overflowed {
            let next = self.used + 1.0 + as_number(units);
            if next <= self.limit {
                self.used = next;
                self.prefix.push(StreamLine {
                    text: text.to_owned(),
                    has_ref,
                    section: None,
                    kept_end: None,
                });
                if starts_kept {
                    self.kept_depth += 1;
                }
                return LineMark {
                    index,
                    text_units_before,
                    in_kept,
                };
            }
            self.overflowed = true;
            self.in_crossing_kept = self.kept_depth > 0;
        }
        if self.kept_depth > 0 {
            if starts_kept {
                self.kept_depth += 1;
            }
            if self.in_crossing_kept {
                self.crossing_kept_tail.push(text.to_owned());
            } else if let Some(block) = self.overflow_kept_blocks.last_mut() {
                block.push(text.to_owned());
            }
        } else if starts_kept {
            self.kept_depth = 1;
            self.in_crossing_kept = false;
            self.overflow_kept_blocks.push(vec![text.to_owned()]);
        } else {
            self.overflow_more += 1;
            if has_ref {
                self.overflow_with_refs += 1;
            }
        }
        LineMark {
            index,
            text_units_before,
            in_kept,
        }
    }

    fn end_kept(&mut self, mark: LineMark) {
        if self.abandoned {
            return;
        }
        if let Some(line) = self.prefix.get_mut(mark.index) {
            line.kept_end = Some(self.total_lines);
        }
        self.kept_depth = self.kept_depth.saturating_sub(1);
        if self.kept_depth == 0 {
            self.in_crossing_kept = false;
        }
    }

    fn end_section(&mut self, mark: LineMark, r#ref: &str, role: &str, name: Option<&str>) {
        if self.abandoned {
            return;
        }
        let end = self.total_lines;
        if let Some(line) = self.prefix.get_mut(mark.index) {
            let own_units = (self.total_text_units - mark.text_units_before)
                + (end - mark.index).saturating_sub(1);
            line.section = Some(StreamSection {
                r#ref: r#ref.to_owned(),
                label: format_section_label(role, name),
                end,
                own_units,
            });
        } else if !mark.in_kept
            && self
                .overflow_later
                .as_ref()
                .is_none_or(|(index, _)| mark.index < *index)
        {
            self.overflow_later = Some((
                mark.index,
                StreamSection {
                    r#ref: r#ref.to_owned(),
                    label: format_section_label(role, name),
                    end,
                    own_units: 0,
                },
            ));
        }
    }
}

/// Sink that measures only the joined UTF-16 length of a walk without storing any lines.
#[derive(Debug, Default)]
pub(crate) struct UnitsSink {
    total_lines: usize,
    total_text_units: usize,
}

impl UnitsSink {
    pub(crate) fn joined_units(&self) -> usize {
        self.total_text_units + self.total_lines.saturating_sub(1)
    }
}

impl LineSink for UnitsSink {
    fn push_line(&mut self, text: &str, _meta: LineMeta) -> LineMark {
        let index = self.total_lines;
        let text_units_before = self.total_text_units;
        self.total_lines += 1;
        self.total_text_units += js::units(text);
        LineMark {
            index,
            text_units_before,
            in_kept: false,
        }
    }

    fn end_kept(&mut self, _mark: LineMark) {}

    fn end_section(&mut self, _mark: LineMark, _ref: &str, _role: &str, _name: Option<&str>) {}
}

/// Sink that counts lines and lines with refs without formatting or allocating line text.
#[derive(Debug, Default)]
pub(crate) struct CountSink {
    pub(crate) lines: usize,
    pub(crate) with_refs: usize,
}

impl LineSink for CountSink {
    fn needs_text(&self) -> bool {
        false
    }

    fn push_line(&mut self, _text: &str, meta: LineMeta) -> LineMark {
        let index = self.lines;
        self.lines += 1;
        if meta.has_ref {
            self.with_refs += 1;
        }
        LineMark {
            index,
            text_units_before: 0,
            in_kept: false,
        }
    }

    fn end_kept(&mut self, _mark: LineMark) {}

    fn end_section(&mut self, _mark: LineMark, _ref: &str, _role: &str, _name: Option<&str>) {}
}

/// Formats a section label (`role` plus JSON-quoted non-empty `name`) for cut offers.
pub(crate) fn format_section_label(role: &str, name: Option<&str>) -> String {
    let mut label = role.to_owned();
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        label.push(' ');
        js::json_quote(name, &mut label);
    }
    label
}

/// Appends `level * 2` ASCII spaces to `out` without intermediate allocation.
pub(crate) fn push_indent(out: &mut String, level: usize) {
    const SPACES: &str = "                                                                ";
    let mut remaining = level.saturating_mul(2);
    while remaining >= SPACES.len() {
        out.push_str(SPACES);
        remaining -= SPACES.len();
    }
    if remaining > 0
        && let Some(chunk) = SPACES.get(..remaining)
    {
        out.push_str(chunk);
    }
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
    let non_pinned_count = head.len() + lines.len();
    if non_pinned_count == 0 {
        return join(pinned.iter().map(String::as_str));
    }
    let head_units: usize = head.iter().map(|line| js::units(line)).sum();
    let line_units: usize = lines.iter().map(|line| js::units(&line.text)).sum();
    let measured = as_number(head_units + line_units + (non_pinned_count - 1));
    if measured <= limit {
        return join(
            head.iter()
                .chain(pinned)
                .map(String::as_str)
                .chain(lines.iter().map(|line| line.text.as_str())),
        );
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
        return join(
            head.iter()
                .chain(pinned)
                .map(String::as_str)
                .chain(lines.iter().map(|line| line.text.as_str())),
        );
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
pub(crate) fn partial_head(closed_roots: Option<&ClosedRoots>) -> Option<String> {
    closed_roots.map(|ClosedRoots { sent, found }| {
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
