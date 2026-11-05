//! Form controls of one frame's document, read from the same `DOMSnapshot` capture the clickable
//! scan takes, so masking sensitive fields costs no extra CDP call.
//!
//! The snapshot renderer pairs each control's attributes with its accessibility name and asks
//! [`FieldRules`](crate::sensitive_fields::FieldRules) whether it holds a secret. Only
//! attributes and the text of the field's form are read here: the capture also carries every
//! typed value (`inputValue`, `textValue`), and those columns are never parsed.
//!
//! Form text follows `seal.js`, which reads `closest("form").textContent`: text nodes under the
//! nearest `<form>` ancestor, not crossing into or out of a shadow root, so a user-agent shadow
//! root's text (where Chromium keeps an input's typed value) is never part of it.

use super::{ClickableDocument, Document, DomSnapshots, View, select_document};
use crate::ProtocolSession;
use crate::observer::acquisition::SnapshotBudget;
use crate::sensitive_fields::{FieldAttributes, FieldRules, FieldTag};
use crate::snapshot::FrameReadReason;
use std::collections::HashMap;

/// The frame's form controls by backend node id, or why its document could not be read.
pub(in crate::observer) async fn find_form_fields(
    snapshots: &DomSnapshots,
    session: &ProtocolSession,
    document: ClickableDocument,
    budget: &SnapshotBudget,
) -> Result<HashMap<i64, FieldAttributes>, FrameReadReason> {
    if let ClickableDocument::Unavailable = document {
        return Err(FrameReadReason::NotFound);
    }
    let capture = snapshots.capture(session, budget).await?;
    let selected = select_document(&capture, document).ok_or(FrameReadReason::NotFound)?;
    Ok(form_fields(selected, &capture.strings))
}

