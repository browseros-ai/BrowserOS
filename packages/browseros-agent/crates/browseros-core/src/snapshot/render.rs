use crate::{
    FrameId,
    sensitive_fields::{
        FIELD_MASK, FieldAttributes, FieldRules, FieldValues, mask_value, passes_luhn,
    },
    snapshot::{
        ax_types::{AxNode, AxValue},
        refs::{DocumentId, MintRef, RefMap},
        roles::{is_interactive_role, is_root_role, is_skip_role, is_value_role},
    },
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use zeroize::Zeroizing;

const IFRAME_ROLES: &[&str] = &["Iframe", "iframe"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IframeStitch {
    pub backend_node_id: i64,
    pub line_index: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderResult {
    pub text: String,
    pub iframes: Vec<IframeStitch>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SnapshotMode {
    #[default]
    Full,
    Interactive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotOptions {
    pub mode: SnapshotMode,
    pub depth: Option<usize>,
}

pub struct RenderOptions<'a> {
    pub refs: &'a mut RefMap,
    pub frame_id: Option<FrameId>,
    pub document_id: Option<DocumentId>,
    pub cursor_hits: Option<HashMap<i64, Vec<String>>>,
    pub base_depth: usize,
}

pub fn render_snapshot<'a>(nodes: &'a [AxNode], opts: &mut RenderOptions<'a>) -> RenderResult {
    render(nodes, opts, None).0
}

/// One frame's form controls, read from its DOM snapshot, for [`render_snapshot_masked`].
#[derive(Debug, Default)]
pub struct FrameFields {
    /// Form controls by backend node id.
    pub fields: HashMap<i64, FieldAttributes>,
    /// The frame's DOM snapshot could not be read: fields are classified from their
    /// accessibility node alone ([`FieldRules::classify_ax`]) instead of shown raw.
    pub degraded: bool,
}

/// A frame rendered with its sensitive fields masked, plus those fields' raw values so the
/// caller can mask them wherever else the assembled snapshot repeats them.
#[derive(Debug)]
pub struct MaskedRender {
    /// The rendered lines, as [`render_snapshot`] returns them.
    pub result: RenderResult,
    /// The raw values of the fields that were masked (never Chromium's password bullets).
    pub values: FieldValues,
}

/// [`render_snapshot`] for [`SensitiveValues::Mask`](crate::sensitive_fields::SensitiveValues):
/// each value field the shared rule table calls sensitive shows a fixed mask (a card number
/// keeps its last four), as does any value Chromium already masked with bullets, so a
/// password's length does not leak either. Unclassified fields keep [`render_snapshot`]'s
/// heuristic as a second net.
pub fn render_snapshot_masked<'a>(
    nodes: &'a [AxNode],
    opts: &mut RenderOptions<'a>,
    fields: &FrameFields,
) -> MaskedRender {
    let (result, values) = render(nodes, opts, Some(fields));
    MaskedRender {
        result,
        values: FieldValues::new(values),
    }
}

fn render<'a>(
    nodes: &'a [AxNode],
    opts: &mut RenderOptions<'a>,
    fields: Option<&FrameFields>,
) -> (RenderResult, Vec<Zeroizing<String>>) {
    let by_id = nodes
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut ctx = RenderContext {
        by_id,
        card_forms: card_form_scopes(nodes),
        control_labels: labels_of_rendered_controls(nodes),
        lines: Vec::new(),
        iframes: Vec::new(),
        fields,
        masked_values: Vec::new(),
        opts,
    };

    for root_id in entry_node_ids(nodes) {
        ctx.visit(&root_id, 0, false);
    }

    let result = RenderResult {
        text: ctx.lines.join("\n"),
        iframes: ctx.iframes,
    };
    (result, ctx.masked_values)
}

#[must_use]
pub fn apply_snapshot_options(text: &str, options: SnapshotOptions) -> String {
    let text = match options.mode {
        SnapshotMode::Full => text.to_string(),
        SnapshotMode::Interactive => filter_interactive_lines(text),
    };
    if let Some(max_depth) = options.depth {
        filter_depth_lines(&text, max_depth)
    } else {
        text
    }
}

struct RenderContext<'a, 'b> {
    by_id: HashMap<String, &'a AxNode>,
    /// See [`card_form_scopes`].
    card_forms: HashSet<String>,
    /// See [`labels_of_rendered_controls`].
    control_labels: HashSet<i64>,
    lines: Vec<String>,
    iframes: Vec<IframeStitch>,
    /// Set only by [`render_snapshot_masked`].
    fields: Option<&'b FrameFields>,
    /// Raw values of the fields masked so far, for echo masking.
    masked_values: Vec<Zeroizing<String>>,
    opts: &'b mut RenderOptions<'a>,
}

impl RenderContext<'_, '_> {
    fn visit(&mut self, node_id: &str, depth: usize, in_card_form: bool) {
        let Some(node) = self.by_id.get(node_id).copied() else {
            return;
        };
        let in_card_form = in_card_form || self.card_forms.contains(node_id);

        let role = if node.ignored.unwrap_or(false) {
            None
        } else {
            str_val(node.role.as_ref())
        };
        let name = str_val(node.name.as_ref()).unwrap_or_default();
        let is_cursor_hit = node.backend_dom_node_id.is_some_and(|id| {
            !self.control_labels.contains(&id)
                && self
                    .opts
                    .cursor_hits
                    .as_ref()
                    .is_some_and(|hits| hits.contains_key(&id))
        });
        // Chromium leaves a label's own name empty and puts its words in text children, which
        // an interactive snapshot drops. A label that is its own target needs them to be read.
        // Only the line shows them: the ref keeps the tree's name, which ref recovery matches.
        let label_text = (is_cursor_hit && name.is_empty() && role.as_deref() == Some("LabelText"))
            .then(|| self.text_of(node));

        if is_dropped(role.as_deref(), &name, is_cursor_hit) {
            for child_id in node.child_ids.as_deref().unwrap_or(&[]) {
                self.visit(child_id, depth, in_card_form);
            }
            return;
        }

        let Some(role) = role else {
            return;
        };
        let absolute_depth = self.opts.base_depth + depth;
        if IFRAME_ROLES.contains(&role.as_str()) {
            let mut line = format!("{}- iframe", "  ".repeat(absolute_depth));
            if !name.is_empty() {
                line.push(' ');
                line.push_str(&json_quote(&name));
            }
            self.lines.push(line);
            if let Some(backend_node_id) = node.backend_dom_node_id {
                self.iframes.push(IframeStitch {
                    backend_node_id,
                    line_index: self.lines.len() - 1,
                    depth: absolute_depth,
                });
            }
            return;
        }

        let value = self.shown_value(node, &role, &name, in_card_form);
        let line = format_line(
            node,
            &role,
            LineName {
                tree: &name,
                shown: label_text.as_deref().unwrap_or(&name),
            },
            absolute_depth,
            value.as_deref(),
            is_cursor_hit,
            self.opts,
        );
        self.lines.push(line);
        for child_id in node.child_ids.as_deref().unwrap_or(&[]) {
            self.visit(child_id, depth + 1, in_card_form);
        }
    }

    /// The value a value-role node shows, or `None` when it shows none. Without `fields` this
    /// is claw-server's heuristic ([`visible_value`]); with them, the rule table decides first.
    fn shown_value(
        &mut self,
        node: &AxNode,
        role: &str,
        name: &str,
        in_card_form: bool,
    ) -> Option<String> {
        // A slider's value arrives as a number, so it is displayed rather than read as a string.
        let value = node
            .value
            .as_ref()
            .and_then(|value| value.value.as_ref())
            .map(value_display)
            .filter(|value| is_value_role(role) && !value.is_empty())?;
        let Some(fields) = self.fields else {
            return Some(visible_value(name, &value, in_card_form).into_owned());
        };
        let bullets = value.chars().all(|c| c == '•');
        let rules = FieldRules::get();
        let kind = if fields.degraded {
            rules.classify_ax(name, &value)
        } else {
            node.backend_dom_node_id
                .and_then(|id| fields.fields.get(&id))
                .and_then(|field| rules.classify(field, name))
        };
        match kind {
            Some(kind) => {
                let shown = mask_value(kind, &value).into_owned();
                if !bullets {
                    self.masked_values.push(Zeroizing::new(value));
                }
                Some(shown)
            }
            // Chromium masked it, so it is a password field the table missed: hide its length.
            None if bullets => Some(FIELD_MASK.to_string()),
            None => Some(visible_value(name, &value, in_card_form).into_owned()),
        }
    }

    /// The words under `node`, as one line short enough to serve as a name.
    fn text_of(&self, node: &AxNode) -> String {
        const MAX_CHARS: usize = 100;
        let mut words = Vec::new();
        let mut pending = node.child_ids.clone().unwrap_or_default();
        pending.reverse();
        while let Some(id) = pending.pop() {
            let Some(child) = self.by_id.get(&id) else {
                continue;
            };
            if str_val(child.role.as_ref()).as_deref() == Some("StaticText") {
                let text = str_val(child.name.as_ref()).unwrap_or_default();
                words.extend(text.split_whitespace().map(str::to_string));
            } else {
                pending.extend(child.child_ids.iter().flatten().rev().cloned());
            }
        }
        let text = words.join(" ");
        match text.char_indices().nth(MAX_CHARS) {
            Some((end, _)) => format!("{}…", &text[..end]),
            None => text,
        }
    }
}

/// Labels that name a control the tree renders. Clicking a `<label>` clicks its control, so
/// Chromium reports the label as clickable; when that control is in the tree with a ref of its
/// own, the label is the same target and gets no second ref. A control the tree leaves out
/// (`display: none`, or ignored like `aria-hidden`) has no ref, so its label stays the target.
/// Only HTML label sources count: an `aria-labelledby` element is a node of its own.
fn labels_of_rendered_controls(nodes: &[AxNode]) -> HashSet<i64> {
    nodes
        .iter()
        .filter(|node| {
            !node.ignored.unwrap_or(false)
                && node.backend_dom_node_id.is_some()
                && str_val(node.role.as_ref()).is_some_and(|role| is_interactive_role(&role))
        })
        .filter_map(|node| node.name.as_ref()?.sources.as_ref())
        .flatten()
        .filter(|source| {
            matches!(
                source.native_source.as_deref(),
                Some("labelfor" | "labelwrapped")
            )
        })
        .filter_map(|source| source.native_source_value.as_ref())
        .flat_map(|value| &value.related_nodes)
        .filter_map(|related| related.backend_dom_node_id)
        .collect()
}

fn entry_node_ids(nodes: &[AxNode]) -> Vec<String> {
    let roots = nodes
        .iter()
        .filter(|node| {
            str_val(node.role.as_ref())
                .as_deref()
                .is_some_and(is_root_role)
        })
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        nodes
            .first()
            .map(|node| vec![node.node_id.clone()])
            .unwrap_or_default()
    } else {
        roots
    }
}

