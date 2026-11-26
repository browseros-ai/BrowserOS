use std::mem;

use crate::aria_node::AriaNode;
use crate::cap::{Printed, Section};
use crate::js;
use crate::playwright::{NodeAlone, node_text, text_line};
use crate::snapshot::PartialFrame;

use super::hidden::Hidden;
use super::node_line::{Keep, node_line};
use super::on_top::OnTop;
use super::own_text::{name_as_content, own_text};
use super::rule::{Child, FIELD, Rule, actionable, kept_ref, named, section_label, shown_text};
use super::walk::Walked;

/// A nameless control's own text, or a link's url as its label, is clipped to this many UTF-16
/// units when the page's text is left out.
const OWN_TEXT_CLIP: usize = 80;

/// Past this many options, a native select lists only its selected ones when the page's text is
/// left out.
const LONG_SELECT: usize = 12;

/// The walk of one form of Neo's default format (`Printer` in `snapshot-v1.ts`): it turns the
/// tree into lines by the format's rules, a child at a time.
///
/// The steps wait on a stack of their own rather than the call stack, as in the TypeScript: real
/// pages nest deeper than a recursive walk allows, and a block's closing step must run after all
/// of its content. A block's line is written when it opens and edited when it closes (its colon,
/// its leaf form, or its removal when nothing printed inside it), so a line's index can be taken
/// back and pushed again by another line.
pub(super) struct Printer<'a, 't> {
    lines: Vec<Line>,
    /// The walk's pending steps; the next one is last.
    steps: Vec<Step<'a>>,
    /// Text pieces join only lines at or past this index: the page after a whole-view layer's
    /// content starts a new text run even at the same level.
    text_boundary: usize,
    urls: bool,
    /// The frames not read whose iframe line has not been printed.
    partials: Partials<'a>,
    inert: Inert<'a, 't>,
    on_top: &'t OnTop<'a>,
    /// The serial the next line gets.
    next_serial: usize,
}

/// What a printer does with a subtree the page marked inert.
pub(super) enum Inert<'a, 't> {
    /// Prints it like any other: under `{ inert: true }`, and on an omission's own printer.
    Listed,
    /// Prints it on printers of its own, which count it for the `# not listed:` line.
    Omitted(Omissions<'a, 't>),
}

impl Inert<'_, '_> {
    fn listed(&self) -> bool {
        matches!(self, Self::Listed)
    }
}

/// The inert subtrees a printer left out, rendered as the walk met them.
#[derive(Default)]
pub(super) struct Omissions<'a, 't> {
    /// How many inert roots were left out.
    skipped: usize,
    /// One printer per run of omissions: back-to-back omissions with nothing printed between
    /// them join, as their text did in the page.
    segments: Vec<Printer<'a, 't>>,
    /// Where the last omission was met, to tell whether the next one is back to back.
    boundary: Option<Boundary>,
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

/// One line before its indentation.
struct Line {
    level: usize,
    /// A text line's raw joined pieces, quoted only when the walk is done; any other line as
    /// printed, which for a block's first line may hold a line break inside a quoted key.
    body: String,
    kind: LineKind,
    has_ref: bool,
    section: Option<Section>,
    kept_end: Option<usize>,
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
enum Step<'a> {
    Node {
        child: Child<'a>,
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
        node: &'a AriaNode,
        header: usize,
        level: usize,
        hidden: Hidden,
    },
    /// Closes a section after its content.
    Section { node: &'a AriaNode, header: usize },
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

impl<'a, 't> Printer<'a, 't> {
    pub(super) fn new(
        urls: bool,
        partials: Partials<'a>,
        inert: Inert<'a, 't>,
        on_top: &'t OnTop<'a>,
    ) -> Self {
        Self {
            lines: Vec::new(),
            steps: Vec::new(),
            text_boundary: 0,
            urls,
            partials,
            inert,
            on_top,
            next_serial: 0,
        }
    }

    /// Prints `children` at `level`, depth first in document order.
    pub(super) fn walk(
        &mut self,
        children: impl DoubleEndedIterator<Item = Child<'a>>,
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
                    header,
                    level,
                    hidden,
                } => self.close_kept(node, header, level, hidden),
                Step::Section { node, header } => self.close_section(node, header),
            }
        }
    }

