use std::{fmt, mem};

use serde::de::value::MapAccessDeserializer;
use serde::de::{self, MapAccess, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Number;

/// One node of a page's aria snapshot: Playwright's `AriaNodeJSON` at 1.64.1 (vendored
/// `isomorphic/ariaSnapshot.ts`) plus Neo's two marks, [`AriaNode::inert`] and
/// [`AriaNode::on_top`]. Today the page's walk writes this shape; any producer that builds it can
/// feed the crate.
///
/// The serde spellings, the field order and the skip rules are the JSON's own, so a tree read and
/// written again is the same JSON. Each field takes exactly the values Playwright's types allow:
/// an unknown field, a `null` or a value outside a field's type fails the decode, so nothing
/// downstream has to decide what to print for a value no producer writes. The errors of serde's
/// derived parts quote the value they could not read, which may be page text, so a reader of
/// untrusted text reports a fixed message of its own instead.
///
/// Depth: the derived decode, encode, clone, comparison and `Debug` recurse once per level, so
/// they rely on the producer keeping a tree to [`MAX_TREE_DEPTH`] levels. The drop does not
/// recurse (see its `Drop`), and code that walks a tree for its text keeps its own stack, as the
/// TypeScript renderer does, so a deeper tree built in code can be printed and dropped.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AriaNode {
    /// An ARIA role from Playwright's own list, or `iframe`, or `text`: a string at the top of the
    /// tree, held in [`AriaNode::text`].
    pub role: String,
    /// The accessible name, page text. Empty prints no name, as the TypeScript's truthiness test
    /// skips it.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A checkbox's, radio's or switch's state.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<Tristate>,
    /// The element is disabled, natively or by `aria-disabled` on it or an ancestor.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// The element is expanded (`aria-expanded`, or an open native control).
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// The element has the focus.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    /// Neo's mark: the top-most node of a subtree the browser made inert, which nothing inside can
    /// be clicked or typed into.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inert: Option<bool>,
    /// Neo's mark: the ref of the whole-view layer this node is the top-most content of.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_top: Option<String>,
    /// `aria-invalid`: whether the value is invalid, and how.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<Invalid>,
    /// A heading's or tree item's level (`h1` to `h6`, or `aria-level`). Any JSON number. Its value
    /// and whether it was an integer are kept, not its spelling: `1e+21` from the page is written
    /// back as `1e21`, which a script reads as the same number. 0 prints no level.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Number>,
    /// A toggle button's state.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<Tristate>,
    /// An option, a tab or a row is selected.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    /// `aria-hidden="true"` was on the element.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aria_hidden: Option<bool>,
    /// The ref a script acts on the element by (`e5`, `f3e5`). Empty counts as present where the
    /// TypeScript asks `ref !== undefined`, and prints no `[ref=]` where it asks `if (node.ref)`.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    /// `pointer` on a node with a ref whose element takes clicks.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// The element's rounded client rect, written only when a snapshot asks for boxes.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#box: Option<NodeBox>,
    /// A link's raw `href`, page text: not normalized, and it may hold newlines.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A text box's placeholder where it differs from the name, page text.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// The node's only content when that is one string, page text.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Strings and nodes in document order. Empty is written as no `children` key, as Playwright
    /// writes it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AriaChild>,
}

/// The deepest tree, in levels, that every producer of [`AriaNode`] keeps to: a top-level node is
/// level 1. Decoding, writing, cloning and comparing a tree recurse once per level and rely on it.
///
/// It is the bound that always holds, not the reader's exact rule. neo-playwright refuses a probe
/// reply, and a frame stitched under its iframe, past 297 levels of JSON (its `MAX_VALUE_DEPTH`,
/// what Chromium returns of a page's value). After the reply's object and its array, a level is a
/// node and its `children` array, and the deepest node may hold a `box`: 147 levels fit whatever
/// the nodes hold. A 148th fits only when its nodes hold neither a box nor any child, text
/// included, since a `children` array of strings is a level of JSON too. neo-playwright checks
/// at compile time that its bound admits exactly this many.
///
/// A producer that builds the tree without a reply in between, and must refuse the pages the
/// reader refuses, measures the levels of JSON its tree would have, as the reader does.
pub const MAX_TREE_DEPTH: usize = 147;

