use crate::proto::{
    BrowserOsContentAttributes, BrowserOsTriState, ContentAttributes, ContentData, FormControlType,
};

use super::hidden;
use super::role::{self, Role};
use super::tree_doc::TreeDoc;
use super::tree_node::TreeNode;

/// Tri-state ARIA value for `checked` and `pressed` controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriState {
    /// Explicitly false or unset on a role that supports tri-state.
    False,
    /// Explicitly true.
    True,
    /// Mixed or indeterminate (`aria-checked="mixed"`, `aria-pressed="mixed"`, or
    /// `input.indeterminate`).
    Mixed,
}

/// Effective `aria-invalid` classification matching Playwright's `getAriaInvalid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidState {
    /// Valid (`aria-invalid` absent, blank after JavaScript whitespace trim, or ASCII `"false"`).
    False,
    /// Invalid (`aria-invalid="true"` or any unrecognized non-empty value).
    True,
    /// Grammatical error (`aria-invalid="grammar"`).
    Grammar,
    /// Spelling error (`aria-invalid="spelling"`).
    Spelling,
}

impl InvalidState {
    /// Parses a raw `aria-invalid` attribute string with Playwright's exact semantics.
    #[must_use]
    pub fn from_aria_attr(raw: Option<&str>) -> Self {
        let Some(value) = raw else {
            return Self::False;
        };
        if role::trim_js_whitespace(value).is_empty() || value.eq_ignore_ascii_case("false") {
            return Self::False;
        }
        match value {
            "grammar" => Self::Grammar,
            "spelling" => Self::Spelling,
            _ => Self::True,
        }
    }
}

/// Precomputed WAI-ARIA states, focus flag, and inert flag for one [`TreeNode`].
///
/// State fields gated by role capabilities (`checked`, `pressed`, `expanded`, `selected`, `level`,
/// `disabled`, `invalid`) yield `None` when the node's [`Role`] does not support them, matching
/// Playwright's `toAriaNode` and `aria-template` rules, while [`Self::is_disabled`] also matches
/// `getAriaDisabled` and `getByRole(..., { disabled })` on roles outside `kAriaDisabledRoles`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    checked: Option<TriState>,
    pressed: Option<TriState>,
    expanded: Option<bool>,
    selected: Option<bool>,
    level: Option<u32>,
    disabled: Option<bool>,
    is_disabled: bool,
    invalid: Option<InvalidState>,
    active: bool,
    inert: bool,
}

impl State {
    /// Checked tri-state when this node's role supports `checked`, else `None`.
    #[must_use]
    pub const fn checked(self) -> Option<TriState> {
        self.checked
    }

    /// Pressed tri-state when this node's role supports `pressed`, else `None`.
    #[must_use]
    pub const fn pressed(self) -> Option<TriState> {
        self.pressed
    }

    /// Expanded boolean when this node's role supports `expanded` and the attribute is set, else
    /// `None` (distinguishing unset from `false`).
    #[must_use]
    pub const fn expanded(self) -> Option<bool> {
        self.expanded
    }

    /// Selected boolean when this node's role supports `selected`, else `None`.
    #[must_use]
    pub const fn selected(self) -> Option<bool> {
        self.selected
    }

    /// Heading or hierarchy level when this node's role supports `level` (`1..=6` for `h1..=h6`,
    /// else `aria-level >= 1` or `0`), else `None`.
    #[must_use]
    pub const fn level(self) -> Option<u32> {
        self.level
    }

    /// Disabled boolean when this node's role supports `disabled`, else `None`.
    #[must_use]
    pub const fn disabled(self) -> Option<bool> {
        self.disabled
    }

    /// Whether this node is disabled under Playwright's `getAriaDisabled` rules (native form
    /// control disabled state, or inherited `aria-disabled` on roles supporting `disabled`).
    #[must_use]
    pub const fn is_disabled(self) -> bool {
        self.is_disabled
    }

    /// Effective `aria-invalid` state when this node's role supports `invalid`, else `None`.
    #[must_use]
    pub const fn invalid(self) -> Option<InvalidState> {
        self.invalid
    }

    /// Whether this element holds keyboard focus in its owning document.
    #[must_use]
    pub const fn active(self) -> bool {
        self.active
    }

    /// Whether this element's own computed style is inert (never inherited across descendants).
    #[must_use]
    pub const fn inert(self) -> bool {
        self.inert
    }
}

fn decode_tri_state(raw: Option<i32>) -> TriState {
    match raw.and_then(|code| BrowserOsTriState::try_from(code).ok()) {
        Some(BrowserOsTriState::True) => TriState::True,
        Some(BrowserOsTriState::Mixed) => TriState::Mixed,
        Some(BrowserOsTriState::False | BrowserOsTriState::Unset) | None => TriState::False,
    }
}

fn compute_checked(
    role: Option<Role>,
    raw: &ContentAttributes,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<TriState> {
    if !role.is_some_and(Role::supports_checked) {
        return None;
    }
    if let Some(code) = browseros.and_then(|b| b.checked) {
        return Some(decode_tri_state(Some(code)));
    }
    if hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case("input"))
        && let Some(ContentData::FormControl(control)) = raw.content_data.as_ref()
        && let Some(control_type) = control
            .form_control_type
            .and_then(|code| FormControlType::try_from(code).ok())
        && matches!(
            control_type,
            FormControlType::InputCheckbox | FormControlType::InputRadio
        )
    {
        return Some(if control.is_checked == Some(true) {
            TriState::True
        } else {
            TriState::False
        });
    }
    Some(TriState::False)
}

