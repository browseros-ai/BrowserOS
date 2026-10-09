//! A typed view of one page, built from the accessibility snapshot the browser
//! already produces.
//!
//! Nothing here parses rendered text. The snapshot layer already computes a
//! typed node with its role, name, value and state properties, and already
//! assigns the `eN` reference the act tool accepts, so this is a translation
//! between two typed representations rather than a second extraction.
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
                "checked" => state.checked = value.and_then(serde_json::Value::as_bool),
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

        let url = url.into();
        let fingerprint = fingerprint_of(&url, &controls);
        Self {
            url,
            title: title.into(),
            text: text.into(),
            controls,
            fingerprint,
        }
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

    /// Trims the text to a byte budget on a line boundary.
    ///
    /// Irrelevant detail in the state measurably degrades the answer, so this is
    /// an accuracy measure as much as a cost one.
    pub fn budget_text(&mut self, budget: usize) {
        if self.text.len() <= budget {
            return;
        }
        // Floor to a character boundary before slicing: a budget that lands
        // mid-character would otherwise panic on the slice itself.
        let ceiling = floor_char_boundary(&self.text, budget);
        let cut = self.text[..ceiling].rfind('\n').unwrap_or(ceiling);
        self.text.truncate(cut);
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
    control.state.expanded.hash(&mut hasher);
    control.state.selected.hash(&mut hasher);
    control.state.disabled.hash(&mut hasher);
    hasher.finish()
}

fn fingerprint_of(url: &str, controls: &[Control]) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    for control in controls {
        control.guard.hash(&mut hasher);
    }
    hasher.finish()
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