    /// A whole-view layer's content and the page after it are separate text runs even at the
    /// same level.
    pub(super) fn separate_text(&mut self) {
        self.text_boundary = self.lines.len();
    }

    /// The frames whose iframe printed no line, at the top level, in the order of their refs.
    pub(super) fn comment_unplaced_partials(&mut self) {
        for (_, frames) in mem::take(&mut self.partials.0) {
            for frame in frames {
                self.comment(partial_comment(frame), 0);
            }
        }
    }

    /// The lines as printed, and those of the inert subtrees left out, once the walk is done.
    pub(super) fn finish(self) -> Walked {
        let (omitted, skipped) = match self.inert {
            Inert::Listed => (Vec::new(), 0),
            Inert::Omitted(omissions) => (
                omissions
                    .segments
                    .into_iter()
                    .flat_map(|segment| printed(segment.lines))
                    .collect(),
                omissions.skipped,
            ),
        };
        Walked {
            lines: printed(self.lines),
            omitted,
            skipped,
        }
    }

    /// One child of the node being walked, by the first rule that matches it. `hidden` is what its
    /// line inherits of an `aria-hidden` node above it.
    fn node(&mut self, child: Child<'a>, level: usize, text: bool, hidden: Hidden) {
        if let Child::Node(node) = child
            && node.inert == Some(true)
            && !self.inert.listed()
        {
            self.omit(node, level, text, hidden);
            return;
        }
        match Rule::of(child) {
            Rule::Piece(raw) => self.piece(raw, level, text),
            Rule::Kept(node) => self.kept(node, level, hidden),
            Rule::Section(node) => self.section(node, level, text, hidden),
            Rule::Heading(node) => self.heading(node, level, hidden),
            Rule::Control(node) => self.control(node, level, text, hidden),
            Rule::Image(node) => {
                if text && named(node.name.as_deref()).is_some() {
                    let keep = Keep {
                        r#ref: false,
                        urls: false,
                        text: None,
                        hidden,
                    };
                    self.element(node_line(node, keep), level);
                }
                // No line of its own: its text and children belong to the level it sits in.
                self.content(node, level, text, hidden.beside(node));
            }
            Rule::Content(node) => {
                if let Some(named) = name_as_content(node, self.on_top) {
                    self.piece(named, level, text);
                }
                self.content(node, level, text, hidden.beside(node));
            }
        }
    }