fn compute_pressed(
    role: Option<Role>,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<TriState> {
    if !role.is_some_and(Role::supports_pressed) {
        return None;
    }
    Some(decode_tri_state(browseros.and_then(|b| b.pressed)))
}

fn compute_expanded(
    role: Option<Role>,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<bool> {
    if !role.is_some_and(Role::supports_expanded) {
        return None;
    }
    match browseros
        .and_then(|b| b.expanded)
        .and_then(|code| BrowserOsTriState::try_from(code).ok())
    {
        Some(BrowserOsTriState::True) => Some(true),
        Some(BrowserOsTriState::False | BrowserOsTriState::Mixed) => Some(false),
        Some(BrowserOsTriState::Unset) | None => None,
    }
}

fn compute_selected(
    role: Option<Role>,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<bool> {
    if !role.is_some_and(Role::supports_selected) {
        return None;
    }
    Some(browseros.and_then(|b| b.selected).unwrap_or(false))
}

fn native_heading_level(tag: Option<&str>) -> Option<u32> {
    let bytes = tag?.as_bytes();
    if bytes.len() == 2
        && matches!(bytes.first(), Some(b'h' | b'H'))
        && let Some(&digit @ b'1'..=b'6') = bytes.get(1)
    {
        return Some(u32::from(digit - b'0'));
    }
    None
}

fn compute_level(
    role: Option<Role>,
    raw: &ContentAttributes,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<u32> {
    if !role.is_some_and(Role::supports_level) {
        return None;
    }
    if let Some(native) = native_heading_level(hidden::raw_tag(raw)) {
        return Some(native);
    }
    let aria_level = browseros
        .and_then(|b| b.level)
        .filter(|&level| level >= 1)
        .unwrap_or(0);
    Some(aria_level)
}

fn compute_invalid(
    role: Option<Role>,
    browseros: Option<&BrowserOsContentAttributes>,
) -> Option<InvalidState> {
    if !role.is_some_and(Role::supports_invalid) {
        return None;
    }
    Some(InvalidState::from_aria_attr(
        browseros.and_then(|b| b.aria_invalid.as_deref()),
    ))
}

pub(crate) fn compute_states(
    nodes: &[TreeNode],
    docs: &[TreeDoc],
    roles: &[Option<Role>],
) -> Vec<State> {
    let mut states = Vec::with_capacity(nodes.len());
    let mut effective_aria_disabled = Vec::with_capacity(nodes.len());

    for (index, node) in nodes.iter().enumerate() {
        let parent_aria_disabled = node
            .parent()
            .filter(|&parent_id| {
                nodes
                    .get(parent_id.index())
                    .is_some_and(|parent| parent.doc() == node.doc())
            })
            .and_then(|parent_id| effective_aria_disabled.get(parent_id.index()).copied())
            .unwrap_or(false);

        let raw = node.raw_attributes();
        let browseros = raw.and_then(|attrs| attrs.browseros.as_deref());
        let node_aria_disabled = match browseros.and_then(|b| b.aria_disabled) {
            Some(explicit) => explicit,
            None => parent_aria_disabled,
        };
        effective_aria_disabled.push(node_aria_disabled);

        let Some(raw_attrs) = raw else {
            states.push(State::default());
            continue;
        };
        if hidden::is_text_node(raw_attrs) || hidden::is_ua_shadow(raw_attrs) {
            states.push(State::default());
            continue;
        }

        let role = roles.get(index).copied().flatten();
        let supports_disabled = role.is_some_and(Role::supports_disabled);
        let is_disabled =
            role::is_natively_disabled(raw_attrs) || (supports_disabled && node_aria_disabled);
        let disabled = supports_disabled.then_some(is_disabled);

        let doc_focused = docs
            .get(node.doc().index())
            .and_then(TreeDoc::focused_node_id);
        let active = node.dom_node_id().is_some() && node.dom_node_id() == doc_focused;
        let inert = browseros.is_some_and(|b| b.inert == Some(true));

        states.push(State {
            checked: compute_checked(role, raw_attrs, browseros),
            pressed: compute_pressed(role, browseros),
            expanded: compute_expanded(role, browseros),
            selected: compute_selected(role, browseros),
            level: compute_level(role, raw_attrs, browseros),
            disabled,
            is_disabled,
            invalid: compute_invalid(role, browseros),
            active,
            inert,
        });
    }

    states
}

#[cfg(test)]
mod tests {
    use super::InvalidState;

    #[test]
    fn invalid_state_from_aria_attr_matches_playwright() {
        let cases: &[(Option<&str>, InvalidState, &str)] = &[
            (None, InvalidState::False, "absent attribute"),
            (Some(""), InvalidState::False, "empty string"),
            (
                Some(" \t\u{FEFF}\u{00A0} "),
                InvalidState::False,
                "JS whitespace only",
            ),
            (Some("false"), InvalidState::False, "lowercase false"),
            (Some("FALSE"), InvalidState::False, "uppercase FALSE"),
            (Some("true"), InvalidState::True, "lowercase true"),
            (Some("grammar"), InvalidState::Grammar, "lowercase grammar"),
            (
                Some("spelling"),
                InvalidState::Spelling,
                "lowercase spelling",
            ),
            (
                Some(" false "),
                InvalidState::True,
                "whitespace around false prevents lowercase equality",
            ),
            (
                Some("GRAMMAR"),
                InvalidState::True,
                "case-sensitive GRAMMAR falls back to True",
            ),
            (
                Some("\u{0085}"),
                InvalidState::True,
                "U+0085 NEL is not JS whitespace and falls back to True",
            ),
        ];

        for &(raw, expected, reason) in cases {
            assert_eq!(
                InvalidState::from_aria_attr(raw),
                expected,
                "aria-invalid={raw:?} ({reason}) must map to {expected:?}"
            );
        }
    }
}