fn is_dropped(role: Option<&str>, name: &str, is_cursor_hit: bool) -> bool {
    let Some(role) = role else {
        return true;
    };
    if is_skip_role(role) || is_root_role(role) {
        return true;
    }
    (role == "generic" || role == "group") && name.is_empty() && !is_cursor_hit
}

/// The line a rendered snapshot shows for `ref_id`, without its indent and list marker.
/// The tag counts only outside quotes: names, urls and values are page text, quoted and
/// escaped by [`json_quote`], so a page cannot write a line that passes for another ref's.
#[must_use]
pub fn ref_line<'t>(text: &'t str, ref_id: &str) -> Option<&'t str> {
    let tag = format!("[ref={ref_id}]");
    split_rendered_lines(text)
        .into_iter()
        .find(|line| unquoted(line).contains(&tag))
        .map(|line| {
            let line = line.trim_start();
            line.strip_prefix("- ").unwrap_or(line)
        })
}

/// `line` with every quoted string removed.
fn unquoted(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let (mut quoted, mut escaped) = (false, false);
    for ch in line.chars() {
        match (quoted, escaped, ch) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (_, _, '"') => quoted = !quoted,
            (false, _, _) => out.push(ch),
            (true, false, _) => {}
        }
    }
    out
}

fn filter_interactive_lines(text: &str) -> String {
    let lines = split_rendered_lines(text);
    if lines.is_empty() {
        return String::new();
    }

    let mut keep = vec![false; lines.len()];
    let mut ancestors = Vec::<usize>::new();
    for (index, line) in lines.iter().enumerate() {
        let depth = rendered_depth(line);
        if ancestors.len() > depth {
            ancestors.truncate(depth);
        }

        if index == 0 || has_ref(line) || rendered_role(line) == Some("heading") {
            keep[index] = true;
            for ancestor in &ancestors {
                keep[*ancestor] = true;
            }
        }

        if ancestors.len() == depth {
            ancestors.push(index);
        } else if depth < ancestors.len() {
            ancestors[depth] = index;
        } else {
            ancestors.resize(depth, index);
            ancestors.push(index);
        }
    }

    lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| keep[index].then_some(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn filter_depth_lines(text: &str, max_depth: usize) -> String {
    split_rendered_lines(text)
        .into_iter()
        .filter(|line| rendered_depth(line) <= max_depth)
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_rendered_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

fn rendered_depth(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count() / 2
}

fn rendered_role(line: &str) -> Option<&str> {
    let body = line.trim_start().strip_prefix("- ")?;
    let end = body
        .find(|ch: char| ch.is_whitespace() || ch == '[' || ch == ':')
        .unwrap_or(body.len());
    Some(&body[..end])
}

fn has_ref(line: &str) -> bool {
    line.contains(" [ref=e")
}

/// A node's name as the tree has it, which refs are minted and recovered by, and as its line
/// shows it, which differs only for a nameless label read by its text.
struct LineName<'a> {
    tree: &'a str,
    shown: &'a str,
}

fn format_line(
    node: &AxNode,
    role: &str,
    name: LineName<'_>,
    depth: usize,
    value: Option<&str>,
    cursor_hit: bool,
    opts: &mut RenderOptions<'_>,
) -> String {
    let mut line = format!("{}- {role}", "  ".repeat(depth));
    if !name.shown.is_empty() {
        line.push(' ');
        line.push_str(&json_quote(name.shown));
    }

    for state in format_states(node) {
        line.push_str(" [");
        line.push_str(&state);
        line.push(']');
    }

    let actionable =
        node.backend_dom_node_id.is_some() && (is_interactive_role(role) || cursor_hit);

    if actionable && let Some(backend_node_id) = node.backend_dom_node_id {
        let ref_id = opts.refs.mint(MintRef {
            backend_node_id,
            role,
            name: name.tree,
            document_id: opts.document_id.as_deref(),
            frame_id: opts.frame_id.as_ref(),
        });
        line.push_str(" [ref=");
        line.push_str(ref_id.as_str());
        line.push(']');
    }
    if cursor_hit {
        line.push_str(" [cursor=pointer]");
    }

    if role == "link"
        && let Some(url) = node
            .properties
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|property| property.name == "url")
            .and_then(|property| str_val(Some(&property.value)))
            .filter(|url| !url.is_empty())
    {
        line.push_str(" [url=");
        line.push_str(&json_quote(&url));
        line.push(']');
    }

    if let Some(value) = value {
        line.push_str(": ");
        line.push_str(&json_quote(value));
    }

    line
}

/// Card numbers and security codes stay out of snapshots. After a payment card
/// fill they sit in plain text fields, unlike passwords, which the browser
/// already masks. A card number keeps its last four digits. A 3-4 digit value
/// is hidden inside a card form, whatever its field is called, and elsewhere
/// in a field named like a security code.
fn visible_value<'a>(name: &str, value: &'a str, in_card_form: bool) -> std::borrow::Cow<'a, str> {
    let Some(digits) = digit_run(value) else {
        return value.into();
    };
    if is_card_number(&digits) {
        let last_four: String = digits[digits.len() - 4..]
            .iter()
            .map(|digit| char::from_digit(*digit, 10).unwrap_or('0'))
            .collect();
        return format!("•••• {last_four}").into();
    }
    let name = name.to_ascii_lowercase();
    if (3..=4).contains(&digits.len())
        && (in_card_form
            || [
                "cvc",
                "cvv",
                "csc",
                "security code",
                "card code",
                "verification",
            ]
            .iter()
            .any(|label| name.contains(label)))
    {
        return "•••".into();
    }
    value.into()
}

