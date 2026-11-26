use std::collections::VecDeque;
use std::mem;

use crate::aria_sink::{EmittedNode, INLINE_REF_CAP};
use crate::aria_source::{AriaSource, SourceChild};
use crate::cap::{CountSink, LineMark, LineMeta, LineSink, push_indent};
use crate::js;
use crate::playwright::{NodeAlone, node_text_into, text_line};
use crate::snapshot::PartialFrame;

use super::hidden::Hidden;
use super::node_line::{Keep, node_line};
use super::on_top::OnTop;
use super::own_text::{TextScratch, name_as_content, own_text};
use super::rule::{FIELD, Rule, actionable, kept_ref, named, shown_text};

/// A nameless control's own text, or a link's url as its label, is clipped to this many UTF-16
/// units when the page's text is left out.
const OWN_TEXT_CLIP: usize = 80;

/// Past this many options, a native select lists only its selected ones when the page's text is
/// left out.
const LONG_SELECT: usize = 12;

/// Counts collected for inert subtrees left out of a walk.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OmittedStats {
    pub(super) lines: usize,
    pub(super) with_refs: usize,
    pub(super) skipped: usize,
}

/// The walk of one form of Neo's default format (`Printer` in `snapshot-v1.ts`): it turns the
/// tree into finalized lines by the format's rules and streams them directly into `K: LineSink`.
///
/// The steps wait on a stack of their own rather than the call stack, as in the TypeScript: real
/// pages nest deeper than a recursive walk allows, and a block's closing step must run after all
/// of its content. Only tentative block headers and a trailing joinable text line stay buffered
/// in `pending`; once a block contains finalized content, its header and preceding settled lines
/// flush straight into the sink so memory stays bounded by nesting depth rather than tree size.
pub(super) struct Printer<'a, 's, 't, S: AriaSource<'a>, K: LineSink> {
    source: &'s S,
    sink: K,
    pending: VecDeque<Line>,
    /// Number of raw lines (including dropped separator-only text lines) already popped from
    /// `pending`, so `flushed_raw_lines + i` matches the index in an un-streamed line vector.
    flushed_raw_lines: usize,
    /// `(serial, body.len())` of the most recently flushed raw line, preserving `Boundary` checks
    /// even after `pending` empties or truncates back to a flushed line.
    flushed_last_sig: Option<(usize, usize)>,
    open_blocks: Vec<OpenBlock>,
    /// The walk's pending steps; the next one is last.
    steps: Vec<Step<'a, S::Id>>,
    /// Text pieces join only lines at or past this global raw index: the page after a whole-view
    /// layer's content starts a new text run even at the same level.
    text_boundary: usize,
    urls: bool,
    /// The frames not read whose iframe line has not been printed.
    partials: Partials<'a>,
    inert: Inert<'a, 's, 't, S>,
    on_top: &'t OnTop<S::Id, S::Key>,
    /// The serial the next line gets.
    next_serial: usize,
    scratch_key: String,
    scratch_own: String,
    scratch_line: String,
    text_scratch: TextScratch<'a, S::Id>,
}

/// What a printer does with a subtree the page marked inert.
pub(super) enum Inert<'a, 's, 't, S: AriaSource<'a>> {
    /// Prints it like any other: under `{ inert: true }`, and on an omission's own printer.
    Listed,
    /// Prints it on counting printers of its own, which tally it for the `# not listed:` line.
    Omitted(Box<Omissions<'a, 's, 't, S>>),
}

impl<'a, S: AriaSource<'a>> Inert<'a, '_, '_, S> {
    fn listed(&self) -> bool {
        matches!(self, Self::Listed)
    }
}

/// The inert subtrees a printer left out, counted as the walk meets them.
pub(super) struct Omissions<'a, 's, 't, S: AriaSource<'a>> {
    stats: OmittedStats,
    /// Active counting printer for the current run of back-to-back omissions.
    active: Option<Printer<'a, 's, 't, S, CountSink>>,
    /// Where the last omission was met, to tell whether the next one is back to back.
    boundary: Option<Boundary>,
}

impl<'a, S: AriaSource<'a>> Default for Omissions<'a, '_, '_, S> {
    fn default() -> Self {
        Self {
            stats: OmittedStats::default(),
            active: None,
            boundary: None,
        }
    }
}

impl<'a, S: AriaSource<'a>> Omissions<'a, '_, '_, S> {
    fn finish_active(&mut self) {
        if let Some(active) = self.active.take() {
            let (count, _) = active.finish();
            self.stats.lines += count.lines;
            self.stats.with_refs += count.with_refs;
        }
    }
}

