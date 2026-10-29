pub const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "textarea",
    "checkbox",
    "radio",
    "combobox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "switch",
    "slider",
    "spinbutton",
    "option",
    "treeitem",
    "listbox",
    "DisclosureTriangle",
    // Chromium's roles for <input type=date|month|week>, datetime-local, time and color. They
    // take a value rather than typed text, and without a ref fill could not reach them.
    "Date",
    "DateTime",
    "InputTime",
    "ColorWell",
];

pub const SKIP_ROLES: &[&str] = &[
    "none",
    "presentation",
    "LineBreak",
    "InlineTextBox",
    "StaticText",
    "text",
];

pub const ROOT_ROLES: &[&str] = &["RootWebArea", "WebArea"];

pub const VALUE_ROLES: &[&str] = &[
    "textbox",
    "searchbox",
    "textarea",
    "combobox",
    "spinbutton",
    "Date",
    "DateTime",
    "InputTime",
    "ColorWell",
];

#[must_use]
pub fn is_interactive_role(role: &str) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

#[must_use]
pub fn is_skip_role(role: &str) -> bool {
    SKIP_ROLES.contains(&role)
}

#[must_use]
pub fn is_root_role(role: &str) -> bool {
    ROOT_ROLES.contains(&role)
}

#[must_use]
pub fn is_value_role(role: &str) -> bool {
    VALUE_ROLES.contains(&role)
}
