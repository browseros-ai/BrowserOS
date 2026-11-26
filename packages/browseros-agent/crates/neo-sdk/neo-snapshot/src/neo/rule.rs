use crate::aria_sink::EmittedNode;
use crate::cap::format_section_label;

/// Roles an agent acts on: the list desktop Neo's clickable observer uses.
const INTERACTIVE: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
];

/// Fields keep their state visible even when the engine gives them no pointer target.
pub(super) const FIELD: &[&str] = &[
    "textbox",
    "searchbox",
    "spinbutton",
    "slider",
    "combobox",
    "listbox",
    "checkbox",
    "radio",
    "switch",
    "menuitemcheckbox",
    "menuitemradio",
];

/// Regions an agent can scope a snapshot to (the cut line names one), so their refs print. A
/// cut `mode: 'full'` text offers them too.
pub(crate) const SECTION: &[&str] = &[
    "banner",
    "navigation",
    "main",
    "complementary",
    "contentinfo",
    "search",
    "form",
    "region",
    "article",
    "iframe",
];

/// The roles the cap keeps whole past its cut, in Neo's default format and in `mode: 'full'`:
/// blocks whose text says what the controls around them mean (an error, a confirm question), so
/// their text prints even in the interactive form, and a cut never hides them. Part of the
/// interface because the run description says so in words, and its test compares its prose with
/// this value.
pub const KEPT_ROLES: &[&str] = &["dialog", "alertdialog", "alert", "status"];

/// The kept blocks whose line always prints its ref: an agent scopes to a dialog.
const KEPT_WITH_REF: &[&str] = &["dialog", "alertdialog"];

/// Which of the format's rules prints a child: the first that matches, in the TypeScript's order
/// (`Printer.node` in `snapshot-v1.ts`). An inert subtree the call did not ask for is decided
/// before any rule, by the printer that meets it.
#[derive(Clone, Copy, Debug)]
pub(super) enum Rule<'a, Id> {
    /// A string child, or a `text` node's text: joins the text line at its level.
    Piece(&'a str),
    /// A dialog, an alert or a status: its line, then all of its text.
    Kept(Id, EmittedNode<'a>),
    /// A region an agent can scope to: its line with its ref, its content indented.
    Section(Id, EmittedNode<'a>),
    /// Its line, with its own text when it has no name.
    Heading(Id, EmittedNode<'a>),
    /// Something to act on, or a field: one line with its ref and its value.
    Control(Id, EmittedNode<'a>),
    /// No line, unless it has a name and the page's text is printed; its content moves up.
    Image(Id, EmittedNode<'a>),
    /// No line; its name prints as text when nothing under it does, and its content moves up.
    Content(Id, EmittedNode<'a>),
}

impl<'a, Id: Copy> Rule<'a, Id> {
    pub(super) fn of_node(id: Id, node: EmittedNode<'a>) -> Self {
        let role = node.role;
        if role == "text" {
            Self::Piece(node.text.unwrap_or_default())
        } else if KEPT_ROLES.contains(&role) {
            Self::Kept(id, node)
        } else if SECTION.contains(&role) {
            Self::Section(id, node)
        } else if role == "heading" {
            Self::Heading(id, node)
        } else if actionable(&node) || field_without_ref(&node) {
            Self::Control(id, node)
        } else if role == "img" {
            Self::Image(id, node)
        } else {
            Self::Content(id, node)
        }
    }
}

/// `actionable`: a ref, and an interactive role or a pointer cursor. An empty ref counts, as the
/// TypeScript asks `ref !== undefined`.
pub(super) fn actionable(node: &EmittedNode<'_>) -> bool {
    node.r#ref.is_some() && (interactive(node.role) || node.cursor == Some("pointer"))
}

/// `fieldWithoutRef`: a field the engine gave no ref.
pub(super) fn field_without_ref(node: &EmittedNode<'_>) -> bool {
    node.r#ref.is_none() && FIELD.contains(&node.role)
}

/// `keptRef`: a kept block's line prints its ref always for a dialog, and for an alert or a
/// status only when it is actionable (a toast the user can click).
pub(super) fn kept_ref(node: &EmittedNode<'_>) -> bool {
    KEPT_WITH_REF.contains(&node.role) || actionable(node)
}

pub(super) fn interactive(role: &str) -> bool {
    INTERACTIVE.contains(&role)
}

/// `shownText`: a node's own `text` as Neo's formats print it, the default format and
/// `mode: 'full'`. An iframe's text is the fallback markup written between its tags, which no
/// browser shows; printing it would hide the frame's content, which is stitched under the same
/// node as children, since upstream's printer prints a node with text as one line and never
/// visits its children.
pub(crate) fn shown_text<'a>(node: &EmittedNode<'a>) -> Option<&'a str> {
    if node.role == "iframe" {
        None
    } else {
        node.text
    }
}

/// `sectionLabel`: how the cut line names a section, its role and its quoted name when it has
/// one.
pub(crate) fn section_label(node: &EmittedNode<'_>) -> String {
    format_section_label(node.role, node.name)
}

/// The string when JavaScript holds it true: there and not empty.
pub(super) fn named(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.is_empty())
}