fn form_fields(document: &Document, strings: &[String]) -> HashMap<i64, FieldAttributes> {
    let view = View::new(document, strings);
    let count = view.len();
    let limit = FieldRules::get().form_text_limit();

    // Pre-order: a parent's facts are known before its children's. A shadow root starts a new
    // scope, as `closest()` and `textContent` do.
    let mut form_of: Vec<Option<usize>> = vec![None; count];
    let mut in_user_agent_shadow = vec![false; count];
    for i in 0..count {
        let shadow_root = view.shadow_root_type(i);
        let Some(parent) = view.parent(i).filter(|&parent| parent < i) else {
            in_user_agent_shadow[i] = shadow_root == Some("user-agent");
            continue;
        };
        in_user_agent_shadow[i] = shadow_root == Some("user-agent") || in_user_agent_shadow[parent];
        if shadow_root.is_some() {
            continue;
        }
        form_of[i] = if view.is_element(parent) && view.name(parent) == "form" {
            Some(parent)
        } else {
            form_of[parent]
        };
    }

    let mut form_text: HashMap<usize, String> = HashMap::new();
    for (i, form) in form_of.iter().enumerate() {
        let (Some(form), true) = (*form, view.is_text(i)) else {
            continue;
        };
        let text = form_text.entry(form).or_default();
        if text.chars().count() < limit {
            text.push_str(view.node_value(i).unwrap_or_default());
        }
    }

    let mut fields = HashMap::new();
    for i in 0..count {
        if !view.is_element(i) || in_user_agent_shadow[i] {
            continue;
        }
        let Some(tag) = FieldTag::from_name(&view.name(i)) else {
            continue;
        };
        let Some(backend_id) = view.backend_id(i) else {
            continue;
        };
        let attribute = |name: &str| view.attribute(i, name).map(str::to_string);
        let mut field = FieldAttributes::new(tag);
        field.input_type = attribute("type");
        field.autocomplete = attribute("autocomplete");
        field.name = attribute("name");
        field.id = attribute("id");
        field.aria_label = attribute("aria-label");
        field.placeholder = attribute("placeholder");
        field.title = attribute("title");
        field.max_length = view
            .attribute(i, "maxlength")
            .and_then(|value| value.trim().parse().ok());
        field.input_mode = attribute("inputmode");
        field.form_text = form_of[i].map(|form| {
            let text = form_text.get(&form).map(String::as_str).unwrap_or_default();
            text.chars().take(limit).collect()
        });
        fields.insert(backend_id, field);
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::super::{Document, Layout, Nodes};
    use super::form_fields;
    use crate::sensitive_fields::{FieldRules, FieldTag, SensitiveKind};

    const ELEMENT: i64 = 1;
    const TEXT: i64 = 3;
    const FRAGMENT: i64 = 11;

    /// A captured document built column by column, the way Chromium serializes it.
    struct Capture {
        strings: Vec<String>,
        nodes: Nodes,
    }

    impl Capture {
        fn new() -> (Self, usize) {
            let mut capture = Self {
                strings: Vec::new(),
                nodes: Nodes::default(),
            };
            let document = capture.node(None, 9, "#document", &[]);
            (capture, document)
        }

        fn string(&mut self, value: &str) -> i64 {
            let index = self
                .strings
                .iter()
                .position(|known| known == value)
                .unwrap_or_else(|| {
                    self.strings.push(value.to_string());
                    self.strings.len() - 1
                });
            i64::try_from(index).unwrap_or(i64::MAX)
        }

        fn node(
            &mut self,
            parent: Option<usize>,
            node_type: i64,
            name: &str,
            attributes: &[(&str, &str)],
        ) -> usize {
            let index = self.nodes.node_name.len();
            self.nodes
                .parent_index
                .push(parent.map_or(-1, |parent| parent as i64));
            self.nodes.node_type.push(node_type);
            let name = self.string(name);
            self.nodes.node_name.push(name);
            self.nodes.backend_node_id.push(1000 + index as i64);
            let flat = attributes
                .iter()
                .flat_map(|(key, value)| [self.string(key), self.string(value)])
                .collect();
            self.nodes.attributes.push(flat);
            self.nodes.node_value.push(-1);
            index
        }

        fn text(&mut self, parent: usize, value: &str) -> usize {
            let index = self.node(Some(parent), TEXT, "#text", &[]);
            self.nodes.node_value[index] = self.string(value);
            index
        }

        fn shadow_root(&mut self, host: usize, kind: &str) -> usize {
            let root = self.node(Some(host), FRAGMENT, "#document-fragment", &[]);
            self.nodes.shadow_root_type.index.push(root as i64);
            let kind = self.string(kind);
            self.nodes.shadow_root_type.value.push(kind);
            root
        }

        fn fields(
            self,
        ) -> std::collections::HashMap<i64, crate::sensitive_fields::FieldAttributes> {
            let document = Document {
                nodes: self.nodes,
                layout: Layout::default(),
            };
            form_fields(&document, &self.strings)
        }
    }

    fn backend(index: usize) -> i64 {
        1000 + index as i64
    }

    #[test]
    fn reads_attributes_and_form_text_but_never_a_typed_value() {
        let (mut page, document) = Capture::new();
        let form = page.node(Some(document), ELEMENT, "FORM", &[]);
        page.text(form, "Enter the verification code");
        let digit = page.node(
            Some(form),
            ELEMENT,
            "INPUT",
            &[
                ("maxlength", " 1 "),
                ("inputmode", "numeric"),
                ("aria-label", "digit 1"),
            ],
        );
        // Chromium keeps the typed value in the input's user-agent shadow root.
        let inner = page.shadow_root(digit, "user-agent");
        let editor = page.node(Some(inner), ELEMENT, "DIV", &[]);
        page.text(editor, "7");
        let card = page.node(
            Some(form),
            ELEMENT,
            "input",
            &[("autocomplete", "cc-number")],
        );
        let outside = page.node(Some(document), ELEMENT, "TEXTAREA", &[("id", "notes")]);

        let fields = page.fields();
        let Some(digit) = fields.get(&backend(digit)) else {
            panic!("the digit box was not read");
        };
        assert_eq!(digit.max_length, Some(1));
        assert_eq!(digit.input_mode.as_deref(), Some("numeric"));
        assert_eq!(
            digit.form_text.as_deref(),
            Some("Enter the verification code")
        );
        assert_eq!(
            FieldRules::get().classify(digit, "digit 1"),
            Some(SensitiveKind::OneTimeCode)
        );
        let Some(card) = fields.get(&backend(card)) else {
            panic!("the card field was not read");
        };
        assert_eq!(card.tag, FieldTag::Input);
        assert_eq!(card.autocomplete.as_deref(), Some("cc-number"));
        let Some(notes) = fields.get(&backend(outside)) else {
            panic!("the textarea was not read");
        };
        assert_eq!(notes.tag, FieldTag::Textarea);
        assert_eq!(notes.form_text, None);
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn a_shadow_root_is_its_own_scope_for_form_text() {
        let (mut page, document) = Capture::new();
        let form = page.node(Some(document), ELEMENT, "form", &[]);
        page.text(form, "Sign in ");
        let host = page.node(Some(form), ELEMENT, "DIV", &[]);
        let root = page.shadow_root(host, "closed");
        page.text(root, "verification code");
        let inside = page.node(Some(root), ELEMENT, "INPUT", &[("type", "password")]);
        let light = page.node(Some(form), ELEMENT, "INPUT", &[]);

        let fields = page.fields();
        assert_eq!(
            fields
                .get(&backend(inside))
                .and_then(|f| f.form_text.clone()),
            None
        );
        assert_eq!(
            fields
                .get(&backend(light))
                .and_then(|f| f.form_text.clone())
                .as_deref(),
            Some("Sign in ")
        );
    }
}
