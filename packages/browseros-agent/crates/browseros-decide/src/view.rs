//! A typed view of one page, built from the accessibility snapshot the browser
//! already produces.
//!
//! There are two ways in, and the difference matters.
//!
//! [`PageView::from_ax`] reads the typed nodes directly, which is the shape this
//! wants: a role, a name, a value and the state properties, already computed.
//!
//! [`PageView::from_snapshot`] exists because the browser does not hand those
//! nodes out. They are consumed while the snapshot is rendered, and retaining
//! them would mean changing the capture path every tool shares. So this reads
//! the rendered line for each reference instead. That is reading our own
//! renderer's output, whose format this crate does not guess at, rather than
//! reading the page; but it is still a parse, and the typed path is the one to
//! prefer if the capture ever exposes its nodes.
//!
//! Two hashes come out of it. A page fingerprint covers everything a decision
//! was made about, and a per-control guard covers one control. Both exist so a
//! decision can be checked against the page it was made for before it is acted
//! on: a page that changed underneath a decision invalidates it, while a change
//! somewhere else on the page does not.

use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use browseros_core::snapshot::{AxNode, RefMap};

/// Roles that can be acted on, and so can be offered as a candidate.
///
/// A role not listed here still appears in the page text; it is simply not
/// something an operation can target.
const ACTIONABLE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "tab",
    "treeitem",
];

/// Roles that hold text a caller can type into.
const EDITABLE_ROLES: &[&str] = &["textbox", "searchbox", "spinbutton", "combobox"];

/// What a control currently is, as distinct from what it is called.
///
/// Carried as typed fields rather than a list of words, because the questions
/// built from this need to read "is this already checked" without parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlState {
    pub checked: Option<bool>,
    /// Some of a group is selected and some is not.
    ///
    /// Its own state rather than folded into `checked`, because the model is
    /// documented as doing better with a named situation than a raw value, and
    /// collapsing it to unchecked told the model "none of these are selected",
    /// which is not what the page says. A goal of clearing the selection and a
    /// goal of selecting everything then read identically.
    pub indeterminate: bool,
    pub expanded: Option<bool>,
    pub selected: Option<bool>,
    pub disabled: bool,
    pub required: bool,
    pub editable: bool,
}

impl ControlState {
    fn from_ax(node: &AxNode, role: &str) -> Self {
        let mut state = Self {
            editable: EDITABLE_ROLES.contains(&role),
            ..Self::default()
        };
        for property in node.properties.as_deref().unwrap_or(&[]) {
            let value = property.value.value.as_ref();
            match property.name.as_str() {
                "checked" => match value {
                    Some(serde_json::Value::String(mixed)) if mixed == "mixed" => {
                        state.indeterminate = true;
                    }
                    other => state.checked = other.and_then(serde_json::Value::as_bool),
                },
                "expanded" => state.expanded = value.and_then(serde_json::Value::as_bool),
                "selected" => state.selected = value.and_then(serde_json::Value::as_bool),
                "disabled" => {
                    state.disabled = value == Some(&serde_json::Value::Bool(true));
                }
                "required" => {
                    state.required = value == Some(&serde_json::Value::Bool(true));
                }
                "readonly" if value == Some(&serde_json::Value::Bool(true)) => {
                    state.editable = false;
                }
                _ => {}
            }
        }
        state
    }

    /// A short description of the state, for a candidate's description.
    ///
    /// Words rather than flags here, because this half is read by the model and
    /// "unchecked" carries more than `false` does.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        if self.indeterminate {
            return Some("partially selected: some of this group, not all".to_string());
        }
        if let Some(checked) = self.checked {
            return Some(if checked { "checked" } else { "unchecked" }.to_string());
        }
        if let Some(selected) = self.selected {
            return Some(if selected { "selected" } else { "not selected" }.to_string());
        }
        if let Some(expanded) = self.expanded {
            return Some(if expanded { "expanded" } else { "collapsed" }.to_string());
        }
        None
    }
}

/// One thing on the page an operation could target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// The reference the act tool already understands.
    pub reference: String,
    /// The page gives this a pointer cursor, so a person would click it even
    /// though its role is not one of the interactive ones.
    ///
    /// Sites build facets as a styled label over a hidden input, and the thing
    /// a user clicks then has a role like `LabelText` or `generic`. The
    /// renderer already marks those, and ignoring the mark meant a whole filter
    /// panel was visible in the snapshot and unusable: named, referenced, and
    /// filtered out one layer later as not actionable.
    pub clickable: bool,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub state: ControlState,
    /// The selectable values, for a control that has a fixed set of them.
    ///
    /// This is the field that makes a dropdown reachable. A native select draws
    /// its options outside the document, so no click can land on one; naming the
    /// values here lets the value be chosen as an argument instead.
    pub options: Vec<String>,
    /// Covers this control alone. Compared before acting on it.
    pub guard: u64,
    /// The label of the repeated block this control sits in, where the page
    /// has one.
    ///
    /// A listing page calls every one of its buttons "Add to cart", and the
    /// thing that tells them apart is the card each sits in. Without it the
    /// only way to stop them splitting the answer is to drop all but one,
    /// which discards sixty-four real targets and picks the survivor by word
    /// overlap. With it they are sixty-five options the model can choose
    /// between.
    ///
    /// Absent on the typed path, which has no tree to walk.
    pub group: Option<String>,
}

impl Control {
    /// Whether this control can be offered at all.
    ///
    /// Disabled controls are dropped here rather than offered and refused later,
    /// because an option the model cannot usefully pick still costs tokens and
    /// still competes for the answer.
    #[must_use]
    pub fn is_offerable(&self) -> bool {
        if self.state.disabled {
            return false;
        }
        if self.name.is_empty() && self.options.is_empty() {
            return false;
        }
        ACTIONABLE_ROLES.contains(&self.role.as_str()) || self.clickable
    }
}

/// One page, as a decision needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    pub url: String,
    pub title: String,
    /// Visible text. Budgeted by the caller before it travels.
    pub text: String,
    pub controls: Vec<Control>,
    /// Covers the whole page. A decision is made against one of these.
    pub fingerprint: u64,
    /// Covers the arrangement of the page rather than everything on it.
    ///
    /// The fingerprint answers "is this decision still about this page", and it
    /// has to include the text to do that. The page's memory asks a different
    /// question, "is this the same page I was just looking at", and keying that
    /// on the text throws the memory away whenever a character changes.
    ///
    /// A commercial page rewrites one every second. Two snapshots of the same
    /// home page taken a second apart differ only in a deal countdown reading
    /// `Ends in 08:54:33` and then `08:54:32`. Measured there, ten snapshots
    /// one second apart produced ten distinct fingerprints, and the note saying
    /// a control had been tried and did nothing reached the model zero times
    /// across fifteen requests of one run.
    pub arrangement: u64,
    /// Images on the page carrying no accessible name.
    ///
    /// This model reads an accessibility tree, so an image with no name says
    /// nothing to it at all: whatever that picture conveys is invisible to
    /// every decision made here. Counted rather than acted on, because the
    /// caller already has eyes and already owns the screenshot tool for this
    /// page, and an image attached to every hand-back would spend its context
    /// whether or not it asked.
    ///
    /// Counted over the whole snapshot, before the text budget trims it, since
    /// the question is what the page contains and not what happened to fit.
    pub unlabelled_images: usize,
}