/// One entry of a node's `children`: a run of text or a node, in document order.
///
/// Decoding is written by hand. serde's `untagged` derive buffers each child into its private
/// `Content` before it tries the variants, and when the deserializer it reads from is itself such
/// buffered content it clones the whole subtree first, at every level, so the cost grows with
/// nodes times depth. On the largest tree measured (about 50,000 nodes, a release build), the step
/// into typed nodes took 0.5 to 0.6 s of a 0.6 to 0.7 s read. The visitor here reads a string as
/// [`AriaChild::Text`] and an object as [`AriaChild::Node`] straight from the deserializer it is
/// given, and refuses every other value without echoing it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AriaChild {
    /// Text between elements, page text.
    Text(String),
    /// An element's node.
    Node(Box<AriaNode>),
}

/// `checked` and `pressed`: `true`, `false` or `"mixed"`. Playwright's renderer prints
/// `[checked]` for `true`, `[checked=mixed]` for `"mixed"` and nothing for `false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tristate {
    /// JSON `false`.
    False,
    /// JSON `true`.
    True,
    /// JSON `"mixed"`: partly checked or pressed.
    Mixed,
}

/// `invalid`: `true`, `false`, `"grammar"` or `"spelling"`. Playwright's renderer prints
/// `[invalid]` for `true` and `[invalid=grammar]` or `[invalid=spelling]` for a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// JSON `false`.
    False,
    /// JSON `true`.
    True,
    /// JSON `"grammar"`.
    Grammar,
    /// JSON `"spelling"`.
    Spelling,
}

/// `box`: the element's client rect in its own frame's viewport, each number rounded. Playwright's
/// renderer prints it as `[box=x,y,width,height]`. Each number's value and whether it was an
/// integer are kept, not its spelling: `1e+21` is written back as `1e21`, the same number to a
/// script.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeBox {
    /// The left edge, in CSS pixels.
    pub x: Number,
    /// The top edge, in CSS pixels.
    pub y: Number,
    /// The width, in CSS pixels.
    pub width: Number,
    /// The height, in CSS pixels.
    pub height: Number,
}

/// Drops a tree with a stack of its own: each node's children move onto one list before the node
/// goes, so no node is dropped while it still holds a child. The derived drop recursed once per
/// level, which a tree deeper than [`MAX_TREE_DEPTH`] (a case built in code, or a producer's bug)
/// could take past the thread's stack, ending the whole process rather than one call.
impl Drop for AriaNode {
    fn drop(&mut self) {
        let mut pending = mem::take(&mut self.children);
        while let Some(child) = pending.pop() {
            if let AriaChild::Node(mut node) = child {
                pending.append(&mut node.children);
            }
        }
    }
}

/// An optional field that is there, read as the field's own type, so a `null` is refused like any
/// other value outside it. serde reads `null` as `None` for an `Option`, and no field of
/// `AriaNodeJSON` takes `null`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for AriaChild {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ChildVisitor)
    }
}

/// Reads one child straight from the deserializer it is given (see [`AriaChild`]). Values it does
/// not take fall to serde's defaults, which name the kind of value and quote none of the strings,
/// since every string is taken.
struct ChildVisitor;

impl<'de> Visitor<'de> for ChildVisitor {
    type Value = AriaChild;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string or a node")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<AriaChild, E> {
        Ok(AriaChild::Text(text.to_owned()))
    }

    fn visit_string<E: de::Error>(self, text: String) -> Result<AriaChild, E> {
        Ok(AriaChild::Text(text))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<AriaChild, A::Error> {
        let node = AriaNode::deserialize(MapAccessDeserializer::new(map))?;
        Ok(AriaChild::Node(Box::new(node)))
    }
}

