use serde_json::Number;

use crate::aria_node::{AriaNode, Invalid, NodeBox, Tristate};
use crate::aria_sink::{EmittedNode, NodeRef};
use crate::js;

use super::yaml_escape::YamlEscape;

/// The longest name upstream prints, in UTF-16 units. A longer one is left out of the key without
/// a trace: YAML keys are limited to 1,024 characters, and the role and the marks need the rest.
const MAX_NAME_UNITS: usize = 900;

/// One node as upstream's printer is handed it: the fields its `createKey` and `visit` read,
/// borrowed from the tree, and no children.
///
/// A format builds one for each node it prints, as the TypeScript renderer builds a copy of the
/// node without its children for upstream's function, and prints what its own rules keep of the
/// node (a text left out or replaced, a ref dropped) by setting a field. With no children here,
/// the printer cannot recurse: the walk, and so the depth a tree may have, is always the caller's.
///
/// The printer asks whether a mark is true, never whether it is there, as upstream's truthiness
/// tests do: an explicit `false`, which the tree's type admits and no page writes, prints as an
/// absent mark does.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeAlone<'a> {
    pub(crate) role: &'a str,
    pub(crate) name: Option<&'a str>,
    pub(crate) checked: Option<Tristate>,
    pub(crate) disabled: Option<bool>,
    pub(crate) expanded: Option<bool>,
    pub(crate) active: Option<bool>,
    pub(crate) invalid: Option<Invalid>,
    pub(crate) level: Option<&'a Number>,
    pub(crate) pressed: Option<Tristate>,
    pub(crate) selected: Option<bool>,
    pub(crate) aria_hidden: Option<bool>,
    pub(crate) r#ref: Option<NodeRef<'a>>,
    pub(crate) cursor: Option<&'a str>,
    pub(crate) r#box: Option<&'a NodeBox>,
    /// Printed under the key as `- /url:`, which makes the key open a block.
    pub(crate) url: Option<&'a str>,
    /// Printed under the key as `- /placeholder:`, which makes the key open a block.
    pub(crate) placeholder: Option<&'a str>,
    /// On the key's line after `: `, or as a `- text:` line under the url and placeholder.
    pub(crate) text: Option<&'a str>,
}

/// Every field of the node upstream's printer reads, as Playwright's own format prints it.
impl<'a> From<&'a AriaNode> for NodeAlone<'a> {
    fn from(node: &'a AriaNode) -> Self {
        Self {
            role: &node.role,
            name: node.name.as_deref(),
            checked: node.checked,
            disabled: node.disabled,
            expanded: node.expanded,
            active: node.active,
            invalid: node.invalid,
            level: node.level.as_ref(),
            pressed: node.pressed,
            selected: node.selected,
            aria_hidden: node.aria_hidden,
            r#ref: node.r#ref.as_deref().map(NodeRef::borrowed),
            cursor: node.cursor.as_deref(),
            r#box: node.r#box.as_ref(),
            url: node.url.as_deref(),
            placeholder: node.placeholder.as_deref(),
            text: node.text.as_deref(),
        }
    }
}

impl<'a> NodeAlone<'a> {
    /// Borrows upstream's printer fields from an [`EmittedNode`].
    pub(crate) fn from_emitted(node: &EmittedNode<'a>) -> Self {
        Self {
            role: node.role,
            name: node.name,
            checked: node.checked,
            disabled: node.disabled,
            expanded: node.expanded,
            active: node.active,
            invalid: node.invalid,
            level: node.level,
            pressed: node.pressed,
            selected: node.selected,
            aria_hidden: node.aria_hidden,
            r#ref: node.r#ref,
            cursor: node.cursor,
            r#box: node.r#box,
            url: node.url,
            placeholder: node.placeholder,
            text: node.text,
        }
    }
}

/// Appends what upstream's `renderAriaSnapshotAsYaml([node])` prints for one node without
/// children (`visit` at depth 0): a text node's `- text:` line; otherwise `- ` and the key, with
/// the node's text after `: `, or, when the node has a url or a placeholder, a `:` and those lines
/// with its text one level under the key. A role or a ref holding a line break keeps it raw inside
/// the quoted key, so the result can be more lines than these.
#[cfg(test)]
pub(crate) fn node_text(node: &NodeAlone<'_>, out: &mut String) {
    let mut key_buf = String::new();
    node_text_into(node, &mut key_buf, out);
}