impl PageView {
    /// Builds the view from a snapshot's nodes and the refs minted for them.
    ///
    /// The join is on the backend node id, which both sides already carry, so
    /// neither representation needs to change to support this.
    #[must_use]
    pub fn from_ax(
        url: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
        nodes: &[AxNode],
        refs: &RefMap,
    ) -> Self {
        let by_backend: HashMap<i64, &AxNode> = nodes
            .iter()
            .filter_map(|node| node.backend_dom_node_id.map(|id| (id, node)))
            .collect();
        let by_node_id: HashMap<&str, &AxNode> = nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node))
            .collect();

        let mut controls = Vec::new();
        for entry in refs.entries_in_order() {
            let Some(node) = by_backend.get(&entry.backend_node_id) else {
                continue;
            };
            let role = text_of(node.role.as_ref()).unwrap_or_else(|| entry.role.clone());
            if !ACTIONABLE_ROLES.contains(&role.as_str()) {
                continue;
            }
            let name = text_of(node.name.as_ref()).unwrap_or_else(|| entry.name.clone());
            let state = ControlState::from_ax(node, &role);
            let control = Control {
                reference: entry.ref_id.to_string(),
                // The typed path has no cursor information to read, so a
                // control reaches this only by its role.
                clickable: false,
                role: role.clone(),
                name,
                value: text_of(node.value.as_ref()).filter(|value| !value.is_empty()),
                options: options_of(node, &by_node_id),
                state,
                guard: 0,
                group: None,
                // The typed path has no rendered tree to find a card in.
            };
            controls.push(control);
        }
        for control in &mut controls {
            control.guard = guard_of(control);
        }

        let mut view = Self {
            url: url.into(),
            title: title.into(),
            text: text.into(),
            controls,
            fingerprint: 0,
            arrangement: 0,
            // The typed path has no rendered lines to scan, and the nodes it
            // does have are only the referenced ones, so this is reported as
            // unknown rather than guessed at.
            unlabelled_images: 0,
        };
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
        view.arrangement = arrangement_of(&view.url, &view.controls);
        view
    }

    /// Builds the view from a rendered snapshot and the refs minted for it.
    ///
    /// Each reference appears on exactly one line, in a format the renderer
    /// owns: an indent, the role, an optional quoted name, zero or more
    /// bracketed states, the reference, and an optional quoted value after a
    /// colon. A select's values are the `option` lines nested under it.
    #[must_use]
    pub fn from_snapshot(
        url: impl Into<String>,
        title: impl Into<String>,
        text: &str,
        refs: &RefMap,
    ) -> Self {
        // URLs remain in public snapshots; this decision view uses names and states.
        let text = text
            .split('\n')
            .map(without_link_url)
            .collect::<Vec<_>>()
            .join("\n");
        let lines: Vec<&str> = text.lines().collect();
        let mut by_ref: HashMap<&str, usize> = HashMap::new();
        for (index, line) in lines.iter().enumerate() {
            if let Some(reference) = reference_in(line) {
                by_ref.insert(reference, index);
            }
        }

        let mut controls = Vec::new();
        for entry in refs.entries_in_order() {
            let reference = entry.ref_id.to_string();
            let Some(&index) = by_ref.get(reference.as_str()) else {
                continue;
            };
            let line = lines[index];
            let role = role_in(line).unwrap_or_else(|| entry.role.clone());
            // Role alone is not the test any more: a styled facet label carries
            // a role like `LabelText` and is still what a person clicks. The
            // renderer's pointer mark is the signal, and `is_offerable` applies
            // it.
            let clickable = line.contains("[cursor=pointer]");
            if !ACTIONABLE_ROLES.contains(&role.as_str()) && !clickable {
                continue;
            }
            let states = states_in(line);
            let mut state = ControlState {
                editable: EDITABLE_ROLES.contains(&role.as_str()),
                ..ControlState::default()
            };
            for word in &states {
                match word.as_str() {
                    "checked" => state.checked = Some(true),
                    "unchecked" => state.checked = Some(false),
                    // The renderer's own word for the mixed case.
                    "indeterminate" => state.indeterminate = true,
                    "expanded" => state.expanded = Some(true),
                    "collapsed" => state.expanded = Some(false),
                    "selected" => state.selected = Some(true),
                    "disabled" => state.disabled = true,
                    "required" => state.required = true,
                    _ => {}
                }
            }
            // The renderer prints only the true half of checked, so a checkbox
            // with no annotation is unticked rather than unknown.
            if state.checked.is_none()
                && !state.indeterminate
                && matches!(role.as_str(), "checkbox" | "radio" | "switch")
            {
                state.checked = Some(false);
            }
            controls.push((
                depth_of(line),
                Control {
                    reference,
                    clickable,
                    options: nested_options(&lines, index),
                    role,
                    name: name_in(line).unwrap_or_else(|| entry.name.clone()),
                    value: value_in(line),
                    state,
                    guard: 0,
                    group: card_label(&lines, index),
                },
            ));
        }
        let mut controls = without_nested_twins(controls);
        for control in &mut controls {
            control.guard = guard_of(control);
        }

        let mut view = Self {
            url: url.into(),
            title: title.into(),
            text: text.to_string(),
            controls,
            fingerprint: 0,
            arrangement: 0,
            unlabelled_images: unlabelled_images(&lines),
        };
        // Budget before fingerprinting, so the hash covers the text that
        // travels to the model and not the whole document.
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
        view.arrangement = arrangement_of(&view.url, &view.controls);
        view
    }

    /// Builds a view from parts that are already decided.
    ///
    /// For callers that hold a control list and want to vary the rest, which is
    /// how the page-identity behaviour is checked.
    #[must_use]
    pub fn from_snapshot_parts(
        url: impl Into<String>,
        title: impl Into<String>,
        text: &str,
        mut controls: Vec<Control>,
    ) -> Self {
        for control in &mut controls {
            control.guard = guard_of(control);
        }
        let mut view = Self {
            url: url.into(),
            title: title.into(),
            text: text.to_string(),
            controls,
            fingerprint: 0,
            arrangement: 0,
            unlabelled_images: unlabelled_images(&text.lines().collect::<Vec<_>>()),
        };
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
        view.arrangement = arrangement_of(&view.url, &view.controls);
        view
    }

    /// The control with this reference, if it is still on the page.
    #[must_use]
    pub fn control(&self, reference: &str) -> Option<&Control> {
        self.controls
            .iter()
            .find(|control| control.reference == reference)
    }

    /// Whether a decision made against `fingerprint` is still about this page.
    #[must_use]
    pub fn is_fresh_for(&self, fingerprint: u64) -> bool {
        self.fingerprint == fingerprint
    }

    /// Whether one control is unchanged since a decision chose it.
    ///
    /// Narrower than the page fingerprint on purpose. A live page changes
    /// constantly in places that have nothing to do with the chosen control, and
    /// refusing to act on any of that would never finish anything.
    #[must_use]
    pub fn control_is_fresh(&self, reference: &str, guard: u64) -> bool {
        self.control(reference)
            .is_some_and(|control| control.guard == guard)
    }

    /// Controls that kept their reference and changed their name.
    ///
    /// Which is what a counter moving looks like from here: the cart link is
    /// still `e63` and is called "2 items in cart" where it was called "0 items
    /// in cart". So is "Showing 1-16 of 400", and so is a tab becoming
    /// selected. It is the only generic way to say what an action actually did
    /// without knowing anything about the site.
    ///
    /// `about` is the goal, used only to order the answer: a live page has
    /// countdown timers on it, and a run that moved three things should report
    /// the ones the goal is about rather than the clock.
    #[must_use]
    pub fn what_moved(&self, after: &Self, about: &str, most: usize) -> Vec<Moved> {
        let words = crate::relevance::words(about);
        let mut moved: Vec<(bool, Moved)> = Vec::new();
        for was in self
            .controls
            .iter()
            .filter(|control| control.is_offerable())
        {
            let Some(now) = after.control(&was.reference) else {
                continue;
            };
            if now.name.trim() == was.name.trim() || now.role != was.role {
                continue;
            }
            let about_the_goal = crate::relevance::words(&now.name)
                .iter()
                .any(|word| words.contains(word));
            moved.push((
                about_the_goal,
                Moved {
                    reference: was.reference.clone(),
                    was: was.name.clone(),
                    now: now.name.clone(),
                },
            ));
        }
        // The ones the goal mentions first, page order within each group.
        moved.sort_by_key(|(about_the_goal, _)| !*about_the_goal);
        moved.into_iter().map(|(_, one)| one).take(most).collect()
    }

    /// What is covering this page, if anything.
    ///
    /// A caller told to read a page that a dialog is covering reads the dialog.
    /// So the one thing a reply has to say about a page it is handing over is
    /// whether the page is the thing on screen.
    #[must_use]
    pub fn obstructed(&self) -> Option<String> {
        self.controls
            .iter()
            .find(|control| {
                matches!(control.role.as_str(), "dialog" | "alertdialog")
                    || (control.state.expanded == Some(true)
                        && matches!(control.role.as_str(), "combobox" | "listbox"))
            })
            .map(|control| {
                let what = if control.name.trim().is_empty() {
                    control.role.clone()
                } else {
                    format!("{} {:?}", control.role, control.name)
                };
                format!("a {what} is open over the page")
            })
    }

    /// Whether this page offers a way to search itself.
    ///
    /// Asked when the model has just said nothing listed can advance the goal,
    /// which is the one situation where a page's own search is the obvious next
    /// move and the goal never mentions it.
    #[must_use]
    pub fn can_search_itself(&self) -> bool {
        self.controls
            .iter()
            .filter(|control| control.is_offerable())
            .any(|control| crate::kind::of(control) == crate::kind::Kind::SearchField)
    }

    /// How much page text travels with a decision, in bytes.
    ///
    /// Irrelevant detail in the state measurably degrades the answer as well as
    /// costing input tokens, so this is an accuracy measure as much as a cost
    /// one. Applied where the view is built rather than left to each caller,
    /// because the cost this design claims was measured with it applied.
    pub const TEXT_BUDGET: usize = 1_500;

    /// Trims the text to a byte budget on a line boundary.
    ///
    /// Irrelevant detail in the state measurably degrades the answer, so this is
    /// an accuracy measure as much as a cost one.
    pub fn budget_text(&mut self, budget: usize) {
        if self.text.len() <= budget {
            return;
        }
        // Trimming changes what a decision would be made from, so the
        // fingerprint is recomputed below rather than left describing text that
        // no longer travels.
        // Floor to a character boundary before slicing: a budget that lands
        // mid-character would otherwise panic on the slice itself.
        let ceiling = floor_char_boundary(&self.text, budget);
        let cut = self.text[..ceiling].rfind('\n').unwrap_or(ceiling);
        self.text.truncate(cut);
        self.fingerprint = fingerprint_of(&self.url, &self.title, &self.text, &self.controls);
    }
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn text_of(value: Option<&browseros_core::snapshot::AxValue>) -> Option<String> {
    value
        .and_then(|value| value.value.as_ref())
        .map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
}