/// The digits of a value made only of digits, spaces and dashes.
fn digit_run(value: &str) -> Option<Vec<u32>> {
    value
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
        .then(|| value.chars().filter_map(|c| c.to_digit(10)).collect())
}

fn is_card_number(digits: &[u32]) -> bool {
    (13..=19).contains(&digits.len()) && passes_luhn(digits)
}

/// The nodes whose subtrees are card forms: for each value holding a card
/// number, its nearest `form` ancestor, or its root when it has none.
fn card_form_scopes(nodes: &[AxNode]) -> HashSet<String> {
    let parents: HashMap<&str, &str> = nodes
        .iter()
        .flat_map(|node| {
            node.child_ids
                .iter()
                .flatten()
                .map(move |child| (child.as_str(), node.node_id.as_str()))
        })
        .collect();
    let roles: HashMap<&str, Option<String>> = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), str_val(node.role.as_ref())))
        .collect();
    let mut scopes = HashSet::new();
    for node in nodes {
        let holds_card = str_val(node.value.as_ref())
            .and_then(|value| digit_run(&value))
            .is_some_and(|digits| is_card_number(&digits));
        if !holds_card {
            continue;
        }
        let mut scope = node.node_id.as_str();
        // Bounded by the node count, so a malformed cyclic tree can't spin.
        for _ in 0..nodes.len() {
            let Some(parent) = parents.get(scope) else {
                break;
            };
            scope = parent;
            if roles
                .get(scope)
                .is_some_and(|role| role.as_deref() == Some("form"))
            {
                break;
            }
        }
        scopes.insert(scope.to_owned());
    }
    scopes
}