/// [`node_text`] reusing `key_buf` across calls so hot line walks avoid allocating a fresh key
/// string per node.
pub(crate) fn node_text_into(node: &NodeAlone<'_>, key_buf: &mut String, out: &mut String) {
    if node.role == "text" {
        // `visitText(node.text || '')`: a text node prints its text, whatever else it holds.
        text_line(node.text.unwrap_or_default(), out);
        return;
    }
    out.push_str("- ");
    key_buf.clear();
    write_key(node, key_buf);
    YamlEscape::Key.write(key_buf, out);
    if node.url.is_none() && node.placeholder.is_none() {
        // A leaf, or a leaf with just its text.
        if let Some(text) = node.text {
            out.push_str(": ");
            YamlEscape::Value.write(text, out);
        }
        return;
    }
    out.push(':');
    for (prop, value) in [("url", node.url), ("placeholder", node.placeholder)] {
        if let Some(value) = value {
            out.push_str("\n  - /");
            out.push_str(prop);
            out.push_str(": ");
            YamlEscape::Value.write(value, out);
        }
    }
    if let Some(text) = node.text {
        out.push_str("\n  ");
        text_line(text, out);
    }
}

/// Appends upstream's `visitText` at depth 0: `- text: ` and the text, quoted when YAML needs it,
/// so an empty text prints `- text: ""`.
pub(crate) fn text_line(text: &str, out: &mut String) {
    out.push_str("- text: ");
    YamlEscape::Value.write(text, out);
}

/// `createKey`: the role, the name, then each mark in upstream's order, written into `key`.
fn write_key(node: &NodeAlone<'_>, key: &mut String) {
    key.push_str(node.role);
    // `node.name && node.name.length <= 900`: an empty name prints nothing.
    if let Some(name) = node
        .name
        .filter(|name| !name.is_empty() && js::units(name) <= MAX_NAME_UNITS)
    {
        key.push(' ');
        js::json_quote(name, key);
    }
    match node.checked {
        Some(Tristate::Mixed) => key.push_str(" [checked=mixed]"),
        Some(Tristate::True) => key.push_str(" [checked]"),
        Some(Tristate::False) | None => {}
    }
    for (set, mark) in [
        (node.disabled, " [disabled]"),
        (node.expanded, " [expanded]"),
        (node.active, " [active]"),
    ] {
        if set == Some(true) {
            key.push_str(mark);
        }
    }
    match node.invalid {
        Some(Invalid::Grammar) => key.push_str(" [invalid=grammar]"),
        Some(Invalid::Spelling) => key.push_str(" [invalid=spelling]"),
        Some(Invalid::True) => key.push_str(" [invalid]"),
        Some(Invalid::False) | None => {}
    }
    if let Some(level) = node.level.filter(|level| truthy(level)) {
        key.push_str(" [level=");
        key.push_str(&js::number_text(level));
        key.push(']');
    }
    match node.pressed {
        Some(Tristate::Mixed) => key.push_str(" [pressed=mixed]"),
        Some(Tristate::True) => key.push_str(" [pressed]"),
        Some(Tristate::False) | None => {}
    }
    for (set, mark) in [
        (node.selected, " [selected]"),
        (node.aria_hidden, " [aria-hidden]"),
    ] {
        if set == Some(true) {
            key.push_str(mark);
        }
    }
    // `if (node.ref)`: an empty ref prints neither itself nor the cursor.
    if let Some(reference) = node.r#ref.filter(|reference| !reference.is_empty()) {
        key.push_str(" [ref=");
        reference.write_to(key);
        key.push(']');
        if node.cursor == Some("pointer") {
            key.push_str(" [cursor=pointer]");
        }
    }
    if let Some(NodeBox {
        x,
        y,
        width,
        height,
    }) = node.r#box
    {
        key.push_str(" [box=");
        key.push_str(&js::number_text(x));
        key.push(',');
        key.push_str(&js::number_text(y));
        key.push(',');
        key.push_str(&js::number_text(width));
        key.push(',');
        key.push_str(&js::number_text(height));
        key.push(']');
    }
}

/// `if (node.level)`: JavaScript's truth of the double `JSON.parse` makes of `number`, false for
/// 0 and -0 alone, since no JSON number is `NaN`. A number only serde's `arbitrary_precision`
/// cannot make a double is past the double's range, an infinity to `JSON.parse`, and true.
fn truthy(number: &Number) -> bool {
    number.as_f64().is_none_or(|double| double != 0.0)
}