/// The selectable values of a control that has a fixed set of them.
///
/// Read from the node's own option children, so the values are the page's and
/// never invented.
fn options_of(node: &AxNode, by_node_id: &HashMap<&str, &AxNode>) -> Vec<String> {
    let role = text_of(node.role.as_ref()).unwrap_or_default();
    if role != "combobox" && role != "listbox" {
        return Vec::new();
    }
    node.child_ids
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|child_id| by_node_id.get(child_id.as_str()))
        .filter(|child| text_of(child.role.as_ref()).as_deref() == Some("option"))
        .filter_map(|child| text_of(child.name.as_ref()))
        .filter(|name| !name.is_empty())
        .collect()
}

/// Hashes are compared only within one run, so a hasher whose output is stable
/// for this build is enough and a cryptographic one would be misleading about
/// the guarantee.
fn guard_of(control: &Control) -> u64 {
    let mut hasher = DefaultHasher::new();
    control.reference.hash(&mut hasher);
    control.role.hash(&mut hasher);
    control.name.hash(&mut hasher);
    control.value.hash(&mut hasher);
    control.options.hash(&mut hasher);
    control.state.checked.hash(&mut hasher);
    control.state.indeterminate.hash(&mut hasher);
    control.state.expanded.hash(&mut hasher);
    control.state.selected.hash(&mut hasher);
    control.state.disabled.hash(&mut hasher);
    hasher.finish()
}

/// Covers everything a decision was made from: where the page was, what it
/// said, and what could be acted on.
///
/// The text is included because the model reads it, and without it a page whose
/// copy changed from a success message to an error, with the url and every
/// control unchanged, counted as the same page. A completion claim formed by
/// reading the first was then accepted against the second.
///
/// The text hashed is the budgeted text, so this covers the slice that actually
/// travelled to the model rather than the whole document. Per-control guards
/// deliberately exclude it: a control's identity must not move because
/// unrelated copy did, or the narrow check becomes as blunt as this one and the
/// loop refuses work for no reason.
/// Covers where the page is and what can be acted on, and nothing that a clock
/// or a carousel rewrites.
///
/// Names cannot be part of it: on the page this was measured against, the
/// countdown lives inside a link's name. What is left is the reference and the
/// role of each control, which is what changes when the page really changes.
fn arrangement_of(url: &str, controls: &[Control]) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    for control in controls {
        control.reference.hash(&mut hasher);
        control.role.hash(&mut hasher);
    }
    hasher.finish()
}

fn fingerprint_of(url: &str, title: &str, text: &str, controls: &[Control]) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    title.hash(&mut hasher);
    text.hash(&mut hasher);
    for control in controls {
        control.guard.hash(&mut hasher);
    }
    hasher.finish()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// A control that kept its reference and changed its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub reference: String,
    pub was: String,
    pub now: String,
}

/// How deeply a snapshot line sits in the page's tree.
///
/// The renderer writes each level as two spaces, so the leading run divides
/// exactly. A line it did not write has no depth to read and counts as the root.
fn depth_of(line: &str) -> usize {
    (line.len() - line.trim_start().len()) / 2
}

/// Drops a control that is the same element as the one it sits inside.
///
/// Markup doubles a control often enough to matter: Google News renders its
/// section tabs as a tab nested directly inside an identical tab, and both
/// reach the snapshot. Offering the pair splits the probability between two
/// options that do the same thing, which on one measured page held the right
/// answer at 0.61 where it needed 0.75.
///
/// The test is nesting, not the name alone. A page with twenty "Add to cart"
/// buttons has them as siblings in separate cards, and those are twenty
/// different targets that all have to survive.
/// Roles that hold a control rather than being one.
///
/// Markup does not oblige a wrapper to share its child's role. An autocomplete
/// nests row inside gridcell inside button, all carrying the same name, and a
/// results page wraps each Add to cart button in a clickable generic. Measured
/// on one live dropdown, a single suggestion held 0.31 as its button and 0.09
/// as its cell, which is 0.40 of the mass for one move.
const WRAPPER_ROLES: [&str; 4] = ["generic", "gridcell", "cell", "row"];

#[must_use]
pub fn without_nested_twins(controls: Vec<(usize, Control)>) -> Vec<Control> {
    // A wrapper holding a control of the same name is not a second target. The
    // control inside it is the one that works, so the wrapper goes.
    let wraps_the_same_thing = |index: usize| -> bool {
        let (depth, me) = &controls[index];
        if !WRAPPER_ROLES.contains(&me.role.as_str()) || me.name.trim().is_empty() {
            return false;
        }
        controls[index + 1..]
            .iter()
            .take_while(|(other, _)| other > depth)
            .any(|(_, inside)| {
                !WRAPPER_ROLES.contains(&inside.role.as_str())
                    && inside.name.trim().eq_ignore_ascii_case(me.name.trim())
            })
    };
    let twin = |index: usize| -> bool {
        let (depth, me) = &controls[index];
        if *depth == 0 || me.name.trim().is_empty() {
            return false;
        }
        // The nearest line above me that is shallower than me is my parent.
        controls[..index]
            .iter()
            .rev()
            .find(|(other, _)| other < depth)
            .is_some_and(|(parent_depth, parent)| {
                *parent_depth == depth - 1
                    && parent.role == me.role
                    && parent.name.trim().eq_ignore_ascii_case(me.name.trim())
            })
    };
    (0..controls.len())
        .filter(|index| !twin(*index) && !wraps_the_same_thing(*index))
        .map(|index| controls[index].1.clone())
        .collect()
}

/// Roles that mark one repeated thing among many of its kind.
const CARD_ROLES: [&str; 3] = ["listitem", "article", "row"];

/// How much of a card's label travels to the model.
///
/// A product title runs to two hundred characters of marketing copy, and that
/// multiplied by a batch of options is most of a question's budget.
const LABEL_BUDGET: usize = 80;

/// Names that sit on every card and so distinguish none of them.
fn says_nothing_about_which_card(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["rating", "review", "out of 5", "stars"]
        .iter()
        .any(|noise| lower.contains(noise))
}

/// The label of the card this control sits in, read from the rendered tree.
///
/// The card is the nearest ancestor whose role is one of [`CARD_ROLES`]. Its
/// label is the heading inside it, failing that the longest link, which on a
/// product card is the title and on a search result is the result itself.
///
/// Two exclusions are not optional, and the second was paid for. Text inside a
/// nested card belongs to that card. Text inside this control's own subtree is
/// content rather than a container: without that rule, a documentation page
/// whose chapters nest described "Understanding Ownership" as sitting inside
/// "References and Borrowing", which is one of its own sections, and the case
/// that had always passed went to zero.
fn card_label(lines: &[&str], index: usize) -> Option<String> {
    let my_depth = depth_of(lines[index]);
    let my_reference = reference_in(lines[index]);

    let mut shallower = my_depth;
    let mut card: Option<(usize, usize)> = None;
    for at in (0..index).rev() {
        let depth = depth_of(lines[at]);
        if depth >= shallower {
            continue;
        }
        shallower = depth;
        if role_in(lines[at]).is_some_and(|role| CARD_ROLES.contains(&role.as_str())) {
            card = Some((at, depth));
            break;
        }
        if depth == 0 {
            break;
        }
    }
    let (card, card_depth) = card?;

    // Where this control's own subtree ends.
    let mine_ends = lines[index + 1..]
        .iter()
        .position(|line| depth_of(line) <= my_depth)
        .map_or(lines.len(), |at| index + 1 + at);

    let mut best: Option<(usize, String)> = None;
    let mut nested_until: Option<usize> = None;
    for (at, line) in lines.iter().enumerate().skip(card + 1) {
        let depth = depth_of(line);
        if depth <= card_depth {
            break;
        }
        if let Some(inner) = nested_until {
            if depth > inner {
                continue;
            }
            nested_until = None;
        }
        let Some(role) = role_in(line) else { continue };
        if CARD_ROLES.contains(&role.as_str()) {
            nested_until = Some(depth);
            continue;
        }
        if at > index && at < mine_ends {
            continue;
        }
        if reference_in(line).is_some() && reference_in(line) == my_reference {
            continue;
        }
        if !matches!(role.as_str(), "heading" | "link") {
            continue;
        }
        let Some(name) = name_in(line) else { continue };
        if name.trim().is_empty() || says_nothing_about_which_card(&name) {
            continue;
        }
        let score = usize::from(role == "heading") * 1000 + name.len();
        if best.as_ref().is_none_or(|(had, _)| score > *had) {
            best = Some((score, name));
        }
    }
    best.map(|(_, name)| trimmed_to_budget(&name))
}