    /// Leaves out an inert subtree: it is printed on the printer of its run of omissions, with
    /// the text form and the mark it inherits here, and its frames not read go with it.
    fn omit(&mut self, node: &'a AriaNode, level: usize, text: bool, hidden: Hidden) {
        let Inert::Omitted(omissions) = &mut self.inert else {
            return;
        };
        omissions.skipped += 1;
        let boundary = Boundary {
            last: self.lines.last().map(|line| (line.serial, line.body.len())),
            lines: self.lines.len(),
            level,
        };
        // A printed line or text piece between two omissions separates them, as it did in the
        // page; so does a change of level.
        if omissions.boundary != Some(boundary) {
            omissions.segments.push(Printer::new(
                self.urls,
                Partials::default(),
                Inert::Listed,
                self.on_top,
            ));
        }
        if let Some(segment) = omissions.segments.last_mut() {
            segment.walk([Child::Node(node)].into_iter(), level, text, hidden);
        }
        omissions.boundary = Some(boundary);
        // An omitted iframe owns its frames' `# partial:` lines, so none may reappear later.
        let on_top = self.on_top;
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            if let Some(iframe) = named(node.r#ref.as_deref()) {
                self.partials.take(iframe);
            }
            pending.extend(on_top.children(node).filter_map(|child| match child {
                Child::Node(node) => Some(node),
                Child::Text(_) => None,
            }));
        }
    }

    /// Schedules a node's own text (none for an iframe: `shown_text`), then its children, as
    /// the next steps. Children go on in reverse so they come off in document order, ahead of the
    /// siblings already waiting.
    fn content(&mut self, node: &'a AriaNode, level: usize, text: bool, hidden: Hidden) {
        let on_top = self.on_top;
        for child in on_top.children(node).rev() {
            self.steps.push(Step::Node {
                child,
                level,
                text,
                hidden,
            });
        }
        if let Some(raw) = shown_text(node) {
            self.steps.push(Step::Piece { raw, level, text });
        }
    }

    /// A text piece joins the text line printed last, when that is at the same level, with one
    /// space. Pieces stay raw until the walk is done, so YAML's quoting covers the whole line.
    fn piece(&mut self, raw: &str, level: usize, text: bool) {
        if !text || raw.is_empty() {
            return;
        }
        let joins = self.lines.len() > self.text_boundary;
        if let Some(last) = self.lines.last_mut()
            && joins
            && last.kind == LineKind::Text
            && last.level == level
        {
            last.body.push(' ');
            last.body.push_str(raw);
            return;
        }
        self.push(level, raw.to_owned(), LineKind::Text, false);
    }

    fn kept(&mut self, node: &'a AriaNode, level: usize, hidden: Hidden) {
        // The cap may print the block outside the sections around it.
        let hidden = hidden.movable();
        let keep = Keep {
            r#ref: kept_ref(node),
            urls: false,
            text: None,
            hidden,
        };
        let header = self.header(node_line(node, keep), level);
        self.steps.push(Step::Kept {
            node,
            header,
            level,
            hidden,
        });
        // Its text is what tells the agent what its buttons do, so it prints whatever the form.
        self.content(node, level + 1, true, hidden.under(node));
    }

    /// After a kept block's content: the leaf form, a colon, or no line at all.
    fn close_kept(&mut self, node: &'a AriaNode, header: usize, level: usize, hidden: Hidden) {
        let only_text = match self.lines.get(header + 1..).unwrap_or_default() {
            [only] if only.kind == LineKind::Text && !separators_only(only) => {
                Some(only.body.clone())
            }
            _ => None,
        };
        if let Some(only_text) = only_text {
            // All of its content is one piece of text: the leaf form, `- alert: Invalid password.`
            self.lines.truncate(header);
            let keep = Keep {
                r#ref: kept_ref(node),
                urls: false,
                text: Some(&only_text),
                hidden,
            };
            self.element(node_line(node, keep), level);
        } else if !self.close(header, named(node.name.as_deref()).is_some()) {
            return;
        }
        let end = self.lines.len();
        if let Some(line) = self.lines.get_mut(header) {
            line.kept_end = Some(end);
        }
    }

    fn section(&mut self, node: &'a AriaNode, level: usize, text: bool, hidden: Hidden) {
        let keep = Keep {
            r#ref: true,
            urls: false,
            text: None,
            hidden,
        };
        let header = self.header(node_line(node, keep), level);
        if node.role == "iframe"
            && let Some(iframe) = node.r#ref.as_deref()
        {
            // Under its iframe line, as Playwright's format places it.
            for frame in self.partials.take(iframe) {
                self.comment(partial_comment(frame), level + 1);
            }
        }
        self.steps.push(Step::Section { node, header });
        self.content(node, level + 1, text, hidden.under(node));
    }

    /// After a section's content: its colon, or no line when nothing printed inside it. An
    /// iframe keeps its line with nothing inside, for the `# partial:` comment under it.
    fn close_section(&mut self, node: &'a AriaNode, header: usize) {
        if !self.close(header, node.role == "iframe") {
            return;
        }
        let end = self.lines.len();
        if let Some(line) = self.lines.get_mut(header)
            && line.body.ends_with(':')
            && let Some(r#ref) = &node.r#ref
        {
            line.section = Some(Section {
                r#ref: r#ref.clone(),
                label: section_label(node),
                end,
            });
        }
    }

    fn heading(&mut self, node: &'a AriaNode, level: usize, hidden: Hidden) {
        let own = if named(node.name.as_deref()).is_some() {
            String::new()
        } else {
            own_text(node, self.inert.listed(), self.on_top)
        };
        let keep = Keep {
            r#ref: actionable(node),
            urls: false,
            text: named(Some(own.as_str())),
            hidden,
        };
        self.element(node_line(node, keep), level);
        // A link inside the heading gets its own line; the heading's text is not repeated.
        self.content(node, level, false, hidden.beside(node));
    }

    fn control(&mut self, node: &'a AriaNode, level: usize, text: bool, hidden: Hidden) {
        let name = node.name.as_deref();
        let value = if let Some(own) = node.text.as_deref()
            && Some(own) != name
        {
            // A text that differs from the name: how a text box shows its value.
            Some(own.to_owned())
        } else if FIELD.contains(&node.role.as_str()) && node.text.is_none() {
            // A dropdown's or an editor's value can be its children instead of a `text`. It is
            // kept on the field's line before the children's walk leaves out repeated text.
            let own = own_text(node, self.inert.listed(), self.on_top);
            (!own.is_empty() && Some(own.as_str()) != name).then_some(own)
        } else if named(name).is_none() {
            // A link with no words of its own still says where it goes, unless its `/url` line
            // does.
            let mut label = own_text(node, self.inert.listed(), self.on_top);
            if label.is_empty() && node.role == "link" && !self.urls {
                label = node.url.clone().unwrap_or_default();
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
            native_options(node, hidden.under(node), self.on_top)
        } else {
            Vec::new()
        };
        if choices.is_empty() {
            self.element(line, level);
        } else {
            // A native select: its options carry no refs; they are its value and its choices.
            self.header(line, level);
            if let Some(header) = self.lines.last_mut() {
                header.body.push(':');
            }
            let shown: Vec<Choice<'_>> = if text || choices.len() <= LONG_SELECT {
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
                    text: choice.option.text.as_deref(),
                    hidden: choice.hidden,
                };
                self.element(node_line(choice.option, keep), level + 1);
            }
            if shown.len() < choices.len() {
                let more = choices.len() - shown.len();
                self.push(
                    level + 1,
                    format!("- text: {more} more options"),
                    LineKind::Element,
                    false,
                );
            }
        }
        // Nested controls get lines of their own; the control's own text is not printed again.
        self.content(node, level, false, hidden.beside(node));
    }

    /// Upstream's lines for one childless node, at `level`.
    fn element(&mut self, line: NodeAlone<'_>, level: usize) {
        let mut rendered = String::new();
        node_text(&line, &mut rendered);
        for (index, body) in rendered.split('\n').enumerate() {
            let has_ref = index == 0 && line.r#ref.is_some();
            self.push(level, body.to_owned(), LineKind::Element, has_ref);
        }
    }

    /// A block's first line, its key alone, whose colon `close` adds; returns its index. A
    /// block's text and props are its content, not its line. A line break in the key stays
    /// inside this one line.
    fn header(&mut self, line: NodeAlone<'_>, level: usize) -> usize {
        let key = NodeAlone {
            text: None,
            url: None,
            placeholder: None,
            ..line
        };
        let mut body = String::new();
        node_text(&key, &mut body);
        self.push(level, body, LineKind::Element, key.r#ref.is_some());
        self.lines.len() - 1
    }

    fn comment(&mut self, body: String, level: usize) {
        self.push(level, body, LineKind::Comment, false);
    }

    fn push(&mut self, level: usize, body: String, kind: LineKind, has_ref: bool) {
        self.lines.push(Line {
            level,
            body,
            kind,
            has_ref,
            section: None,
            kept_end: None,
            serial: self.next_serial,
        });
        self.next_serial += 1;
    }

    /// Ends the block opened at `header`: a colon when an element or text line printed inside
    /// it. A text line of separators alone is final here (nothing after the block joins a line
    /// inside it) and does not count. With nothing printed inside, its line stays only when
    /// `keep_empty` or a comment hangs under it; otherwise it goes, so the text before and after
    /// it can join as if it was never there. Says whether it stayed.
    fn close(&mut self, header: usize, keep_empty: bool) -> bool {
        let inside = self.lines.get(header + 1..).unwrap_or_default();
        if inside
            .iter()
            .any(|line| line.kind != LineKind::Comment && !separators_only(line))
        {
            if let Some(line) = self.lines.get_mut(header) {
                line.body.push(':');
            }
            return true;
        }
        if keep_empty || inside.iter().any(|line| line.kind == LineKind::Comment) {
            return true;
        }
        self.lines.truncate(header);
        false
    }
}