/// The main printer's state at an omission. The TypeScript compares the last line by identity
/// and its text: a line keeps its serial for as long as it lives and its body only grows (a text
/// piece, a colon), so the serial and the body's length say the same.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Boundary {
    last: Option<(usize, usize)>,
    lines: usize,
    level: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Section,
    Kept,
}

#[derive(Clone, Copy)]
enum BlockState {
    /// Header still sits in `pending` at global raw index `header_raw_idx`.
    Tentative { header_raw_idx: usize },
    /// Header already gained `:` and flushed to the sink at `sink_mark`.
    Committed { sink_mark: LineMark },
}

#[derive(Clone, Copy)]
struct OpenBlock {
    kind: BlockKind,
    state: BlockState,
}

/// One pending line before its indentation.
struct Line {
    level: usize,
    /// A text line's raw joined pieces, quoted only when flushed; any other line as printed,
    /// which for a block's first line may hold a line break inside a quoted key.
    body: String,
    kind: LineKind,
    has_ref: bool,
    /// Which line this is: no two lines of one printer share one, as no two JavaScript objects
    /// are the same object.
    serial: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// Text pieces, joined with spaces, printed as a `- text:` line.
    Text,
    /// A node's line, or a select's `- text: N more options`.
    Element,
    /// A `# partial:` comment.
    Comment,
}

/// One step of the walk.
enum Step<'a, Id> {
    Node {
        child: SourceChild<'a, Id>,
        level: usize,
        text: bool,
        hidden: Hidden,
    },
    Piece {
        raw: &'a str,
        level: usize,
        text: bool,
    },
    /// Closes a kept block after its content.
    Kept {
        node: EmittedNode<'a>,
        level: usize,
        hidden: Hidden,
    },
    /// Closes a section after its content.
    Section { node: EmittedNode<'a> },
}

/// The frames not read by iframe ref, in the order each ref first came, as a JavaScript `Map`
/// iterates; a delete keeps the others' order. Each is removed once printed, or once its iframe
/// is left out as inert, so none appears twice.
#[derive(Default)]
pub(super) struct Partials<'a>(Vec<(&'a str, Vec<&'a PartialFrame>)>);