/// Cuts a label to the budget on a word boundary, so it ends as a phrase
/// rather than mid-word.
fn trimmed_to_budget(name: &str) -> String {
    let name = name.trim();
    if name.chars().count() <= LABEL_BUDGET {
        return name.to_string();
    }
    let cut: String = name.chars().take(LABEL_BUDGET).collect();
    match cut.rfind(char::is_whitespace) {
        Some(at) if at > LABEL_BUDGET / 2 => cut[..at].trim_end().to_string(),
        _ => cut.trim_end().to_string(),
    }
}

fn reference_in(line: &str) -> Option<&str> {
    let start = line.find("[ref=")? + "[ref=".len();
    let rest = &line[start..];
    let end = rest.find(']')?;
    Some(&rest[..end])
}

/// How many images on the page carry no accessible name.
///
/// An image with a name has already told the accessibility tree what it is. One
/// without is a picture that only a reader with eyes can use, and counting them
/// is the cheapest honest way to say "some of this page is not in the tree you
/// are reading".
fn unlabelled_images(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| matches!(role_in(line).as_deref(), Some("image" | "img")))
        .filter(|line| name_in(line).is_none_or(|name| name.trim().is_empty()))
        .count()
}

fn role_in(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("- ")?;
    Some(
        rest.split([' ', ':'])
            .next()
            .unwrap_or_default()
            .to_string(),
    )
}

/// Decodes the JSON string starting at `start`, which must be its opening
/// quote, and reports where it ended.
///
/// The renderer writes names and values with `serde_json::to_string`, so they
/// are JSON strings and have to be read as such. Scanning for the next quote
/// character truncates anything containing one: an option named `Say "hi"`
/// became `Say \`, and that truncated value could never match the real option
/// when it was sent back as a select value.
fn json_string_at(line: &str, start: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => {
                let decoded = serde_json::from_str::<String>(line.get(start..=index)?).ok()?;
                return Some((decoded, index + 1));
            }
            _ => index += 1,
        }
    }
    None
}

fn without_link_url(line: &str) -> &str {
    if role_in(line).as_deref() != Some("link") {
        return line;
    }
    let Some(start) = line.rfind(" [url=\"") else {
        return line;
    };
    if json_string_at(line, start + " [url=".len())
        .is_some_and(|(_, end)| line.get(end..) == Some("]"))
    {
        return &line[..start];
    }
    line
}

/// The first quoted run on the line, which the renderer writes as the name.
fn name_in(line: &str) -> Option<String> {
    let start = line.find('"')?;
    json_string_at(line, start).map(|(name, _)| name)
}

/// The bracketed words, excluding the reference and the cursor hint, which are
/// not states.
///
/// Scanned from after the name, so a name containing brackets cannot be read as
/// a state. A control called `Filter [beta]` would otherwise contribute one.
fn states_in(line: &str) -> Vec<String> {
    let mut states = Vec::new();
    let after_name = line
        .find('"')
        .and_then(|start| json_string_at(line, start).map(|(_, end)| end))
        .unwrap_or(0);
    let mut rest = line.get(after_name..).unwrap_or(line);
    while let Some(start) = rest.find('[') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find(']') else { break };
        let word = &rest[..end];
        if !word.starts_with("ref=") && !word.starts_with("cursor=") {
            states.push(word.to_string());
        }
        rest = &rest[end + 1..];
    }
    states
}

/// The quoted value the renderer writes after a colon.
///
/// Decoded as a JSON string for the same reason the name is: a value
/// containing a quote or a backslash is otherwise read wrongly, and this one
/// travels back to the browser as the thing to type or select.
fn value_in(line: &str) -> Option<String> {
    let marker = line.rfind("]: \"").map(|index| index + 3);
    let start = marker.or_else(|| line.rfind(": \"").map(|index| index + 2))?;
    json_string_at(line, start).map(|(value, _)| value)
}

/// The option lines nested directly under a control, which is how a select's
/// values appear in a rendered snapshot.
fn nested_options(lines: &[&str], index: usize) -> Vec<String> {
    let parent_indent = indent_of(lines[index]);
    let mut options = Vec::new();
    for line in lines.iter().skip(index + 1) {
        let indent = indent_of(line);
        if indent <= parent_indent {
            break;
        }
        if role_in(line).as_deref() == Some("option")
            && let Some(name) = name_in(line)
        {
            options.push(name);
        }
    }
    options
}

#[cfg(test)]
mod tests {
    fn named(reference: &str, role: &str, name: &str) -> Control {
        Control {
            reference: reference.to_string(),
            clickable: false,
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            state: ControlState::default(),
            options: Vec::new(),
            guard: 0,
            group: None,
        }
    }

    /// The renderer writes one line per node as `- role "name" [ref=eN]`, and
    /// omits the quoted name when there is none, so an unnamed image is a line
    /// with a role and nothing else. Built in that format rather than from a
    /// live page, because the format is what this reads.
    #[test]
    fn images_without_a_name_are_counted() {
        let text = "- image \"Corsair K70 keyboard\"\n\
                    - image\n\
                    - img\n\
                    - image \"\"\n\
                    - button \"Apply\" [ref=e1]\n\
                    - imagemap \"not an image role\"";
        let view = super::PageView::from_snapshot(
            "https://example.com",
            "Example",
            text,
            &browseros_core::snapshot::RefMap::default(),
        );

        assert_eq!(
            view.unlabelled_images, 3,
            "the two bare roles and the empty name, not the named one and not \
             a role that merely starts with the same letters"
        );
    }

    /// The question is what the page contains, not what happened to fit inside
    /// the text that travels to the model.
    #[test]
    fn the_image_count_is_taken_before_the_text_budget() {
        let filler = "- paragraph \"x\"\n".repeat(400);
        let text = format!("{filler}- image\n- image");
        let view = super::PageView::from_snapshot(
            "https://example.com",
            "Example",
            &text,
            &browseros_core::snapshot::RefMap::default(),
        );

        assert!(
            view.text.len() <= super::PageView::TEXT_BUDGET,
            "the text really was trimmed"
        );
        assert!(
            !view.text.contains("- image"),
            "and the images really did fall outside it"
        );
        assert_eq!(view.unlabelled_images, 2);
    }

    use super::*;
    use browseros_core::snapshot::{AxProperty, AxValue};
    use browseros_core::{FrameId, snapshot::refs::MintRef};

    #[test]
    fn tests_that_urls_leave_page_details_alone() {
        use browseros_core::snapshot::{RenderOptions, render_snapshot};

        let mut nodes = vec![
            ax("1", 1, "RootWebArea", "Shop"),
            ax("2", 2, "listitem", ""),
            ax("3", 3, "link", ""),
            ax("4", 4, "link", "Sensodyne Repair and Protect"),
            ax("5", 5, "button", "Add to cart"),
            with_property(ax("6", 6, "link", ""), "expanded", serde_json::json!(false)),
            ax("7", 7, "link", "Red shoes"),
            ax("8", 8, "link", "Literal [url=\"example\"] label"),
        ];
        nodes[0].child_ids = Some(["2", "6", "7", "8"].map(String::from).into());
        nodes[1].child_ids = Some(["3", "4", "5"].map(String::from).into());

        let page = |nodes: &[AxNode]| {
            let mut refs = RefMap::new();
            let snapshot = render_snapshot(
                nodes,
                &mut RenderOptions {
                    refs: &mut refs,
                    frame_id: None,
                    document_id: None,
                    cursor_hits: None,
                    base_depth: 0,
                },
            );
            PageView::from_snapshot("https://shop.example", "Shop", &snapshot.text, &refs)
        };
        let expected = page(&nodes);
        assert_eq!(
            expected.control("e3").unwrap().group.as_deref(),
            Some("Sensodyne Repair and Protect")
        );
        assert!(!expected.control("e1").unwrap().is_offerable());
        assert_eq!(expected.control("e4").unwrap().state.expanded, Some(false));

        // Long URLs and query brackets must not become names, states or page text.
        let url = format!(
            "https://shop.example/{}?ids[]=1&f[disabled]=1&q=\"hello\"",
            "tracking/".repeat(200)
        );
        for node in nodes.iter_mut().filter(|node| {
            node.role
                .as_ref()
                .and_then(|role| role.value.as_ref())
                .and_then(serde_json::Value::as_str)
                == Some("link")
        }) {
            node.properties
                .get_or_insert_with(Vec::new)
                .push(AxProperty {
                    name: "url".into(),
                    value: AxValue {
                        value_type: "string".into(),
                        value: Some(serde_json::json!(url)),
                    },
                });
        }
        assert_eq!(page(&nodes), expected);
    }