/// The lines as printed (`finish` and `printedLines`): a separator-only text line is judged only
/// now, on its joined text, since a price arrives as `$`, `7`, `.`, `49` and `$` alone is no
/// reason to drop it while its line can still grow. Block ends are indices into these lines, so
/// each moves back by the lines dropped before it. Then each line is indented, and a text line
/// printed as upstream prints a text node.
fn printed(lines: Vec<Line>) -> Vec<Printed> {
    let mut moved = Vec::with_capacity(lines.len() + 1);
    let mut kept: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        moved.push(kept.len());
        if !separators_only(&line) {
            kept.push(line);
        }
    }
    moved.push(kept.len());
    let move_end = |end: usize| moved.get(end).copied().unwrap_or(kept.len());
    let ends: Vec<(Option<usize>, Option<usize>)> = kept
        .iter()
        .map(|line| {
            (
                line.section.as_ref().map(|section| move_end(section.end)),
                line.kept_end.map(move_end),
            )
        })
        .collect();
    kept.into_iter()
        .zip(ends)
        .map(|(line, (section_end, kept_end))| {
            let mut text = "  ".repeat(line.level);
            match line.kind {
                LineKind::Text => text_line(&line.body, &mut text),
                LineKind::Element | LineKind::Comment => text.push_str(&line.body),
            }
            Printed {
                text,
                has_ref: line.has_ref,
                section: line
                    .section
                    .zip(section_end)
                    .map(|(section, end)| Section { end, ..section }),
                kept_end,
            }
        })
        .collect()
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
    option: &'a AriaNode,
    hidden: Hidden,
}

/// `nativeOptions`: a native select's options, which carry no refs, in document order.
/// `<optgroup>`s are `group` nodes, which print no line, so their options are collected through
/// them, each carrying a hidden group's mark. An empty ref counts as none here, as the
/// TypeScript asks `!child.ref`. `hidden` is what the select's own children inherit.
fn native_options<'a>(select: &'a AriaNode, hidden: Hidden, on_top: &OnTop<'a>) -> Vec<Choice<'a>> {
    let mut options = Vec::new();
    // The TypeScript recurses into each group in place; this stack visits in the same order.
    let mut pending: Vec<(Child<'a>, Hidden)> = on_top
        .children(select)
        .rev()
        .map(|child| (child, hidden))
        .collect();
    while let Some((child, hidden)) = pending.pop() {
        let Child::Node(node) = child else {
            continue;
        };
        if node.role == "option" && named(node.r#ref.as_deref()).is_none() {
            options.push(Choice {
                option: node,
                hidden,
            });
        } else if node.role == "group" && !actionable(node) {
            let inside = hidden.beside(node);
            pending.extend(on_top.children(node).rev().map(|child| (child, inside)));
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