impl<'a> Partials<'a> {
    pub(super) fn of(frames: &'a [PartialFrame]) -> Self {
        let mut partials = Self::default();
        for frame in frames {
            match partials
                .0
                .iter_mut()
                .find(|(iframe, _)| *iframe == frame.r#ref)
            {
                Some((_, under)) => under.push(frame),
                None => partials.0.push((&frame.r#ref, vec![frame])),
            }
        }
        partials
    }

    /// The frames under `iframe`, removed.
    fn take(&mut self, iframe: &str) -> Vec<&'a PartialFrame> {
        match self.0.iter().position(|(under, _)| *under == iframe) {
            Some(index) => self.0.remove(index).1,
            None => Vec::new(),
        }
    }
}

impl<'a, 's, 't, S: AriaSource<'a>, K: LineSink> Printer<'a, 's, 't, S, K> {
    pub(super) fn new(
        source: &'s S,
        sink: K,
        urls: bool,
        partials: Partials<'a>,
        inert: Inert<'a, 's, 't, S>,
        on_top: &'t OnTop<S::Id, S::Key>,
    ) -> Self {
        Self {
            source,
            sink,
            pending: VecDeque::new(),
            flushed_raw_lines: 0,
            flushed_last_sig: None,
            open_blocks: Vec::new(),
            steps: Vec::new(),
            text_boundary: 0,
            urls,
            partials,
            inert,
            on_top,
            next_serial: 0,
            scratch_key: String::new(),
            scratch_own: String::new(),
            scratch_line: String::new(),
            text_scratch: TextScratch::default(),
        }
    }

    /// Prints `children` at `level`, depth first in document order.
    pub(super) fn walk(
        &mut self,
        children: impl DoubleEndedIterator<Item = SourceChild<'a, S::Id>>,
        level: usize,
        text: bool,
        hidden: Hidden,
    ) {
        for child in children.rev() {
            self.steps.push(Step::Node {
                child,
                level,
                text,
                hidden,
            });
        }
        while let Some(step) = self.steps.pop() {
            match step {
                Step::Node {
                    child,
                    level,
                    text,
                    hidden,
                } => self.node(child, level, text, hidden),
                Step::Piece { raw, level, text } => self.piece(raw, level, text),
                Step::Kept {
                    node,
                    level,
                    hidden,
                } => self.close_kept(&node, level, hidden),
                Step::Section { node } => self.close_section(&node),
            }
        }
    }

    /// A whole-view layer's content and the page after it are separate text runs even at the
    /// same level.
    pub(super) fn separate_text(&mut self) {
        self.text_boundary = self.logical_line_count();
        self.flush_settled();
    }

    /// The frames whose iframe printed no line, at the top level, in the order of their refs.
    pub(super) fn comment_unplaced_partials(&mut self) {
        for (_, frames) in mem::take(&mut self.partials.0) {
            for frame in frames {
                self.comment(partial_comment(frame), 0);
            }
        }
    }

    /// Flushes any remaining pending lines and returns the sink plus omitted inert stats.
    pub(super) fn finish(mut self) -> (K, OmittedStats) {
        self.flush_all_pending();
        let omitted = match self.inert {
            Inert::Listed => OmittedStats::default(),
            Inert::Omitted(mut omissions) => {
                omissions.finish_active();
                omissions.stats
            }
        };
        (self.sink, omitted)
    }

    fn logical_line_count(&self) -> usize {
        self.flushed_raw_lines + self.pending.len()
    }

    fn current_last_sig(&self) -> Option<(usize, usize)> {
        self.pending
            .back()
            .map(|line| (line.serial, line.body.len()))
            .or(self.flushed_last_sig)
    }

    /// One child of the node being walked, by the first rule that matches it. `hidden` is what its
    /// line inherits of an `aria-hidden` node above it.
    fn node(&mut self, child: SourceChild<'a, S::Id>, level: usize, text: bool, hidden: Hidden) {
        let rule = match child {
            SourceChild::Text(raw) => Rule::Piece(raw),
            SourceChild::Node(id) => {
                let node = self.source.node(id);
                if node.inert == Some(true) && !self.inert.listed() {
                    self.omit(id, level, text, hidden);
                    return;
                }
                Rule::of_node(id, node)
            }
        };
        match rule {
            Rule::Piece(raw) => self.piece(raw, level, text),
            Rule::Kept(id, node) => self.kept(id, node, level, hidden),
            Rule::Section(id, node) => self.section(id, node, level, text, hidden),
            Rule::Heading(id, node) => self.heading(id, &node, level, hidden),
            Rule::Control(id, node) => self.control(id, &node, level, text, hidden),
            Rule::Image(id, node) => {
                if text && named(node.name).is_some() {
                    let keep = Keep {
                        r#ref: false,
                        urls: false,
                        text: None,
                        hidden,
                    };
                    self.emit_element_lines(node_line(&node, keep), level, false);
                }
                // No line of its own: its text and children belong to the level it sits in.
                self.content(id, &node, level, text, hidden.beside(&node));
            }
            Rule::Content(id, node) => {
                if text
                    && let Some(named) =
                        name_as_content(self.source, id, &node, self.on_top, &mut self.text_scratch)
                {
                    self.piece(named, level, text);
                }
                self.content(id, &node, level, text, hidden.beside(&node));
            }
        }
    }

    /// Leaves out an inert subtree: it is printed on the counting printer of its run of
    /// omissions, with the text form and the mark it inherits here, and its frames not read go
    /// with it.
    fn omit(&mut self, id: S::Id, level: usize, text: bool, hidden: Hidden) {
        let boundary = Boundary {
            last: self.current_last_sig(),
            lines: self.logical_line_count(),
            level,
        };
        let Inert::Omitted(omissions) = &mut self.inert else {
            return;
        };
        omissions.stats.skipped += 1;
        // A printed line or text piece between two omissions separates them, as it did in the
        // page; so does a change of level.
        if omissions.boundary != Some(boundary) {
            omissions.finish_active();
            omissions.active = Some(Printer::new(
                self.source,
                CountSink::default(),
                self.urls,
                Partials::default(),
                Inert::Listed,
                self.on_top,
            ));
        }
        if let Some(segment) = omissions.active.as_mut() {
            segment.walk([SourceChild::Node(id)].into_iter(), level, text, hidden);
        }
        omissions.boundary = Some(boundary);
        // An omitted iframe owns its frames' `# partial:` lines, so none may reappear later.
        if self.partials.0.is_empty() {
            return;
        }
        let on_top = self.on_top;
        let source = self.source;
        let mut pending = vec![id];
        while let Some(cur_id) = pending.pop() {
            let cur_node = source.node(cur_id);
            if let Some(node_ref) = cur_node.r#ref
                && !node_ref.is_empty()
            {
                let mut ref_buf = [0u8; INLINE_REF_CAP];
                self.partials.take(node_ref.format_into(&mut ref_buf));
            }
            if cur_node.has_children {
                pending.extend(
                    on_top
                        .children(source, cur_id)
                        .filter_map(|child| match child {
                            SourceChild::Node(child_id) => Some(child_id),
                            SourceChild::Text(_) => None,
                        }),
                );
            }
        }
    }

    /// Schedules a node's own text (none for an iframe: `shown_text`), then its children, as
    /// the next steps. Children go on in reverse so they come off in document order, ahead of the
    /// siblings already waiting.
    fn content(
        &mut self,
        id: S::Id,
        node: &EmittedNode<'a>,
        level: usize,
        text: bool,
        hidden: Hidden,
    ) {
        if node.has_children {
            let on_top = self.on_top;
            let source = self.source;
            for child in on_top.children(source, id).rev() {
                self.steps.push(Step::Node {
                    child,
                    level,
                    text,
                    hidden,
                });
            }
        }
        if let Some(raw) = shown_text(node) {
            self.steps.push(Step::Piece { raw, level, text });
        }
    }

    /// A text piece joins the text line printed last, when that is at the same level, with one
    /// space. Pieces stay raw until flushed, so YAML's quoting covers the whole line.
    fn piece(&mut self, raw: &str, level: usize, text: bool) {
        if !text || raw.is_empty() {
            return;
        }
        let joins = self.logical_line_count() > self.text_boundary;
        if let Some(last) = self.pending.back_mut()
            && joins
            && last.kind == LineKind::Text
            && last.level == level
        {
            last.body.push(' ');
            last.body.push_str(raw);
            return;
        }
        self.push_pending(level, raw.to_owned(), LineKind::Text, false);
        self.flush_settled();
    }

    fn kept(&mut self, id: S::Id, node: EmittedNode<'a>, level: usize, hidden: Hidden) {
        // The cap may print the block outside the sections around it.
        let hidden = hidden.movable();
        let keep = Keep {
            r#ref: kept_ref(&node),
            urls: false,
            text: None,
            hidden,
        };
        let header_raw_idx = self.push_tentative_header(node_line(&node, keep), level);
        self.open_blocks.push(OpenBlock {
            kind: BlockKind::Kept,
            state: BlockState::Tentative { header_raw_idx },
        });
        self.steps.push(Step::Kept {
            node,
            level,
            hidden,
        });
        // Its text is what tells the agent what its buttons do, so it prints whatever the form.
        self.content(id, &node, level + 1, true, hidden.under(&node));
    }

    /// After a kept block's content: the leaf form, a colon, or no line at all.
    fn close_kept(&mut self, node: &EmittedNode<'a>, level: usize, hidden: Hidden) {
        let Some(open) = self.open_blocks.pop() else {
            return;
        };
        match open.state {
            BlockState::Committed { sink_mark } => {
                self.flush_all_pending();
                self.sink.end_kept(sink_mark);
            }
            BlockState::Tentative { header_raw_idx } => {
                let pos = header_raw_idx.saturating_sub(self.flushed_raw_lines);
                let is_only_text = self.pending.len() == pos + 2
                    && self
                        .pending
                        .get(pos + 1)
                        .is_some_and(|only| only.kind == LineKind::Text && !separators_only(only));
                if is_only_text {
                    let only_text = self
                        .pending
                        .get_mut(pos + 1)
                        .map(|only| mem::take(&mut only.body))
                        .unwrap_or_default();
                    self.pending.truncate(pos);
                    let keep = Keep {
                        r#ref: kept_ref(node),
                        urls: false,
                        text: Some(&only_text),
                        hidden,
                    };
                    self.emit_element_lines(node_line(node, keep), level, true);
                    return;
                }
                let has_content = self
                    .pending
                    .iter()
                    .skip(pos + 1)
                    .any(|line| line.kind != LineKind::Comment && !separators_only(line));
                if has_content {
                    if let Some(header) = self.pending.get_mut(pos) {
                        header.body.push(':');
                    }
                } else {
                    let keep_empty = named(node.name).is_some()
                        || self
                            .pending
                            .iter()
                            .skip(pos + 1)
                            .any(|line| line.kind == LineKind::Comment);
                    if !keep_empty {
                        self.pending.truncate(pos);
                        return;
                    }
                }
                self.commit_tentative_up_to(header_raw_idx);
                if let Some(mark) = self.flush_one_pending(true) {
                    self.flush_all_pending();
                    self.sink.end_kept(mark);
                }
            }
        }
    }

    fn section(
        &mut self,
        id: S::Id,
        node: EmittedNode<'a>,
        level: usize,
        text: bool,
        hidden: Hidden,
    ) {
        let keep = Keep {
            r#ref: true,
            urls: false,
            text: None,
            hidden,
        };
        let header_raw_idx = self.push_tentative_header(node_line(&node, keep), level);
        self.open_blocks.push(OpenBlock {
            kind: BlockKind::Section,
            state: BlockState::Tentative { header_raw_idx },
        });
        if node.role == "iframe"
            && let Some(node_ref) = node.r#ref
        {
            let mut ref_buf = [0u8; INLINE_REF_CAP];
            // Under its iframe line, as Playwright's format places it.
            for frame in self.partials.take(node_ref.format_into(&mut ref_buf)) {
                self.comment(partial_comment(frame), level + 1);
            }
        }
        self.steps.push(Step::Section { node });
        self.content(id, &node, level + 1, text, hidden.under(&node));
    }

    /// After a section's content: its colon, or no line when nothing printed inside it. An
    /// iframe keeps its line with nothing inside, for the `# partial:` comment under it.
    fn close_section(&mut self, node: &EmittedNode<'a>) {
        let Some(open) = self.open_blocks.pop() else {
            return;
        };
        match open.state {
            BlockState::Committed { sink_mark } => {
                self.flush_all_pending();
                if let Some(reference) = node.r#ref {
                    let mut ref_buf = [0u8; INLINE_REF_CAP];
                    self.sink.end_section(
                        sink_mark,
                        reference.format_into(&mut ref_buf),
                        node.role,
                        node.name,
                    );
                }
            }
            BlockState::Tentative { header_raw_idx } => {
                let pos = header_raw_idx.saturating_sub(self.flushed_raw_lines);
                let has_content = self
                    .pending
                    .iter()
                    .skip(pos + 1)
                    .any(|line| line.kind != LineKind::Comment && !separators_only(line));
                if has_content {
                    if let Some(header) = self.pending.get_mut(pos) {
                        header.body.push(':');
                    }
                } else {
                    let keep_empty = node.role == "iframe"
                        || self
                            .pending
                            .iter()
                            .skip(pos + 1)
                            .any(|line| line.kind == LineKind::Comment);
                    if !keep_empty {
                        self.pending.truncate(pos);
                        return;
                    }
                }
                let ends_with_colon = self
                    .pending
                    .get(pos)
                    .is_some_and(|line| line.body.ends_with(':'));
                self.commit_tentative_up_to(header_raw_idx);
                if let Some(mark) = self.flush_one_pending(false) {
                    self.flush_all_pending();
                    if ends_with_colon && let Some(reference) = node.r#ref {
                        let mut ref_buf = [0u8; INLINE_REF_CAP];
                        self.sink.end_section(
                            mark,
                            reference.format_into(&mut ref_buf),
                            node.role,
                            node.name,
                        );
                    }
                }
            }
        }
    }

    fn heading(&mut self, id: S::Id, node: &EmittedNode<'a>, level: usize, hidden: Hidden) {
        let own = if named(node.name).is_some() {
            String::new()
        } else {
            own_text(
                self.source,
                id,
                self.inert.listed(),
                self.on_top,
                &mut self.text_scratch,
            )
        };
        let keep = Keep {
            r#ref: actionable(node),
            urls: false,
            text: named(Some(own.as_str())),
            hidden,
        };
        self.emit_element_lines(node_line(node, keep), level, false);
        // A link inside the heading gets its own line; the heading's text is not repeated.
        self.content(id, node, level, false, hidden.beside(node));
    }

    fn control(
        &mut self,
        id: S::Id,
        node: &EmittedNode<'a>,
        level: usize,
        text: bool,
        hidden: Hidden,
    ) {
        let name = node.name;
        let value = if let Some(own) = node.text
            && Some(own) != name
        {
            if text || node.role != "generic" || named(name).is_some() {
                // A text that differs from the name: how a text box shows its value.
                Some(own.to_owned())
            } else {
                // A clickable generic's one string is the text its children held, inlined by the
                // ai distillation (`inlineTextIntoGeneric`), so it is clipped as the walk's is.
                Some(clip(own, OWN_TEXT_CLIP))
            }
        } else if FIELD.contains(&node.role) && node.text.is_none() {
            // A dropdown's or an editor's value can be its children instead of a `text`. It is
            // kept on the field's line before the children's walk leaves out repeated text.
            let own = own_text(
                self.source,
                id,
                self.inert.listed(),
                self.on_top,
                &mut self.text_scratch,
            );
            (!own.is_empty() && Some(own.as_str()) != name).then_some(own)
        } else if named(name).is_none() {
            // A link with no words of its own still says where it goes, unless its `/url` line
            // does.
            let mut label = own_text(
                self.source,
                id,
                self.inert.listed(),
                self.on_top,
                &mut self.text_scratch,
            );
            if label.is_empty() && node.role == "link" && !self.urls {
                node.url.unwrap_or_default().clone_into(&mut label);
            }
            if label.is_empty() {
                None
            } else if text {
                Some(label)
            } else {
                Some(clip(&label, OWN_TEXT_CLIP))
            }
        } else {
            None
        };
        let keep = Keep {
            r#ref: true,
            urls: self.urls,
            text: value.as_deref(),
            hidden,
        };
        let line = node_line(node, keep);
        let choices = if node.role == "combobox" || node.role == "listbox" {
            native_options(self.source, id, hidden.under(node), self.on_top)
        } else {
            Vec::new()
        };
        if choices.is_empty() {
            self.emit_element_lines(line, level, false);
        } else {
            // A native select: its options carry no refs; they are its value and its choices.
            self.emit_committed_header_with_colon(line, level);
            let shown: Vec<Choice<'a>> = if text || choices.len() <= LONG_SELECT {
                choices.clone()
            } else {
                choices
                    .iter()
                    .copied()
                    .filter(|choice| choice.option.selected == Some(true))
                    .collect()
            };
            for choice in &shown {
                let keep = Keep {
                    r#ref: false,
                    urls: false,
                    text: choice.option.text,
                    hidden: choice.hidden,
                };
                self.emit_element_lines(node_line(&choice.option, keep), level + 1, false);
            }
            if shown.len() < choices.len() {
                let more = choices.len() - shown.len();
                let body = format!("- text: {more} more options");
                self.emit_raw_element_line(level + 1, &body, false);
            }
        }
        // Nested controls get lines of their own; the control's own text is not printed again.
        self.content(id, node, level, false, hidden.beside(node));
    }

    /// Commits any tentative ancestor blocks, flushes pending lines, and streams upstream's lines
    /// for one childless node directly into `self.sink` without intermediate `Line` allocation.
    fn emit_element_lines(&mut self, line: NodeAlone<'_>, level: usize, as_kept: bool) {
        self.commit_all_tentative_and_flush();
        self.scratch_own.clear();
        node_text_into(&line, &mut self.scratch_key, &mut self.scratch_own);
        let has_node_ref = line.r#ref.is_some();
        let needs_text = self.sink.needs_text();
        let mut first_mark = LineMark::default();
        for (index, body) in self.scratch_own.split('\n').enumerate() {
            let has_ref = index == 0 && has_node_ref;
            let serial = self.next_serial;
            self.next_serial += 1;
            self.flushed_raw_lines += 1;
            self.flushed_last_sig = Some((serial, body.len()));
            let text = if needs_text {
                self.scratch_line.clear();
                push_indent(&mut self.scratch_line, level);
                self.scratch_line.push_str(body);
                self.scratch_line.as_str()
            } else {
                ""
            };
            let mark = self.sink.push_line(
                text,
                LineMeta {
                    has_ref,
                    starts_kept: as_kept && index == 0,
                },
            );
            if index == 0 {
                first_mark = mark;
            }
        }
        if as_kept {
            self.sink.end_kept(first_mark);
        }
    }

    /// Emits a native select header (key alone plus `:`) directly into `self.sink`.
    fn emit_committed_header_with_colon(&mut self, line: NodeAlone<'_>, level: usize) {
        self.commit_all_tentative_and_flush();
        let key = NodeAlone {
            text: None,
            url: None,
            placeholder: None,
            ..line
        };
        self.scratch_own.clear();
        node_text_into(&key, &mut self.scratch_key, &mut self.scratch_own);
        self.scratch_own.push(':');
        let serial = self.next_serial;
        self.next_serial += 1;
        self.flushed_raw_lines += 1;
        self.flushed_last_sig = Some((serial, self.scratch_own.len()));
        let text = if self.sink.needs_text() {
            self.scratch_line.clear();
            push_indent(&mut self.scratch_line, level);
            self.scratch_line.push_str(&self.scratch_own);
            self.scratch_line.as_str()
        } else {
            ""
        };
        let _ = self.sink.push_line(
            text,
            LineMeta {
                has_ref: key.r#ref.is_some(),
                starts_kept: false,
            },
        );
    }

    /// Emits one already-rendered element line directly into `self.sink`.
    fn emit_raw_element_line(&mut self, level: usize, body: &str, has_ref: bool) {
        self.commit_all_tentative_and_flush();
        let serial = self.next_serial;
        self.next_serial += 1;
        self.flushed_raw_lines += 1;
        self.flushed_last_sig = Some((serial, body.len()));
        let text = if self.sink.needs_text() {
            self.scratch_line.clear();
            push_indent(&mut self.scratch_line, level);
            self.scratch_line.push_str(body);
            self.scratch_line.as_str()
        } else {
            ""
        };
        let _ = self.sink.push_line(
            text,
            LineMeta {
                has_ref,
                starts_kept: false,
            },
        );
    }

    /// Buffers a tentative block header (key alone, no colon yet) and returns its global raw
    /// index. A line break in the key stays inside this one line.
    fn push_tentative_header(&mut self, line: NodeAlone<'_>, level: usize) -> usize {
        self.flush_settled();
        let key = NodeAlone {
            text: None,
            url: None,
            placeholder: None,
            ..line
        };
        self.scratch_own.clear();
        node_text_into(&key, &mut self.scratch_key, &mut self.scratch_own);
        let body = self.scratch_own.clone();
        self.push_pending(level, body, LineKind::Element, key.r#ref.is_some());
        self.logical_line_count() - 1
    }

    fn comment(&mut self, body: String, level: usize) {
        self.push_pending(level, body, LineKind::Comment, false);
        self.flush_settled();
    }

    fn push_pending(&mut self, level: usize, body: String, kind: LineKind, has_ref: bool) {
        self.pending.push_back(Line {
            level,
            body,
            kind,
            has_ref,
            serial: self.next_serial,
        });
        self.next_serial += 1;
    }

    /// Flushes all pending lines that can no longer be truncated or joined by future text.
    fn flush_settled(&mut self) {
        let first_tentative = self.open_blocks.iter().find_map(|block| match block.state {
            BlockState::Tentative { header_raw_idx } => Some(header_raw_idx),
            BlockState::Committed { .. } => None,
        });
        let stop_raw_idx = if let Some(tentative_idx) = first_tentative {
            let keep_joinable_before = tentative_idx > self.flushed_raw_lines
                && tentative_idx > self.text_boundary
                && self
                    .pending
                    .get(tentative_idx - 1 - self.flushed_raw_lines)
                    .is_some_and(|line| line.kind == LineKind::Text);
            if keep_joinable_before {
                tentative_idx - 1
            } else {
                tentative_idx
            }
        } else {
            let total = self.logical_line_count();
            let keep_trailing_text = total > self.text_boundary
                && self
                    .pending
                    .back()
                    .is_some_and(|line| line.kind == LineKind::Text);
            if keep_trailing_text { total - 1 } else { total }
        };
        while self.flushed_raw_lines < stop_raw_idx {
            let _ = self.flush_one_pending(false);
        }
    }

    /// Commits every tentative block in `self.open_blocks` and flushes all lines before
    /// `stop_raw_idx`, leaving `stop_raw_idx` at `pending[0]` when `stop_raw_idx` lies in
    /// `pending`.
    fn commit_tentative_up_to(&mut self, stop_raw_idx: usize) {
        let mut index = 0;
        while index < self.open_blocks.len() {
            let Some(block) = self.open_blocks.get(index).copied() else {
                break;
            };
            if let BlockState::Tentative { header_raw_idx } = block.state {
                while self.flushed_raw_lines < header_raw_idx {
                    let _ = self.flush_one_pending(false);
                }
                if let Some(header) = self.pending.front_mut() {
                    header.body.push(':');
                }
                let starts_kept = block.kind == BlockKind::Kept;
                if let Some(sink_mark) = self.flush_one_pending(starts_kept)
                    && let Some(slot) = self.open_blocks.get_mut(index)
                {
                    slot.state = BlockState::Committed { sink_mark };
                }
            }
            index += 1;
        }
        while self.flushed_raw_lines < stop_raw_idx {
            let _ = self.flush_one_pending(false);
        }
    }

    fn commit_all_tentative_and_flush(&mut self) {
        let end = self.logical_line_count();
        self.commit_tentative_up_to(end);
    }

    fn flush_all_pending(&mut self) {
        while !self.pending.is_empty() {
            let _ = self.flush_one_pending(false);
        }
    }

    /// Pops `pending[0]`, skips separator-only text lines, and pushes any surviving line to
    /// `self.sink`.
    fn flush_one_pending(&mut self, starts_kept: bool) -> Option<LineMark> {
        let line = self.pending.pop_front()?;
        self.flushed_raw_lines += 1;
        self.flushed_last_sig = Some((line.serial, line.body.len()));
        if separators_only(&line) {
            return None;
        }
        let text = if self.sink.needs_text() {
            self.scratch_line.clear();
            push_indent(&mut self.scratch_line, line.level);
            match line.kind {
                LineKind::Text => text_line(&line.body, &mut self.scratch_line),
                LineKind::Element | LineKind::Comment => self.scratch_line.push_str(&line.body),
            }
            self.scratch_line.as_str()
        } else {
            ""
        };
        Some(self.sink.push_line(
            text,
            LineMeta {
                has_ref: line.has_ref,
                starts_kept,
            },
        ))
    }
}