fn format_states(node: &AxNode) -> Vec<String> {
    let mut states = Vec::new();
    for prop in node.properties.as_deref().unwrap_or(&[]) {
        let value = prop.value.value.as_ref();
        match prop.name.as_str() {
            "checked" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("checked".to_string());
                } else if value == Some(&Value::String("mixed".to_string())) {
                    states.push("indeterminate".to_string());
                }
            }
            "disabled" if value == Some(&Value::Bool(true)) => {
                states.push("disabled".to_string());
            }
            "expanded" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("expanded".to_string());
                } else if value == Some(&Value::Bool(false)) {
                    states.push("collapsed".to_string());
                }
            }
            "required" if value == Some(&Value::Bool(true)) => {
                states.push("required".to_string());
            }
            "selected" if value == Some(&Value::Bool(true)) => {
                states.push("selected".to_string());
            }
            "level" => {
                if let Some(value) = value {
                    states.push(format!("level={}", value_display(value)));
                }
            }
            _ => {}
        }
    }
    states
}

fn str_val(value: Option<&AxValue>) -> Option<String> {
    value.and_then(|value| value.value.as_ref()?.as_str().map(ToString::to_string))
}

fn value_display(value: &Value) -> String {
    if let Some(number) = value.as_i64() {
        number.to_string()
    } else if let Some(number) = value.as_u64() {
        number.to_string()
    } else if let Some(number) = value.as_f64() {
        number.to_string()
    } else if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    }
}

