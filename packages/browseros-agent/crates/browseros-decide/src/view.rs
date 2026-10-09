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
}

impl Control {
    /// Whether this control can be offered at all.
    ///
    /// Disabled controls are dropped here rather than offered and refused later,
    /// because an option the model cannot usefully pick still costs tokens and
    /// still competes for the answer.
    #[must_use]
    pub fn is_offerable(&self) -> bool {
        !self.state.disabled && (!self.name.is_empty() || !self.options.is_empty())
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
                role: role.clone(),
                name,
                value: text_of(node.value.as_ref()).filter(|value| !value.is_empty()),
                options: options_of(node, &by_node_id),
                state,
                guard: 0,
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
        };
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
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
            if !ACTIONABLE_ROLES.contains(&role.as_str()) {
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
            controls.push(Control {
                reference,
                options: nested_options(&lines, index),
                role,
                name: name_in(line).unwrap_or_else(|| entry.name.clone()),
                value: value_in(line),
                state,
                guard: 0,
            });
        }
        for control in &mut controls {
            control.guard = guard_of(control);
        }

        let mut view = Self {
            url: url.into(),
            title: title.into(),
            text: text.to_string(),
            controls,
            fingerprint: 0,
        };
        // Budget before fingerprinting, so the hash covers the text that
        // travels to the model and not the whole document.
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
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
        };
        view.budget_text(Self::TEXT_BUDGET);
        view.fingerprint = fingerprint_of(&view.url, &view.title, &view.text, &view.controls);
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

fn reference_in(line: &str) -> Option<&str> {
    let start = line.find("[ref=")? + "[ref=".len();
    let rest = &line[start..];
    let end = rest.find(']')?;
    Some(&rest[..end])
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
    use super::*;
    use browseros_core::snapshot::{AxProperty, AxValue};
    use browseros_core::{FrameId, snapshot::refs::MintRef};

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
}