/// `separatorsOnly`: a text line whose joined text holds only these says nothing. Hacker News
/// printed `- text: "|"` between its links, 12% of its front page. Currency signs, `%`, `@`, `#`,
/// `*`, `+`, `=`, `?`, `!` and `&` are not here: beside a field they mean something.
///
/// `/^[\s|·•‧∙/\\\-–—‐›»«‹><:;,.()[\]{}]*$/`, character by character. Every character in the
/// class is one UTF-16 unit, so testing characters is testing the units JavaScript tests.
fn separators_only(line: &Line) -> bool {
    line.kind == LineKind::Text
        && line.body.chars().all(|character| {
            js::is_space(character)
                || matches!(
                    character,
                    '|' | '\u{b7}'
                        | '\u{2022}'
                        | '\u{2027}'
                        | '\u{2219}'
                        | '/'
                        | '\\'
                        | '-'
                        | '\u{2013}'
                        | '\u{2014}'
                        | '\u{2010}'
                        | '\u{203a}'
                        | '\u{bb}'
                        | '\u{ab}'
                        | '\u{2039}'
                        | '>'
                        | '<'
                        | ':'
                        | ';'
                        | ','
                        | '.'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                )
        })
}

/// One option of a native select, and what its line inherits of an `aria-hidden` node above it.
#[derive(Clone, Copy)]
struct Choice<'a> {
    option: EmittedNode<'a>,
    hidden: Hidden,
}