fn json_quote(value: &str) -> String {
    match serde_json::to_string(value) {
        Ok(quoted) => quoted,
        Err(_err) => "\"\"".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FrameFields, RenderOptions, SnapshotMode, SnapshotOptions, apply_snapshot_options,
        ref_line, render_snapshot, render_snapshot_masked,
    };
    use crate::sensitive_fields::{FieldAttributes, FieldTag, FieldValues};
    use crate::snapshot::{
        AxNode, AxProperty, AxRelatedNode, AxRelatedValue, AxValue, AxValueSource, refs::RefMap,
    };
    use serde_json::json;
    use std::collections::HashMap;

    fn ax(node_id: &str, role: &str, children: &[&str]) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role(role)),
            child_ids: (!children.is_empty())
                .then(|| children.iter().map(|child| (*child).to_string()).collect()),
            ..AxNode::default()
        }
    }

    fn name(value: &str) -> AxValue {
        AxValue::string(value)
    }

    fn render(nodes: &[AxNode], refs: &mut RefMap) -> String {
        let mut opts = RenderOptions {
            refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 0,
        };
        render_snapshot(nodes, &mut opts).text
    }

    /// A form with password, card, code and plain fields, and a paragraph that echoes the code.
    fn sign_in_form() -> Vec<AxNode> {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "7"]),
            ax("2", "form", &["3", "4", "5", "6"]),
            ax("3", "textbox", &[]),
            ax("4", "textbox", &[]),
            ax("5", "textbox", &[]),
            ax("6", "textbox", &[]),
            ax("7", "paragraph", &["8"]),
            ax("8", "StaticText", &[]),
        ];
        let fields = [
            ("Password", "••••••••••••", 201),
            ("Card number", "4111111111111111", 202),
            ("One-time code", "482913", 203),
            ("Notes", "NOTES_visible_ok", 204),
        ];
        for (index, (label, value, backend)) in fields.into_iter().enumerate() {
            let node = &mut nodes[index + 2];
            node.name = Some(name(label));
            node.backend_dom_node_id = Some(backend);
            node.value = Some(AxValue {
                value_type: "string".to_string(),
                value: Some(json!(value)),
                sources: None,
            });
        }
        nodes[6].name = Some(name("We sent code 482913 to your phone"));
        nodes
    }

    fn render_masked(nodes: &[AxNode], fields: &FrameFields) -> (String, FieldValues) {
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 0,
        };
        let masked = render_snapshot_masked(nodes, &mut opts, fields);
        (masked.result.text, masked.values)
    }

    fn field(configure: impl FnOnce(&mut FieldAttributes)) -> FieldAttributes {
        let mut field = FieldAttributes::new(FieldTag::Input);
        configure(&mut field);
        field
    }

    #[test]
    fn masking_hides_every_sensitive_field_and_reports_its_value_for_echoes() {
        let fields = FrameFields {
            fields: [
                (201, field(|f| f.input_type = Some("password".into()))),
                (202, field(|f| f.autocomplete = Some("cc-number".into()))),
                (
                    203,
                    field(|f| f.autocomplete = Some("one-time-code".into())),
                ),
                (204, FieldAttributes::new(FieldTag::Textarea)),
            ]
            .into_iter()
            .collect(),
            degraded: false,
        };
        let (mut text, values) = render_masked(&sign_in_form(), &fields);
        assert!(
            text.contains(r#"textbox "Password" [ref=e1]: "••••""#),
            "{text}"
        );
        assert!(
            text.contains(r#"textbox "Card number" [ref=e2]: "•••• 1111""#),
            "{text}"
        );
        assert!(
            text.contains(r#"textbox "One-time code" [ref=e3]: "••••""#),
            "{text}"
        );
        assert!(
            text.contains(r#"textbox "Notes" [ref=e4]: "NOTES_visible_ok""#),
            "{text}"
        );
        assert!(
            text.contains("482913"),
            "the echo is the observer's job: {text}"
        );
        // Chromium's bullets are never treated as a value to look for elsewhere.
        assert_eq!(
            values.registrable().collect::<Vec<_>>(),
            ["4111111111111111", "482913"]
        );
        assert!(values.mask_str(&mut text));
        assert!(
            !text.contains("482913") && !text.contains("4111111111111111"),
            "{text}"
        );
    }

    #[test]
    fn masking_falls_back_to_the_accessibility_node_when_the_dom_was_not_read() {
        let degraded = FrameFields {
            fields: HashMap::new(),
            degraded: true,
        };
        let (text, _) = render_masked(&sign_in_form(), &degraded);
        assert!(text.contains(r#""Password" [ref=e1]: "••••""#), "{text}");
        assert!(
            text.contains(r#""Card number" [ref=e2]: "•••• 1111""#),
            "{text}"
        );
        assert!(
            text.contains(r#""One-time code" [ref=e3]: "••••""#),
            "{text}"
        );
        assert!(
            text.contains(r#""Notes" [ref=e4]: "NOTES_visible_ok""#),
            "{text}"
        );
    }

    #[test]
    fn masking_hides_bullet_lengths_and_keeps_the_heuristic_for_unclassified_fields() {
        // Read, but none of the fields matched a rule: only the second net applies.
        let unmatched = FrameFields::default();
        let (text, values) = render_masked(&sign_in_form(), &unmatched);
        assert!(text.contains(r#""Password" [ref=e1]: "••••""#), "{text}");
        assert!(
            text.contains(r#""Card number" [ref=e2]: "•••• 1111""#),
            "{text}"
        );
        assert!(
            text.contains(r#""One-time code" [ref=e3]: "482913""#),
            "{text}"
        );
        assert!(values.is_empty());
    }

    #[test]
    fn the_heuristic_render_is_unchanged() {
        let text = render(&sign_in_form(), &mut RefMap::new());
        assert!(
            text.contains(r#""Password" [ref=e1]: "••••••••••••""#),
            "{text}"
        );
        assert!(
            text.contains(r#""Card number" [ref=e2]: "•••• 1111""#),
            "{text}"
        );
        assert!(
            text.contains(r#""One-time code" [ref=e3]: "482913""#),
            "{text}"
        );
    }

    fn render_with_snapshot_options(
        nodes: &[AxNode],
        refs: &mut RefMap,
        options: SnapshotOptions,
    ) -> String {
        apply_snapshot_options(&render(nodes, refs), options)
    }

    #[test]
    fn renders_semantic_tree_with_refs_only_on_actionable_nodes() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "6"]),
            ax("2", "navigation", &["3"]),
            ax("3", "link", &[]),
            ax("6", "main", &["7", "8", "9"]),
            ax("7", "heading", &[]),
            ax("8", "button", &[]),
            ax("9", "generic", &["10"]),
            ax("10", "textbox", &[]),
        ];
        nodes[2].name = Some(name("Home"));
        nodes[2].backend_dom_node_id = Some(101);
        nodes[4].name = Some(name("Results"));
        nodes[4].backend_dom_node_id = Some(110);
        nodes[4].properties = Some(vec![AxProperty {
            name: "level".to_string(),
            value: AxValue {
                value_type: "integer".to_string(),
                value: Some(json!(1)),
                sources: None,
            },
        }]);
        nodes[5].name = Some(name("Load more"));
        nodes[5].backend_dom_node_id = Some(111);
        nodes[5].properties = Some(vec![AxProperty {
            name: "disabled".to_string(),
            value: AxValue {
                value_type: "boolean".to_string(),
                value: Some(json!(true)),
                sources: None,
            },
        }]);
        nodes[7].name = Some(name("Search"));
        nodes[7].backend_dom_node_id = Some(112);
        nodes[7].value = Some(AxValue {
            value_type: "string".to_string(),
            value: Some(json!("abc")),
            sources: None,
        });

        let text = render(&nodes, &mut RefMap::new());
        assert_eq!(
            text,
            [
                "- navigation",
                "  - link \"Home\" [ref=e1]",
                "- main",
                "  - heading \"Results\" [level=1]",
                "  - button \"Load more\" [disabled] [ref=e2]",
                "  - textbox \"Search\" [ref=e3]: \"abc\"",
            ]
            .join("\n")
        );
    }

    #[test]
    fn a_slider_shows_its_numeric_value() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "slider", &[])];
        nodes[1].name = Some(name("Volume"));
        nodes[1].backend_dom_node_id = Some(200);
        nodes[1].value = Some(AxValue {
            value_type: "number".to_string(),
            value: Some(json!(75)),
            sources: None,
        });
        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            "- slider \"Volume\" [ref=e1]: \"75\""
        );
    }

    #[test]
    fn date_time_and_color_inputs_get_refs_and_show_their_value() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "3", "4", "5"]),
            ax("2", "Date", &[]),
            ax("3", "DateTime", &[]),
            ax("4", "InputTime", &[]),
            ax("5", "ColorWell", &[]),
        ];
        for (index, (label, value)) in [
            ("Start", "2026-10-29"),
            ("Meeting", "2026-10-29T14:30"),
            ("Alarm", "07:15"),
            ("Accent", "#ff8800"),
        ]
        .into_iter()
        .enumerate()
        {
            let node = &mut nodes[index + 1];
            node.name = Some(name(label));
            node.backend_dom_node_id = Some(200 + index as i64);
            node.value = Some(AxValue {
                value_type: "string".to_string(),
                value: Some(json!(value)),
                sources: None,
            });
        }

        let text = render(&nodes, &mut RefMap::new());
        assert_eq!(
            text,
            [
                "- Date \"Start\" [ref=e1]: \"2026-10-29\"",
                "- DateTime \"Meeting\" [ref=e2]: \"2026-10-29T14:30\"",
                "- InputTime \"Alarm\" [ref=e3]: \"07:15\"",
                "- ColorWell \"Accent\" [ref=e4]: \"#ff8800\"",
            ]
            .join("\n")
        );
    }

    #[test]
    fn tests_that_links_keep_urls() -> Result<(), serde_json::Error> {
        let nodes: Vec<AxNode> = serde_json::from_value(json!([
            {
                "nodeId": "1",
                "role": {"type": "role", "value": "RootWebArea"},
                "childIds": ["2", "3", "4"]
            },
            {
                "nodeId": "2", "backendDOMNodeId": 101,
                "role": {"type": "role", "value": "link"},
                "name": {"type": "computedString", "value": "Read more"},
                "properties": [{"name": "url", "value": {
                    "type": "string", "value": "https://first.example/story"
                }}]
            },
            {
                "nodeId": "3", "backendDOMNodeId": 102,
                "role": {"type": "role", "value": "link"},
                "name": {"type": "computedString", "value": "Read more"},
                "properties": [{"name": "url", "value": {
                    "type": "string", "value": "https://second.example/[story]?q=\"hello\""
                }}]
            },
            {
                "nodeId": "4", "backendDOMNodeId": 103,
                "role": {"type": "role", "value": "button"},
                "name": {"type": "computedString", "value": "Next"}
            }
        ]))?;

        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            concat!(
                "- link \"Read more\" [ref=e1] [url=\"https://first.example/story\"]\n",
                "- link \"Read more\" [ref=e2] [url=\"https://second.example/[story]?q=\\\"hello\\\"\"]\n",
                "- button \"Next\" [ref=e3]"
            )
        );
        Ok(())
    }

    #[test]
    fn drops_unnamed_generic_wrappers_lifting_children() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "generic", &["3"]),
            ax("3", "button", &[]),
        ];
        nodes[2].name = Some(name("Go"));
        nodes[2].backend_dom_node_id = Some(5);
        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            "- button \"Go\" [ref=e1]"
        );
    }

    #[test]
    fn mints_distinct_nth_for_duplicate_role_name() {
        let mut refs = RefMap::new();
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "button", &[]),
        ];
        nodes[1].name = Some(name("OK"));
        nodes[1].backend_dom_node_id = Some(1);
        nodes[2].name = Some(name("OK"));
        nodes[2].backend_dom_node_id = Some(2);
        let _ = render(&nodes, &mut refs);
        assert_eq!(
            refs.get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.nth),
            Some(0)
        );
        assert_eq!(
            refs.get(&crate::Ref("e2".to_string()))
                .map(|entry| entry.nth),
            Some(1)
        );
    }

    #[test]
    fn reuses_refs_for_same_document_backend_nodes_after_insertion() {
        let mut refs = RefMap::new();
        let mut before = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "link", &[]),
        ];
        before[1].name = Some(name("A"));
        before[1].backend_dom_node_id = Some(1);
        before[2].name = Some(name("B"));
        before[2].backend_dom_node_id = Some(2);
        let mut after = vec![
            ax("1", "RootWebArea", &["4", "2", "3"]),
            ax("4", "button", &[]),
            ax("2", "button", &[]),
            ax("3", "link", &[]),
        ];
        after[1].name = Some(name("X"));
        after[1].backend_dom_node_id = Some(3);
        after[2].name = Some(name("A"));
        after[2].backend_dom_node_id = Some(1);
        after[3].name = Some(name("B"));
        after[3].backend_dom_node_id = Some(2);

        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: Some("main:loader-1".to_string()),
            cursor_hits: None,
            base_depth: 0,
        };
        let first = render_snapshot(&before, &mut opts).text;
        refs.begin_snapshot();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: Some("main:loader-1".to_string()),
            cursor_hits: None,
            base_depth: 0,
        };
        let second = render_snapshot(&after, &mut opts).text;

        assert_eq!(first, "- button \"A\" [ref=e1]\n- link \"B\" [ref=e2]");
        assert_eq!(
            second,
            [
                "- button \"X\" [ref=e3]",
                "- button \"A\" [ref=e1]",
                "- link \"B\" [ref=e2]"
            ]
            .join("\n")
        );
    }

    #[test]
    fn marks_cursor_augmented_non_aria_nodes_actionable() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "generic", &[])];
        nodes[1].name = Some(name("Fake button"));
        nodes[1].backend_dom_node_id = Some(42);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(HashMap::from([(42, vec!["onclick".to_string()])])),
            base_depth: 0,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "- generic \"Fake button\" [ref=e1] [cursor=pointer]"
        );
    }

    #[test]
    fn escapes_quotes_in_names() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "button", &[])];
        nodes[1].name = Some(name("Say \"hi\""));
        nodes[1].backend_dom_node_id = Some(1);
        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            "- button \"Say \\\"hi\\\"\" [ref=e1]"
        );
    }

    #[test]
    fn reports_iframe_nodes_as_stitch_points() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "Iframe", &[]),
        ];
        nodes[1].name = Some(name("Outer"));
        nodes[1].backend_dom_node_id = Some(7);
        nodes[2].backend_dom_node_id = Some(8);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 0,
        };
        let out = render_snapshot(&nodes, &mut opts);
        assert_eq!(out.text, "- button \"Outer\" [ref=e1]\n- iframe");
        assert_eq!(out.iframes[0].backend_node_id, 8);
        assert_eq!(out.iframes[0].line_index, 1);
        assert_eq!(out.iframes[0].depth, 0);
    }

    #[test]
    fn honours_base_depth_for_spliced_child_frames() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "button", &[])];
        nodes[1].name = Some(name("Inner"));
        nodes[1].backend_dom_node_id = Some(1);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 2,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "    - button \"Inner\" [ref=e1]"
        );
    }

    #[test]
    fn keeps_nameless_generic_when_cursor_interactive() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "generic", &[])];
        nodes[1].backend_dom_node_id = Some(9);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(HashMap::from([(9, vec!["cursor:pointer".to_string()])])),
            base_depth: 0,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "- generic [ref=e1] [cursor=pointer]"
        );
    }

    /// A control's name as Chromium reports it when a `<label>` supplies it: the source names the
    /// label node by backend id.
    fn named_by_label(native_source: &str, label: i64, text: &str) -> AxValue {
        AxValue {
            sources: Some(vec![AxValueSource {
                native_source: Some(native_source.to_string()),
                native_source_value: Some(AxRelatedValue {
                    related_nodes: vec![AxRelatedNode {
                        backend_dom_node_id: Some(label),
                    }],
                }),
            }]),
            ..AxValue::string(text)
        }
    }

    fn render_with_hits(nodes: &[AxNode], hits: &[i64]) -> String {
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(
                hits.iter()
                    .map(|&id| (id, vec!["clickable".to_string()]))
                    .collect(),
            ),
            base_depth: 0,
        };
        render_snapshot(nodes, &mut opts).text
    }

    #[test]
    fn ref_line_finds_exactly_that_ref() {
        let text = "- form\n  - textbox \"Phone\" [ref=e2]: \"(555) 123-4567\"\n  - textbox \"Name\" [ref=e20]";
        assert_eq!(
            ref_line(text, "e2"),
            Some("textbox \"Phone\" [ref=e2]: \"(555) 123-4567\"")
        );
        assert_eq!(ref_line(text, "e20"), Some("textbox \"Name\" [ref=e20]"));
        assert_eq!(ref_line(text, "e3"), None);
        assert_eq!(ref_line("- textbox \"Name\" [ref=e20]", "e2"), None);
    }

    #[test]
    fn ref_line_ignores_a_ref_tag_inside_page_text() {
        // A page can name a heading or link anything; quoted text never stands for a ref.
        let text = "- heading \"Code [ref=e2]: \\\"000 000\\\"\" [level=1]\n- link \"x\" [ref=e1] [url=\"https://a.test/[ref=e2]\"]\n- textbox \"Code\" [ref=e2]: \"440 212\"";
        assert_eq!(
            ref_line(text, "e2"),
            Some("textbox \"Code\" [ref=e2]: \"440 212\"")
        );
    }

    #[test]
    fn a_label_is_part_of_the_control_it_names() {
        // <label for="email">Email</label><input id="email">: Chromium calls the label
        // clickable because clicking it clicks the input, which has a ref of its own.
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "4"]),
            ax("2", "LabelText", &["3"]),
            ax("3", "StaticText", &[]),
            ax("4", "textbox", &[]),
        ];
        nodes[1].backend_dom_node_id = Some(6);
        nodes[2].name = Some(name("Email"));
        nodes[3].backend_dom_node_id = Some(7);
        nodes[3].name = Some(named_by_label("labelfor", 6, "Email"));
        assert_eq!(
            render_with_hits(&nodes, &[6]),
            "- LabelText\n- textbox \"Email\" [ref=e1]"
        );

        // <label>Wrapped <input></label>
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "LabelText", &["3", "4"]),
            ax("3", "StaticText", &[]),
            ax("4", "checkbox", &[]),
        ];
        nodes[1].backend_dom_node_id = Some(6);
        nodes[2].name = Some(name("Wrapped"));
        nodes[3].backend_dom_node_id = Some(7);
        nodes[3].name = Some(named_by_label("labelwrapped", 6, "Wrapped"));
        assert_eq!(
            render_with_hits(&nodes, &[6]),
            "- LabelText\n  - checkbox \"Wrapped\" [ref=e1]"
        );
    }

    #[test]
    fn a_label_whose_control_is_not_rendered_keeps_a_ref_named_by_its_text() {
        // <input type="checkbox" id="remember" hidden><label for="remember">Remember me</label>:
        // the hidden input is not in the tree, so the label is the only thing to click.
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "LabelText", &["3"]),
            ax("3", "StaticText", &[]),
        ];
        nodes[1].backend_dom_node_id = Some(9);
        nodes[2].name = Some(name("Remember  me"));
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(HashMap::from([(9, vec!["clickable".to_string()])])),
            base_depth: 0,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "- LabelText \"Remember me\" [ref=e1] [cursor=pointer]"
        );
        // The shown name is for reading. The ref keeps the tree's own (empty) name, which is what
        // recovering a ref whose node was replaced matches against.
        assert_eq!(
            refs.get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.name.as_str()),
            Some("")
        );
    }

    #[test]
    fn a_label_of_an_ignored_control_keeps_its_ref() {
        // aria-hidden on the input takes it out of the tree's controls: no ref of its own.
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "4"]),
            ax("2", "LabelText", &["3"]),
            ax("3", "StaticText", &[]),
            ax("4", "checkbox", &[]),
        ];
        nodes[1].backend_dom_node_id = Some(6);
        nodes[2].name = Some(name("Agree"));
        nodes[3].backend_dom_node_id = Some(7);
        nodes[3].ignored = Some(true);
        nodes[3].name = Some(named_by_label("labelfor", 6, "Agree"));
        assert_eq!(
            render_with_hits(&nodes, &[6]),
            "- LabelText \"Agree\" [ref=e1] [cursor=pointer]"
        );
    }

    #[test]
    fn interactive_mode_keeps_refs_headings_first_line_and_ancestors() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4", "5"]),
            ax("3", "paragraph", &[]),
            ax("4", "section", &["6"]),
            ax("5", "heading", &[]),
            ax("6", "button", &[]),
        ];
        nodes[1].name = Some(name("App"));
        nodes[2].name = Some(name("Intro copy"));
        nodes[3].name = Some(name("Actions"));
        nodes[4].name = Some(name("Results"));
        nodes[5].name = Some(name("Save"));
        nodes[5].backend_dom_node_id = Some(10);

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            },
        );

        assert_eq!(
            text,
            [
                "- main \"App\"",
                "  - section \"Actions\"",
                "    - button \"Save\" [ref=e1]",
                "  - heading \"Results\"",
            ]
            .join("\n")
        );
    }

    #[test]
    fn depth_cap_drops_lines_nested_deeper_than_limit() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3"]),
            ax("3", "section", &["4"]),
            ax("4", "button", &[]),
        ];
        nodes[3].name = Some(name("Deep"));
        nodes[3].backend_dom_node_id = Some(10);

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Full,
                depth: Some(1),
            },
        );

        assert_eq!(text, ["- main", "  - section"].join("\n"));
    }

    #[test]
    fn interactive_filter_runs_before_depth_cap() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "5"]),
            ax("3", "section", &["4"]),
            ax("4", "button", &[]),
            ax("5", "paragraph", &[]),
        ];
        nodes[3].name = Some(name("Deep"));
        nodes[3].backend_dom_node_id = Some(10);
        nodes[4].name = Some(name("Static"));

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: Some(1),
            },
        );

        assert_eq!(text, ["- main", "  - section"].join("\n"));
    }

    #[test]
    fn refs_are_identical_across_full_and_interactive_modes() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4", "5"]),
            ax("3", "paragraph", &[]),
            ax("4", "button", &[]),
            ax("5", "link", &[]),
        ];
        nodes[2].name = Some(name("Static"));
        nodes[3].name = Some(name("Save"));
        nodes[3].backend_dom_node_id = Some(10);
        nodes[4].name = Some(name("Home"));
        nodes[4].backend_dom_node_id = Some(11);

        let mut full_refs = RefMap::new();
        let full = render_with_snapshot_options(
            &nodes,
            &mut full_refs,
            SnapshotOptions {
                mode: SnapshotMode::Full,
                depth: None,
            },
        );
        let mut interactive_refs = RefMap::new();
        let interactive = render_with_snapshot_options(
            &nodes,
            &mut interactive_refs,
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            },
        );

        assert_eq!(
            full_refs
                .entries_in_order()
                .into_iter()
                .map(|entry| (entry.ref_id.as_str().to_string(), entry.backend_node_id))
                .collect::<Vec<_>>(),
            interactive_refs
                .entries_in_order()
                .into_iter()
                .map(|entry| (entry.ref_id.as_str().to_string(), entry.backend_node_id))
                .collect::<Vec<_>>()
        );
        assert!(full.contains("paragraph \"Static\""));
        assert!(!interactive.contains("paragraph \"Static\""));
        assert!(interactive.contains("button \"Save\" [ref=e1]"));
        assert!(interactive.contains("link \"Home\" [ref=e2]"));
    }

    #[test]
    fn default_snapshot_options_preserve_full_output_byte_for_byte() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4"]),
            ax("3", "paragraph", &[]),
            ax("4", "button", &[]),
        ];
        nodes[2].name = Some(name("Static"));
        nodes[3].name = Some(name("Save"));
        nodes[3].backend_dom_node_id = Some(10);

        let mut refs = RefMap::new();
        let full = render(&nodes, &mut refs);
        let with_defaults = apply_snapshot_options(&full, SnapshotOptions::default());

        assert_eq!(with_defaults, full);
    }

    #[test]
    fn snapshots_mask_card_numbers_and_security_codes() {
        let field = |id: &str, label: &str, value: &str| {
            let mut node = ax(id, "textbox", &[]);
            node.name = Some(name(label));
            node.value = Some(AxValue {
                value_type: "string".to_string(),
                value: Some(json!(value)),
                sources: None,
            });
            node
        };
        let mut refs = RefMap::default();
        let text = render(
            &[
                ax("1", "RootWebArea", &["6", "7"]),
                ax("6", "form", &["2"]),
                field("2", "Card number", "4111 1111 1111 1111"),
                // Outside the card form, only a security-code name hides a value.
                ax("7", "form", &["3", "4", "5"]),
                field("3", "CVC", "123"),
                field("4", "Quantity", "123"),
                field("5", "Order number", "4111111111111112"),
            ],
            &mut refs,
        );
        assert!(text.contains("\"•••• 1111\""), "{text}");
        assert!(!text.contains("4111 1111 1111 1111"), "{text}");
        assert!(text.contains("\"CVC\": \"•••\""), "{text}");
        assert!(text.contains("\"Quantity\": \"123\""), "{text}");
        // Not checksum-valid, so not a card number.
        assert!(text.contains("4111111111111112"), "{text}");
    }

    #[test]
    fn card_forms_hide_short_numbers_whatever_the_field_is_called() {
        let field = |id: &str, label: &str, value: &str| {
            let mut node = ax(id, "textbox", &[]);
            node.name = Some(name(label));
            node.value = Some(AxValue {
                value_type: "string".to_string(),
                value: Some(json!(value)),
                sources: None,
            });
            node
        };
        let mut refs = RefMap::default();
        let text = render(
            &[
                ax("1", "RootWebArea", &["2", "6"]),
                ax("2", "form", &["3", "4", "5"]),
                field("3", "Numéro de carte", "4111 1111 1111 1111"),
                field("4", "Code", "123"),
                field("5", "Mois", "12"),
                ax("6", "form", &["7"]),
                field("7", "Quantity", "123"),
            ],
            &mut refs,
        );
        assert!(text.contains("\"Code\": \"•••\""), "{text}");
        assert!(text.contains("\"Mois\": \"12\""), "{text}");
        // Another form on the page keeps its numbers.
        assert!(text.contains("\"Quantity\": \"123\""), "{text}");
    }
}