    fn ax(node_id: &str, backend: i64, role: &str, name: &str) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role(role)),
            name: Some(AxValue::string(name)),
            backend_dom_node_id: Some(backend),
            ..AxNode::default()
        }
    }

    fn with_property(mut node: AxNode, name: &str, value: serde_json::Value) -> AxNode {
        node.properties
            .get_or_insert_with(Vec::new)
            .push(AxProperty {
                name: name.to_string(),
                value: AxValue {
                    value_type: "booleanOrUndefined".to_string(),
                    value: Some(value),
                },
            });
        node
    }

    /// Mints a ref per node so the view has something to join against, the same
    /// way rendering a snapshot does.
    fn refs_for(nodes: &[AxNode]) -> RefMap {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        for node in nodes {
            let role = node
                .role
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = node
                .name
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            refs.mint(MintRef {
                backend_node_id: node.backend_dom_node_id.unwrap_or_default(),
                role: &role,
                name: &name,
                document_id: None,
                frame_id: Some(&FrameId::from("frame".to_string())),
            });
        }
        refs
    }

    fn view_of(nodes: &[AxNode]) -> PageView {
        let refs = refs_for(nodes);
        PageView::from_ax("https://example.com", "Example", "", nodes, &refs)
    }

    /// Every state annotation the snapshot layer computes has to survive the
    /// translation, because the questions built from this read them.
    #[test]
    fn every_state_annotation_survives_the_translation() {
        let nodes = vec![
            with_property(ax("1", 11, "checkbox", "Corsair"), "checked", true.into()),
            with_property(ax("2", 12, "checkbox", "Kingston"), "checked", false.into()),
            with_property(ax("3", 13, "button", "Apply"), "disabled", true.into()),
            with_property(ax("4", 14, "button", "Brand"), "expanded", false.into()),
            with_property(ax("5", 15, "option", "Price"), "selected", true.into()),
            with_property(ax("6", 16, "textbox", "Email"), "required", true.into()),
        ];
        let view = view_of(&nodes);
        assert_eq!(view.controls.len(), 6);
        assert_eq!(view.controls[0].state.checked, Some(true));
        assert_eq!(view.controls[1].state.checked, Some(false));
        assert!(view.controls[2].state.disabled);
        assert_eq!(view.controls[3].state.expanded, Some(false));
        assert_eq!(view.controls[4].state.selected, Some(true));
        assert!(view.controls[5].state.required);
        assert!(
            view.controls[5].state.editable,
            "a textbox can be typed into"
        );
    }

    /// A styled facet label is what a person clicks, and its role is not one of
    /// the interactive ones. The renderer marks it with a pointer cursor, and
    /// ignoring that mark meant a whole filter panel reached the snapshot
    /// named and referenced, and was then discarded one layer later.
    #[test]
    fn a_pointer_marked_control_is_offerable_whatever_its_role() {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let minted = refs.mint(MintRef {
            backend_node_id: 30,
            role: "LabelText",
            name: "ADATA",
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let text = format!("- main\n  - LabelText \"ADATA\" [ref={minted}] [cursor=pointer]");
        let view = PageView::from_snapshot("https://e.com", "RAM", &text, &refs);
        let facet = view
            .controls
            .iter()
            .find(|control| control.name == "ADATA")
            .expect("the facet label is a control");
        assert!(facet.clickable);
        assert!(
            facet.is_offerable(),
            "a person would click it, so it can be offered"
        );
    }

    /// Without the mark, a role nothing can act on is still left out, so this
    /// widens the net rather than removing it.
    #[test]
    fn an_unmarked_non_interactive_role_is_still_left_out() {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let minted = refs.mint(MintRef {
            backend_node_id: 31,
            role: "LabelText",
            name: "Just a label",
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let text = format!("- main\n  - LabelText \"Just a label\" [ref={minted}]");
        let view = PageView::from_snapshot("https://e.com", "T", &text, &refs);
        assert!(
            view.controls.is_empty(),
            "no pointer mark and no interactive role means nothing to offer"
        );
    }

    /// And a marked control with no name is still not offerable: a person could
    /// click it, but the model could not say which one it meant.
    #[test]
    fn a_pointer_marked_control_with_no_name_is_not_offerable() {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let minted = refs.mint(MintRef {
            backend_node_id: 32,
            role: "generic",
            name: "",
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let text = format!("- main\n  - generic [ref={minted}] [cursor=pointer]");
        let view = PageView::from_snapshot("https://e.com", "T", &text, &refs);
        assert!(view.controls.iter().all(|control| !control.is_offerable()));
    }

    /// A disabled control is not offered. An option the model cannot usefully
    /// pick still costs tokens and still competes for the answer.
    #[test]
    fn a_disabled_control_is_not_offerable() {
        let nodes = vec![
            with_property(ax("1", 11, "button", "Apply"), "disabled", true.into()),
            ax("2", 12, "button", "Clear"),
        ];
        let view = view_of(&nodes);
        assert!(!view.controls[0].is_offerable());
        assert!(view.controls[1].is_offerable());
    }

    /// A nameless control with nothing selectable about it cannot be described
    /// to the model, so it is not offered either.
    #[test]
    fn a_nameless_control_is_not_offerable() {
        let nodes = vec![ax("1", 11, "button", "")];
        let view = view_of(&nodes);
        assert!(!view.controls[0].is_offerable());
    }

    /// The whole dropdown fix rests on this: the values of a select are read
    /// from the page, so a value can be chosen as an argument rather than
    /// clicked at coordinates where nothing exists.
    #[test]
    fn a_selects_values_are_read_from_its_own_options() {
        let mut select = ax("1", 11, "combobox", "Sort by");
        select.child_ids = Some(vec!["2".to_string(), "3".to_string(), "4".to_string()]);
        let nodes = vec![
            select,
            ax("2", 12, "option", "Featured"),
            ax("3", 13, "option", "Price: Low to High"),
            ax("4", 14, "option", "Newest"),
        ];
        let view = view_of(&nodes);
        let combobox = &view.controls[0];
        assert_eq!(
            combobox.options,
            vec!["Featured", "Price: Low to High", "Newest"]
        );
        assert!(
            combobox.is_offerable(),
            "a select with values is offerable even without a name"
        );
    }

    /// A control that is not a select carries no options, so nothing invents a
    /// closed set where the page does not have one.
    #[test]
    fn a_button_carries_no_options() {
        let nodes = vec![ax("1", 11, "button", "Apply")];
        assert!(view_of(&nodes).controls[0].options.is_empty());
    }

    /// A role nothing can act on is left out of the controls, though it still
    /// appears in the page text.
    #[test]
    fn a_non_actionable_role_is_not_a_control() {
        let nodes = vec![
            ax("1", 11, "heading", "Results"),
            ax("2", 12, "StaticText", "32 GB DDR5"),
            ax("3", 13, "button", "Apply"),
        ];
        let view = view_of(&nodes);
        assert_eq!(view.controls.len(), 1);
        assert_eq!(view.controls[0].name, "Apply");
    }

    /// A guard changes when the thing it keys on changes, and not otherwise.
    /// This is what lets a decision be refused after the control moved under it.
    #[test]
    fn a_guard_changes_only_when_its_control_changes() {
        let base = vec![
            with_property(ax("1", 11, "checkbox", "Corsair"), "checked", false.into()),
            ax("2", 12, "button", "Apply"),
        ];
        let before = view_of(&base);

        let unchanged = view_of(&base);
        assert_eq!(
            before.controls[0].guard, unchanged.controls[0].guard,
            "the same page gives the same guard"
        );

        let ticked = vec![
            with_property(ax("1", 11, "checkbox", "Corsair"), "checked", true.into()),
            ax("2", 12, "button", "Apply"),
        ];
        let after = view_of(&ticked);
        assert_ne!(
            before.controls[0].guard, after.controls[0].guard,
            "ticking the checkbox changes its guard"
        );
        assert_eq!(
            before.controls[1].guard, after.controls[1].guard,
            "and leaves an untouched control's guard alone"
        );
    }

    /// The page fingerprint moves when any control does, which is the coarse
    /// check; the per-control guard above is the one used before acting.
    #[test]
    fn the_page_fingerprint_moves_when_any_control_does() {
        let before = view_of(&[ax("1", 11, "button", "Apply")]);
        let after = view_of(&[ax("1", 11, "button", "Applied")]);
        assert!(!after.is_fresh_for(before.fingerprint));
        assert!(before.is_fresh_for(before.fingerprint));
    }

    /// The hole this closes. A page whose copy changed from a success message
    /// to an error, with the url and every control identical, counted as the
    /// same page, so a completion claim formed from the first was accepted
    /// against the second.
    #[test]
    fn a_page_whose_text_changed_is_not_the_same_page() {
        let nodes = vec![ax("1", 11, "button", "Continue")];
        let refs = refs_for(&nodes);
        let before = PageView::from_ax(
            "https://example.com/cart",
            "Cart",
            "Item added to your cart",
            &nodes,
            &refs,
        );
        let after = PageView::from_ax(
            "https://example.com/cart",
            "Cart",
            "Out of stock",
            &nodes,
            &refs,
        );
        assert_eq!(before.url, after.url);
        assert_eq!(
            before.controls, after.controls,
            "the controls are identical"
        );
        assert!(
            !after.is_fresh_for(before.fingerprint),
            "the text the decision was read from changed, so this is not the same page"
        );
    }

    /// The asymmetry that makes the narrow check worth having. A control's own
    /// guard must not move because unrelated copy did, or refusing a stale
    /// decision would refuse almost every decision.
    #[test]
    fn unrelated_text_does_not_move_a_controls_guard() {
        let nodes = vec![ax("1", 11, "button", "Continue")];
        let refs = refs_for(&nodes);
        let before = PageView::from_ax("https://example.com", "T", "one story", &nodes, &refs);
        let after = PageView::from_ax("https://example.com", "T", "another story", &nodes, &refs);
        assert_eq!(
            before.controls[0].guard, after.controls[0].guard,
            "the control is the same control"
        );
        assert!(
            after.control_is_fresh(&before.controls[0].reference, before.controls[0].guard),
            "so a decision about it is still actionable"
        );
        assert!(
            !after.is_fresh_for(before.fingerprint),
            "while the page as a whole has moved"
        );
    }

    /// The title names the page a claim is about, so it counts too.
    #[test]
    fn a_changed_title_is_not_the_same_page() {
        let nodes = vec![ax("1", 11, "button", "Continue")];
        let refs = refs_for(&nodes);
        let before = PageView::from_ax("https://example.com", "Cart", "same", &nodes, &refs);
        let after = PageView::from_ax("https://example.com", "Checkout", "same", &nodes, &refs);
        assert!(!after.is_fresh_for(before.fingerprint));
    }

    /// The hash covers the text that travels, so trimming further keeps it
    /// honest rather than leaving it describing text the model never saw.
    #[test]
    fn trimming_the_text_moves_the_fingerprint() {
        let nodes = vec![ax("1", 11, "button", "Continue")];
        let refs = refs_for(&nodes);
        let mut view = PageView::from_ax(
            "https://example.com",
            "T",
            "first\nsecond\nthird",
            &nodes,
            &refs,
        );
        let before = view.fingerprint;
        view.budget_text(6);
        assert_ne!(view.fingerprint, before);
        assert_eq!(view.text, "first");
    }

    /// And a page built from a long document is fingerprinted over the budgeted
    /// slice, not the whole thing, so two pages differing only beyond the
    /// budget read as the same page.
    #[test]
    fn text_beyond_the_budget_does_not_affect_the_fingerprint() {
        let nodes = vec![ax("1", 11, "button", "Continue")];
        let refs = refs_for(&nodes);
        let head = "visible\n".repeat(PageView::TEXT_BUDGET / 8 + 10);
        let one = PageView::from_ax(
            "https://e.com",
            "T",
            format!("{head}tail one"),
            &nodes,
            &refs,
        );
        let two = PageView::from_ax(
            "https://e.com",
            "T",
            format!("{head}tail two"),
            &nodes,
            &refs,
        );
        assert_eq!(
            one.fingerprint, two.fingerprint,
            "neither tail reached the model, so neither can move the hash"
        );
    }

    #[test]
    fn freshness_can_be_checked_for_one_control() {
        let view = view_of(&[ax("1", 11, "button", "Apply")]);
        let reference = view.controls[0].reference.clone();
        let guard = view.controls[0].guard;
        assert!(view.control_is_fresh(&reference, guard));
        assert!(!view.control_is_fresh(&reference, guard ^ 1));
        assert!(!view.control_is_fresh("e999", guard));
    }

    /// Text is cut on a line boundary, so a budget never leaves half a line of
    /// a page in the state.
    #[test]
    fn budgeting_text_cuts_on_a_line_boundary() {
        let mut view = view_of(&[]);
        view.text = "first line\nsecond line\nthird line".to_string();
        view.budget_text(20);
        assert_eq!(view.text, "first line");
    }

    #[test]
    fn budgeting_text_under_the_budget_changes_nothing() {
        let mut view = view_of(&[]);
        view.text = "short".to_string();
        view.budget_text(100);
        assert_eq!(view.text, "short");
    }

    /// A budget that lands inside a multi-byte character must not panic.
    #[test]
    fn budgeting_text_respects_character_boundaries() {
        let mut view = view_of(&[]);
        view.text = "ααααααααα".to_string();
        view.budget_text(5);
        assert!(view.text.len() <= 5);
    }

    /// The renderer's own format, read back.
    ///
    /// Built the way production does: the refs are minted first and the lines
    /// are written with the ids that came back, because in a real capture the
    /// same map that minted them produced the text.
    #[test]
    fn a_rendered_snapshot_reads_back_into_controls() {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let mut minted = Vec::new();
        for (backend, role, name) in [
            (34, "checkbox", "Corsair"),
            (35, "checkbox", "Kingston"),
            (36, "button", "Apply"),
            (37, "textbox", "Search"),
            (99, "combobox", "Sort by:"),
            (100, "option", "Featured"),
            (101, "option", "Price: Low to High"),
        ] {
            minted.push(refs.mint(MintRef {
                backend_node_id: backend,
                role,
                name,
                document_id: None,
                frame_id: Some(&FrameId::from("frame".to_string())),
            }));
        }
        let text = format!(
            "- main\n  \
             - checkbox \"Corsair\" [ref={}]\n  \
             - checkbox \"Kingston\" [checked] [ref={}]\n  \
             - button \"Apply\" [disabled] [ref={}]\n  \
             - textbox \"Search\" [required] [ref={}]: \"32gb ddr5\"\n  \
             - combobox \"Sort by:\" [ref={}]\n    \
             - option \"Featured\" [selected] [ref={}]\n    \
             - option \"Price: Low to High\" [ref={}]",
            minted[0], minted[1], minted[2], minted[3], minted[4], minted[5], minted[6]
        );
        let view = PageView::from_snapshot("https://example.com", "Example", &text, &refs);

        let corsair = view
            .controls
            .iter()
            .find(|control| control.name == "Corsair")
            .expect("the unticked checkbox");
        assert_eq!(
            corsair.state.checked,
            Some(false),
            "the renderer prints only the ticked half, so no annotation means unticked"
        );

        let kingston = view
            .controls
            .iter()
            .find(|control| control.name == "Kingston")
            .expect("the ticked checkbox");
        assert_eq!(kingston.state.checked, Some(true));

        let apply = view
            .controls
            .iter()
            .find(|control| control.name == "Apply")
            .expect("the button");
        assert!(apply.state.disabled);
        assert!(!apply.is_offerable());

        let search = view
            .controls
            .iter()
            .find(|control| control.name == "Search")
            .expect("the field");
        assert!(search.state.required);
        assert_eq!(search.value.as_deref(), Some("32gb ddr5"));
        assert!(search.state.editable);

        let sort = view
            .controls
            .iter()
            .find(|control| control.name == "Sort by:")
            .expect("the select");
        assert_eq!(
            sort.options,
            vec!["Featured", "Price: Low to High"],
            "a select's values are the option lines nested under it"
        );
    }

    /// The renderer writes names with serde_json, so a name containing a quote
    /// arrives escaped and has to be decoded. Scanning for the next quote
    /// truncated it, and the truncated value could never match the real option
    /// when it was sent back as a select value.
    #[test]
    fn a_name_containing_a_quote_is_decoded_whole() {
        let name = r#"Say "hi" to everyone"#;
        let line = format!(
            "  - option {} [ref=e5]",
            serde_json::to_string(name).expect("quotes")
        );
        assert_eq!(name_in(&line).as_deref(), Some(name));
    }

    /// And a backslash, which the same scan would have mangled.
    #[test]
    fn a_name_containing_a_backslash_is_decoded_whole() {
        let name = r"C:\Users\shared";
        let line = format!(
            "  - link {} [ref=e6]",
            serde_json::to_string(name).expect("quotes")
        );
        assert_eq!(name_in(&line).as_deref(), Some(name));
    }

    /// A value travels back to the browser as the thing to type or select, so
    /// it has to survive the round trip exactly.
    #[test]
    fn a_value_containing_a_quote_is_decoded_whole() {
        let value = r#"ratio 16:9 "widescreen""#;
        let line = format!(
            "  - textbox \"Note\" [ref=e8]: {}",
            serde_json::to_string(value).expect("quotes")
        );
        assert_eq!(value_in(&line).as_deref(), Some(value));
    }

    /// A select's values are what gets sent back, so an option with punctuation
    /// has to come out of the snapshot intact or the select cannot be driven.
    #[test]
    fn a_select_option_with_a_quote_survives_the_snapshot() {
        let option = r#"13" display"#;
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let select = refs.mint(MintRef {
            backend_node_id: 1,
            role: "combobox",
            name: "Size",
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let child = refs.mint(MintRef {
            backend_node_id: 2,
            role: "option",
            name: option,
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let text = format!(
            "- main\n  - combobox \"Size\" [ref={select}]\n    - option {} [ref={child}]",
            serde_json::to_string(option).expect("quotes")
        );
        let view = PageView::from_snapshot("https://e.com", "T", &text, &refs);
        let combobox = view
            .controls
            .iter()
            .find(|control| control.name == "Size")
            .expect("the select");
        assert_eq!(
            combobox.options,
            vec![option.to_string()],
            "the value is what gets sent back, so it has to be exact"
        );
    }

    /// A name containing brackets must not be read as a state.
    #[test]
    fn brackets_inside_a_name_are_not_states() {
        let line = r#"  - button "Filter [beta]" [disabled] [ref=e7]"#;
        assert_eq!(name_in(line).as_deref(), Some("Filter [beta]"));
        assert_eq!(states_in(line), vec!["disabled".to_string()]);
        assert_eq!(reference_in(line), Some("e7"));
    }

    /// A reference, a cursor hint and a state are all bracketed, and only one of
    /// them is a state.
    #[test]
    fn a_reference_and_a_cursor_hint_are_not_states() {
        let line = "  - link \"Deals\" [ref=e7] [cursor=pointer]";
        assert_eq!(reference_in(line), Some("e7"));
        assert!(states_in(line).is_empty());
        assert_eq!(name_in(line).as_deref(), Some("Deals"));
        assert_eq!(role_in(line).as_deref(), Some("link"));
        assert_eq!(value_in(line), None);
    }

    /// A value containing a colon must not be cut at the colon.
    #[test]
    fn a_value_with_a_colon_survives() {
        let line = "  - textbox \"Note\" [ref=e8]: \"ratio 16:9\"";
        assert_eq!(value_in(line).as_deref(), Some("ratio 16:9"));
    }

    /// Options nested under one select do not leak into the next.
    #[test]
    fn nested_options_stop_at_the_next_sibling() {
        let lines = vec![
            "  - combobox \"First\" [ref=e1]",
            "    - option \"A\" [ref=e2]",
            "  - combobox \"Second\" [ref=e3]",
            "    - option \"B\" [ref=e4]",
        ];
        assert_eq!(nested_options(&lines, 0), vec!["A"]);
        assert_eq!(nested_options(&lines, 2), vec!["B"]);
    }

    /// A mixed checkbox means some of a group is selected, not none. Collapsing
    /// it to unchecked told the model nothing was selected, so a goal of
    /// clearing the selection and a goal of selecting everything read the same.
    #[test]
    fn a_mixed_checkbox_is_neither_checked_nor_unchecked() {
        let nodes = vec![with_property(
            ax("1", 11, "checkbox", "All brands"),
            "checked",
            serde_json::Value::String("mixed".to_string()),
        )];
        let view = view_of(&nodes);
        let control = &view.controls[0];
        assert!(control.state.indeterminate);
        assert_eq!(
            control.state.checked, None,
            "it is not a boolean, so it does not claim to be one"
        );
        assert!(
            control
                .state
                .describe()
                .is_some_and(|state| state.contains("partially")),
            "and the model is told so in words: {:?}",
            control.state.describe()
        );
    }

    /// Ticking a partially selected parent is a real change, so its identity
    /// has to move or the action would look like it did nothing.
    #[test]
    fn a_mixed_checkbox_has_its_own_identity() {
        let mixed = view_of(&[with_property(
            ax("1", 11, "checkbox", "All"),
            "checked",
            serde_json::Value::String("mixed".to_string()),
        )]);
        let ticked = view_of(&[with_property(
            ax("1", 11, "checkbox", "All"),
            "checked",
            true.into(),
        )]);
        let unticked = view_of(&[with_property(
            ax("1", 11, "checkbox", "All"),
            "checked",
            false.into(),
        )]);
        assert_ne!(mixed.controls[0].guard, ticked.controls[0].guard);
        assert_ne!(mixed.controls[0].guard, unticked.controls[0].guard);
    }

    /// The renderer's own word for it is read back the same way.
    #[test]
    fn the_renderers_indeterminate_word_is_read_back() {
        let mut refs = RefMap::new();
        refs.begin_snapshot();
        let minted = refs.mint(MintRef {
            backend_node_id: 11,
            role: "checkbox",
            name: "All",
            document_id: None,
            frame_id: Some(&FrameId::from("f".to_string())),
        });
        let text = format!("- main\n  - checkbox \"All\" [indeterminate] [ref={minted}]");
        let view = PageView::from_snapshot("https://e.com", "T", &text, &refs);
        assert!(view.controls[0].state.indeterminate);
        assert_eq!(
            view.controls[0].state.checked, None,
            "a mixed checkbox is not defaulted to unchecked"
        );
    }

    #[test]
    fn a_states_description_reads_as_words() {
        let checked = ControlState {
            checked: Some(true),
            ..ControlState::default()
        };
        assert_eq!(checked.describe().as_deref(), Some("checked"));
        let collapsed = ControlState {
            expanded: Some(false),
            ..ControlState::default()
        };
        assert_eq!(collapsed.describe().as_deref(), Some("collapsed"));
        assert!(ControlState::default().describe().is_none());
    }

    /// Google News nests a tab inside an identical tab and both reach the
    /// snapshot. Offering the pair split the probability between two options
    /// that do the same thing.
    #[test]
    fn a_control_nested_inside_its_own_twin_is_dropped() {
        let twin = |name: &str| Control {
            reference: name.to_string(),
            clickable: false,
            role: "tab".to_string(),
            name: "World".to_string(),
            value: None,
            state: ControlState::default(),
            options: Vec::new(),
            guard: 0,
            group: None,
        };
        let kept = without_nested_twins(vec![(1, twin("e1")), (2, twin("e2"))]);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].reference, "e1",
            "the outer one is the one that stays"
        );
    }

    /// The test is nesting, not the name. Twenty cards each with their own
    /// button are twenty different targets.
    #[test]
    fn twenty_sibling_add_to_cart_buttons_all_survive() {
        let controls: Vec<(usize, Control)> = (0..20)
            .map(|index| {
                (
                    3,
                    Control {
                        reference: format!("e{index}"),
                        clickable: false,
                        role: "button".to_string(),
                        name: "Add to cart".to_string(),
                        value: None,
                        state: ControlState::default(),
                        options: Vec::new(),
                        guard: 0,
                        group: None,
                    },
                )
            })
            .collect();
        assert_eq!(without_nested_twins(controls).len(), 20);
    }

    /// A parent two levels up is not the element this one is, so a button
    /// inside a same-named section keeps its place.
    #[test]
    fn a_twin_further_up_the_tree_is_not_the_same_element() {
        let at = |depth: usize, reference: &str, role: &str| {
            (
                depth,
                Control {
                    reference: reference.to_string(),
                    clickable: false,
                    role: role.to_string(),
                    name: "Filters".to_string(),
                    value: None,
                    state: ControlState::default(),
                    options: Vec::new(),
                    guard: 0,
                    group: None,
                },
            )
        };
        let kept = without_nested_twins(vec![
            at(1, "e1", "button"),
            at(2, "e2", "generic"),
            at(3, "e3", "button"),
        ]);
        // The two buttons are different elements and both survive, which is
        // what this test is about. The generic between them is a wrapper
        // around the same-named button below it, so it folds into it.
        let references: Vec<&str> = kept.iter().map(|c| c.reference.as_str()).collect();
        assert_eq!(references, vec!["e1", "e3"]);
    }

    /// The autocomplete shape: a suggestion offered as a row, a cell and a
    /// button is one suggestion.
    #[test]
    fn a_wrapper_around_a_control_of_the_same_name_folds_into_it() {
        let at = |depth: usize, reference: &str, role: &str| {
            (
                depth,
                Control {
                    reference: reference.to_string(),
                    clickable: false,
                    role: role.to_string(),
                    name: "sensodyne toothpaste 150g".to_string(),
                    value: None,
                    state: ControlState::default(),
                    options: Vec::new(),
                    guard: 0,
                    group: None,
                },
            )
        };
        let kept = without_nested_twins(vec![
            at(1, "e270", "row"),
            at(2, "e271", "gridcell"),
            at(3, "e280", "button"),
        ]);
        let references: Vec<&str> = kept.iter().map(|c| c.reference.as_str()).collect();
        assert_eq!(references, vec!["e280"], "the control, not its wrappers");
    }

    /// An unnamed control cannot be told apart from its parent by name, so the
    /// rule does not apply to it.
    #[test]
    fn an_unnamed_control_is_never_collapsed() {
        let bare = |depth: usize, reference: &str| {
            (
                depth,
                Control {
                    reference: reference.to_string(),
                    clickable: false,
                    role: "button".to_string(),
                    name: String::new(),
                    value: None,
                    state: ControlState::default(),
                    options: Vec::new(),
                    guard: 0,
                    group: None,
                },
            )
        };
        assert_eq!(
            without_nested_twins(vec![bare(1, "e1"), bare(2, "e2")]).len(),
            2
        );
    }

    /// The cart case. The loop cannot know an action succeeded; it can say what
    /// it clicked and that a counter moved, which is checkable.
    #[test]
    fn a_counter_that_moved_is_what_an_action_observably_did() {
        let before = PageView::from_snapshot_parts(
            "https://shop.test/s?k=toothpaste",
            "results",
            "",
            vec![
                named("e63", "link", "0 items in cart"),
                named("e128", "button", "Add to cart"),
            ],
        );
        let after = PageView::from_snapshot_parts(
            "https://shop.test/s?k=toothpaste",
            "results",
            "",
            vec![
                named("e63", "link", "2 items in cart"),
                named("e128", "button", "Add to cart"),
            ],
        );

        let moved = before.what_moved(&after, "add toothpaste to the cart", 3);

        assert_eq!(moved.len(), 1, "got {moved:?}");
        assert_eq!(moved[0].reference, "e63");
        assert_eq!(moved[0].was, "0 items in cart");
        assert_eq!(moved[0].now, "2 items in cart");
    }

    /// A live page has a clock on it. A step that moved several things reports
    /// the ones the goal is about first, and no more than it was asked for.
    #[test]
    fn the_goals_own_words_order_what_is_reported() {
        let before = PageView::from_snapshot_parts(
            "https://shop.test/",
            "shop",
            "",
            vec![
                named("e1", "link", "Ends in 05:52:02"),
                named("e2", "link", "Ends in 04:11:00"),
                named("e3", "link", "0 items in cart"),
            ],
        );
        let after = PageView::from_snapshot_parts(
            "https://shop.test/",
            "shop",
            "",
            vec![
                named("e1", "link", "Ends in 05:52:01"),
                named("e2", "link", "Ends in 04:10:59"),
                named("e3", "link", "1 item in cart"),
            ],
        );

        let moved = before.what_moved(&after, "add it to the cart", 1);

        assert_eq!(moved.len(), 1, "capped at what was asked for");
        assert_eq!(
            moved[0].reference, "e3",
            "the cart before the clocks, got {moved:?}"
        );
    }

    /// A control that is simply gone, or that changed role, is not a counter
    /// moving and is not reported as one.
    #[test]
    fn only_a_control_that_stayed_counts_as_having_moved() {
        let before = PageView::from_snapshot_parts(
            "https://shop.test/",
            "shop",
            "",
            vec![named("e1", "link", "One"), named("e2", "link", "Two")],
        );
        let after = PageView::from_snapshot_parts(
            "https://shop.test/",
            "shop",
            "",
            vec![named("e2", "button", "Two and a half")],
        );

        assert!(before.what_moved(&after, "do something", 3).is_empty());
    }

    /// The docs-site case. A run ended with its search dialog open over the
    /// page it had reached, and a caller told to read that page would have read
    /// the dialog.
    #[test]
    fn a_dialog_over_the_page_is_reported() {
        let mut dialog = named("e9", "dialog", "Search the API docs");
        dialog.clickable = false;
        let view = PageView::from_snapshot_parts(
            "https://docs.test/guides/webhooks",
            "Webhooks",
            "",
            vec![named("e1", "link", "Home"), dialog],
        );

        assert_eq!(
            view.obstructed().as_deref(),
            Some("a dialog \"Search the API docs\" is open over the page")
        );
    }

    /// An ordinary page says nothing, because saying something would make
    /// every reply carry a caveat it does not need.
    #[test]
    fn an_unobstructed_page_says_nothing() {
        let view = PageView::from_snapshot_parts(
            "https://docs.test/",
            "Docs",
            "",
            vec![named("e1", "link", "Home"), named("e2", "link", "Guides")],
        );

        assert!(view.obstructed().is_none());
    }

    /// An expanded dropdown is over the page too, which is what a half-used
    /// select looks like when a run stops on one.
    #[test]
    fn an_expanded_dropdown_counts_as_covering_the_page() {
        let mut open = named("e5", "combobox", "Sort by");
        open.state.expanded = Some(true);
        let view = PageView::from_snapshot_parts("https://shop.test/", "Shop", "", vec![open]);

        assert!(
            view.obstructed()
                .as_deref()
                .is_some_and(|what| what.contains("Sort by")),
            "got {:?}",
            view.obstructed()
        );
    }
}

#[cfg(test)]
mod cards {
    use super::*;

    fn lines_of(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    const LISTING: &str = r#"- list
  - listitem [ref=e1]
    - heading "Sensodyne Repair and Protect" [ref=e2]
    - link "4.4 out of 5 stars, 16,700 reviews" [ref=e3]
    - button "Add to cart" [ref=e4]
  - listitem [ref=e5]
    - heading "Sensodyne Fresh Mint" [ref=e6]
    - button "Add to cart" [ref=e7]
"#;

    #[test]
    fn a_button_is_labelled_by_the_card_it_sits_in() {
        let lines = lines_of(LISTING);
        let first = lines.iter().position(|l| l.contains("ref=e4")).unwrap();
        let second = lines.iter().position(|l| l.contains("ref=e7")).unwrap();
        assert_eq!(
            card_label(&lines, first).as_deref(),
            Some("Sensodyne Repair and Protect")
        );
        assert_eq!(
            card_label(&lines, second).as_deref(),
            Some("Sensodyne Fresh Mint")
        );
    }

    /// The rating sits on every card and so tells none of them apart.
    #[test]
    fn a_rating_is_not_a_label() {
        let lines = lines_of(LISTING);
        let at = lines.iter().position(|l| l.contains("ref=e4")).unwrap();
        let label = card_label(&lines, at).unwrap();
        assert!(!label.contains("out of 5"), "{label}");
    }

    /// A label taken from inside the control is content, not a container. A
    /// heading nested in the control would otherwise outscore the card's own.
    #[test]
    fn a_label_never_comes_from_inside_the_control() {
        let inside = r#"- list
  - listitem [ref=e1]
    - heading "The card's own title" [ref=e2]
    - link "Buy now" [ref=e3]
      - heading "Inside the control" [ref=e4]
"#;
        let lines = lines_of(inside);
        let at = lines.iter().position(|l| l.contains("ref=e3")).unwrap();
        assert_eq!(
            card_label(&lines, at).as_deref(),
            Some("The card's own title")
        );
    }

    /// The regression this was written for: on a contents page whose chapters
    /// nest, the sections are a card of their own and their titles belong to
    /// it, not to the chapter above them.
    #[test]
    fn a_nested_card_keeps_its_own_label() {
        let nested = r#"- list
  - listitem [ref=e1]
    - link "Understanding Ownership" [ref=e2]
    - list
      - listitem [ref=e3]
        - link "References and Borrowing" [ref=e4]
"#;
        let lines = lines_of(nested);
        let at = lines.iter().position(|l| l.contains("ref=e2")).unwrap();
        assert_ne!(
            card_label(&lines, at).as_deref(),
            Some("References and Borrowing")
        );
    }

    #[test]
    fn a_control_outside_any_card_has_no_label() {
        let lines = lines_of("- button \"Go\" [ref=e1]\n");
        assert_eq!(card_label(&lines, 0), None);
    }

    #[test]
    fn a_long_title_is_cut_on_a_word_boundary() {
        let long = "Sensodyne Toothpaste Repair and Protect Tooth Paste for Daily Sensitivity Relief and Cavity Protection";
        let cut = trimmed_to_budget(long);
        assert!(cut.chars().count() <= LABEL_BUDGET, "{cut}");
        assert!(!cut.ends_with(' '));
        assert!(long.starts_with(&cut), "{cut}");
    }
}