/// `nativeOptions`: a native select's options, which carry no refs, in document order.
/// `<optgroup>`s are `group` nodes, which print no line, so their options are collected through
/// them, each carrying a hidden group's mark. An empty ref counts as none here, as the
/// TypeScript asks `!child.ref`. `hidden` is what the select's own children inherit.
fn native_options<'a, S: AriaSource<'a>>(
    source: &S,
    select: S::Id,
    hidden: Hidden,
    on_top: &OnTop<S::Id, S::Key>,
) -> Vec<Choice<'a>> {
    let mut options = Vec::new();
    // The TypeScript recurses into each group in place; this stack visits in the same order.
    let mut pending: Vec<(SourceChild<'a, S::Id>, Hidden)> = on_top
        .children(source, select)
        .rev()
        .map(|child| (child, hidden))
        .collect();
    while let Some((child, hidden)) = pending.pop() {
        let SourceChild::Node(id) = child else {
            continue;
        };
        let node = source.node(id);
        let ref_empty = node.r#ref.is_none_or(|reference| reference.is_empty());
        if node.role == "option" && ref_empty {
            options.push(Choice {
                option: node,
                hidden,
            });
        } else if node.role == "group" && !actionable(&node) && node.has_children {
            let inside = hidden.beside(&node);
            pending.extend(
                on_top
                    .children(source, id)
                    .rev()
                    .map(|child| (child, inside)),
            );
        }
    }
    options
}

/// `clip`: at most `max` UTF-16 units and `…`, never ending in half a character, the cut's
/// trailing whitespace trimmed as JavaScript trims it.
fn clip(text: &str, max: usize) -> String {
    if js::units(text) <= max {
        return text.to_owned();
    }
    format!(
        "{}…",
        js::trim_end(js::prefix_between_characters(text, max))
    )
}

fn partial_comment(frame: &PartialFrame) -> String {
    format!("# partial: {} not read ({})", frame.frame, frame.reason)
}