/// Reads a field whose JSON is `true`, `false` or one of a few words, as Playwright's types spell
/// `checked`, `pressed` and `invalid`. A string that is none of the words is refused without
/// quoting it, since it may be page text; other values fall to serde's defaults, which quote no
/// string.
struct Literals<T: 'static> {
    when_false: T,
    when_true: T,
    /// Each word the field takes, with what it reads as.
    words: &'static [(&'static str, T)],
    /// What the field takes, as the refusal states it.
    expected: &'static str,
}

impl<T: Copy + 'static> Visitor<'_> for Literals<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expected)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<T, E> {
        Ok(if value {
            self.when_true
        } else {
            self.when_false
        })
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<T, E> {
        self.words
            .iter()
            .find_map(|&(word, read)| (word == value).then_some(read))
            .ok_or_else(|| E::invalid_value(Unexpected::Other("another string"), &self))
    }
}

impl<'de> Deserialize<'de> for Tristate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Literals {
            when_false: Self::False,
            when_true: Self::True,
            words: &[("mixed", Self::Mixed)],
            expected: "true, false or \"mixed\"",
        })
    }
}

impl Serialize for Tristate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::False => serializer.serialize_bool(false),
            Self::True => serializer.serialize_bool(true),
            Self::Mixed => serializer.serialize_str("mixed"),
        }
    }
}

impl<'de> Deserialize<'de> for Invalid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Literals {
            when_false: Self::False,
            when_true: Self::True,
            words: &[("grammar", Self::Grammar), ("spelling", Self::Spelling)],
            expected: "true, false, \"grammar\" or \"spelling\"",
        })
    }
}

impl Serialize for Invalid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::False => serializer.serialize_bool(false),
            Self::True => serializer.serialize_bool(true),
            Self::Grammar => serializer.serialize_str("grammar"),
            Self::Spelling => serializer.serialize_str("spelling"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::Deserialize;
    use serde_json::{Number, Value, json};

    use super::{AriaChild, AriaNode, Invalid, Tristate};

    /// Four frames of one page and the tree upstream's server stitched from them, recorded from
    /// playwright-core 1.64.1 by neo-playwright's capture script.
    const RECORDED: &str = include_str!("../../neo-playwright/tests/data/snapshot-frames.json");

    #[derive(Deserialize)]
    struct Recorded {
        frames: Vec<RecordedFrame>,
        stitched: Vec<AriaNode>,
    }

    #[derive(Deserialize)]
    struct RecordedFrame {
        json: Vec<AriaNode>,
    }

    /// A reply's buffered form: serde's internally tagged enums read the whole object into their
    /// private `Content` before they pick a variant, and a reader of a tagged reply hands the tree
    /// over from there.
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    enum Tagged {
        Snapshot { json: Vec<AriaNode> },
    }

    /// `node`, read from its JSON text and written again.
    fn read_and_write(node: &Value) -> Result<Value, Box<dyn Error>> {
        let read: AriaNode = serde_json::from_str(&node.to_string())?;
        Ok(serde_json::to_value(&read)?)
    }

    #[test]
    fn tests_that_every_value_upstreams_types_allow_reads_and_writes_the_same_json()
    -> Result<(), Box<dyn Error>> {
        for node in [
            json!({ "role": "checkbox", "checked": true }),
            json!({ "role": "checkbox", "checked": "mixed" }),
            json!({ "role": "checkbox", "checked": false }),
            json!({ "role": "button", "pressed": true }),
            json!({ "role": "button", "pressed": "mixed" }),
            json!({ "role": "button", "pressed": false }),
            json!({ "role": "textbox", "invalid": true }),
            json!({ "role": "textbox", "invalid": false }),
            json!({ "role": "textbox", "invalid": "grammar" }),
            json!({ "role": "textbox", "invalid": "spelling" }),
            json!({ "role": "heading", "level": 0 }),
            json!({ "role": "heading", "level": 2 }),
            json!({ "role": "heading", "level": 2.5 }),
            json!({ "role": "heading", "level": 1e21 }),
            json!({ "role": "button", "box": { "x": 12, "y": -3, "width": 80.5, "height": 0 } }),
            json!({
                "role": "link", "name": "", "disabled": false, "expanded": true, "active": true,
                "inert": true, "onTop": "e9", "selected": true, "ariaHidden": true, "ref": "",
                "cursor": "pointer", "url": "/a?b=1\n", "placeholder": "", "text": "",
            }),
        ] {
            assert_eq!(
                read_and_write(&node)?,
                node,
                "a value upstream's types allow is written back as it was read"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_the_vocabulary_fields_read_as_their_words() -> Result<(), Box<dyn Error>> {
        let node: AriaNode = serde_json::from_str(
            r#"{"role":"switch","checked":"mixed","pressed":false,"invalid":"spelling","level":3,
                "box":{"x":1,"y":2,"width":3,"height":4}}"#,
        )?;
        assert_eq!(
            (node.checked, node.pressed, node.invalid),
            (
                Some(Tristate::Mixed),
                Some(Tristate::False),
                Some(Invalid::Spelling)
            )
        );
        assert_eq!(node.level.as_ref().and_then(Number::as_u64), Some(3));
        let node_box = node.r#box.as_ref().ok_or("the box is read")?;
        assert_eq!(
            [&node_box.x, &node_box.y, &node_box.width, &node_box.height].map(Number::as_u64),
            [Some(1), Some(2), Some(3), Some(4)]
        );
        for (checked, expected) in [
            ("true", Tristate::True),
            ("false", Tristate::False),
            ("\"mixed\"", Tristate::Mixed),
        ] {
            let node: AriaNode =
                serde_json::from_str(&format!(r#"{{"role":"checkbox","checked":{checked}}}"#))?;
            assert_eq!(node.checked, Some(expected), "checked: {checked}");
        }
        for (invalid, expected) in [
            ("true", Invalid::True),
            ("false", Invalid::False),
            ("\"grammar\"", Invalid::Grammar),
        ] {
            let node: AriaNode =
                serde_json::from_str(&format!(r#"{{"role":"textbox","invalid":{invalid}}}"#))?;
            assert_eq!(node.invalid, Some(expected), "invalid: {invalid}");
        }
        Ok(())
    }

    #[test]
    fn tests_that_a_value_outside_upstreams_types_is_refused() {
        for node in [
            json!({ "role": "checkbox", "checked": "yes" }),
            json!({ "role": "checkbox", "checked": 1 }),
            json!({ "role": "checkbox", "checked": null }),
            json!({ "role": "checkbox", "checked": "Mixed" }),
            json!({ "role": "button", "pressed": "true" }),
            json!({ "role": "textbox", "invalid": "other" }),
            json!({ "role": "textbox", "invalid": 0 }),
            json!({ "role": "heading", "level": "2" }),
            json!({ "role": "heading", "level": null }),
            json!({ "role": "button", "box": { "x": 1, "y": 2, "width": 3 } }),
            json!({ "role": "button", "box": { "x": 1, "y": 2, "width": 3, "height": 4, "z": 5 } }),
            json!({ "role": "button", "box": { "x": "1", "y": 2, "width": 3, "height": 4 } }),
            json!({ "role": "button", "name": null }),
            json!({ "role": "button", "disabled": "true" }),
            json!({ "role": "button", "ref": 5 }),
            json!({ "role": "list", "children": [5] }),
            json!({ "role": "list", "children": [true] }),
            json!({ "role": "list", "children": [["a list"]] }),
            json!({ "role": "list", "children": null }),
            json!({ "role": "list", "children": [{ "name": "a node without a role" }] }),
            json!({ "name": "no role" }),
        ] {
            assert!(
                serde_json::from_str::<AriaNode>(&node.to_string()).is_err(),
                "{node} is not a node upstream's types allow"
            );
        }
    }

    #[test]
    fn tests_that_a_field_upstream_does_not_write_fails_to_decode() {
        let unknown = serde_json::from_value::<AriaNode>(json!({
            "role": "button",
            "description": "a field 1.64.1 does not have",
        }));
        assert!(
            unknown.is_err(),
            "a field the stitch has not been checked against fails"
        );
    }

    #[test]
    fn tests_that_a_refusal_never_quotes_the_value_it_refused() -> Result<(), Box<dyn Error>> {
        for node in [
            json!({ "role": "checkbox", "checked": "PAGE-SECRET" }),
            json!({ "role": "button", "pressed": "PAGE-SECRET" }),
            json!({ "role": "textbox", "invalid": "PAGE-SECRET" }),
            json!({ "role": "list", "children": [["PAGE-SECRET"]] }),
            json!({
                "role": "list",
                "children": [{ "role": "PAGE-SECRET", "checked": "PAGE-SECRET" }],
            }),
        ] {
            let Err(error) = serde_json::from_str::<AriaNode>(&node.to_string()) else {
                return Err(format!("{node} was read").into());
            };
            assert!(
                !error.to_string().contains("PAGE-SECRET"),
                "a value outside a field's type may be page text, so no refusal quotes it: {error}"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_children_read_as_text_and_nodes_on_every_path() -> Result<(), Box<dyn Error>> {
        // An escaped string and an unescaped one take different paths through the reader.
        let text = json!({
            "role": "paragraph",
            "children": [
                "Read ",
                { "role": "link", "name": "the terms", "children": ["now \"here\"\u{a0}"] },
                "plain",
                { "role": "strong", "text": "!" },
            ],
        })
        .to_string();
        let bare: AriaNode = serde_json::from_str(r#"{"role":"paragraph"}"#)?;
        let direct: AriaNode = serde_json::from_str(&text)?;
        let AriaNode { children, .. } = &direct;
        let [
            AriaChild::Text(first),
            AriaChild::Node(link),
            AriaChild::Text(plain),
            AriaChild::Node(strong),
        ] = children.as_slice()
        else {
            return Err(format!("four children, text and nodes in order: {children:?}").into());
        };
        assert_eq!((first.as_str(), plain.as_str()), ("Read ", "plain"));
        assert_eq!(link.name.as_deref(), Some("the terms"));
        assert_eq!(
            link.children,
            [AriaChild::Text("now \"here\"\u{a0}".to_owned())]
        );
        assert_eq!(strong.text.as_deref(), Some("!"));
        let mut without_children = direct.clone();
        without_children.children.clear();
        assert_eq!(without_children, bare, "nothing but the children was read");
        // The same text through a `Value`, and through a tagged reply's buffer, reads the same.
        let from_value: AriaNode = serde_json::from_value(serde_json::from_str(&text)?)?;
        assert_eq!(from_value, direct, "read from a serde_json::Value");
        let Tagged::Snapshot { json } =
            serde_json::from_str(&format!(r#"{{"kind":"snapshot","json":[{text}]}}"#))?;
        assert_eq!(
            json,
            [direct],
            "read from a tagged reply's buffered content"
        );
        Ok(())
    }

    /// The derived drop recursed once per level, and a stack overflow ends the whole process.
    /// A hundred thousand levels is far past a test thread's stack at a few frames a level.
    #[test]
    fn tests_that_a_tree_deeper_than_any_stack_drops() -> Result<(), Box<dyn Error>> {
        let link: AriaNode = serde_json::from_str(r#"{"role":"generic","name":"level"}"#)?;
        let mut tree: AriaNode = serde_json::from_str(r#"{"role":"button","name":"Deep"}"#)?;
        for _ in 1..100_000 {
            let mut parent = link.clone();
            parent.children.push(AriaChild::Node(Box::new(tree)));
            tree = parent;
        }
        drop(tree);
        Ok(())
    }

    #[test]
    fn tests_that_recorded_frames_read_and_write_the_same_json() -> Result<(), Box<dyn Error>> {
        let recorded: Recorded = serde_json::from_str(RECORDED)?;
        let raw: Value = serde_json::from_str(RECORDED)?;
        assert_eq!(recorded.frames.len(), 4, "the fixture's four frames");
        for (index, frame) in recorded.frames.iter().enumerate() {
            assert_eq!(
                serde_json::to_value(&frame.json)?,
                raw["frames"][index]["json"],
                "frame {index} is written back as it was recorded"
            );
        }
        assert_eq!(
            serde_json::to_value(&recorded.stitched)?,
            raw["stitched"],
            "the stitched tree is written back as it was recorded"
        );
        Ok(())
    }
}
