//! A page for `ChunkRun`'s tests: one document of elements, some with a node in the test's APC
//! tree and some plain, as a capture leaves out a `div` with no role and no id. It answers
//! `stage` with upstream's `querySelectorAll` loop (`injectedScript.ts:376-427`) for the parts
//! the tests send (the three call-scoped engines, `internal:attr`, an `xpath` of `..`, `nth`,
//! `internal:has`, `internal:has-not`, `internal:and`, `internal:or`, `internal:chain` and
//! `internal:describe`), with `sortInDOMOrder` and `isInsideScope` as upstream has them, and the
//! probe's answers to error and stale-ref sets.
//! `describePins`, `hydrate` and `keys` answer as the probe (`stage.ts`) and engine chunk 10's
//! hydration do. A page stage that carries a tree part fails the test: the tree's parts are
//! Rust's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error;
use std::fmt::Write as _;
use std::future;

use neo_snapshot_apc::select::{
    AttributeSelectorOperator, AttributeValue, NestedSelectorBody, ParsedSelector, SelectorBody,
    SelectorPart, UnquotedStrings, parse_attribute_selector,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocToken, DomNodeId, NodeBuilder, NodeId, NodeKey, SnapshotTree,
};
use serde_json::Value;

use super::ChunkPage;
use crate::probe::{
    self, ErrorClass, PinPreview, ProbeReply, ProbeRequest, SetMember, StageMatch, StageSet,
};
use crate::world::Pin;

const MAIN: &str = "doc-main";

/// One element of a test page, before it is built.
pub(super) struct El {
    dom: i32,
    /// The element's APC node, without its element children; `None` for a plain element.
    node: Option<NodeBuilder>,
    attrs: Vec<(String, String)>,
    preview: String,
    aka: String,
    kids: Vec<El>,
}

/// An element the capture has a node for. `node` carries the facts the tree reads (tag, role,
/// text); its `dom_id` is set here.
pub(super) fn tracked(node: NodeBuilder, dom: i32) -> El {
    El {
        dom,
        node: Some(node.dom_id(dom)),
        attrs: Vec::new(),
        preview: String::new(),
        aka: String::new(),
        kids: Vec::new(),
    }
}

/// An element the capture has no node for.
pub(super) fn plain(dom: i32) -> El {
    El {
        dom,
        node: None,
        attrs: Vec::new(),
        preview: String::new(),
        aka: String::new(),
        kids: Vec::new(),
    }
}

impl El {
    pub(super) fn attr(mut self, name: &str, value: &str) -> Self {
        self.attrs.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(super) fn kids(mut self, kids: Vec<El>) -> Self {
        self.kids = kids;
        self
    }

    /// The page's words for the element: upstream's `previewNode`, and the locator its
    /// `generateSelectorSimple` writes, as `asLocator` prints it.
    pub(super) fn words(mut self, preview: &str, aka: &str) -> Self {
        self.preview = preview.to_owned();
        self.aka = aka.to_owned();
        self
    }
}

/// A node of the page: the Document, or an element by its index in document order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Document,
    Element(usize),
}

struct Element {
    dom: i32,
    /// The parent element; `None` for `<body>`, whose parent the tests never reach.
    parent: Option<usize>,
    /// One past the last element of its subtree, in document order.
    end: usize,
    attrs: Vec<(String, String)>,
    preview: String,
    aka: String,
    connected: bool,
    /// The node is gone for good: `DOM.resolveNode` fails for it, so the seam cannot hand it over.
    destroyed: bool,
}

/// What the run asked the page, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Sent {
    Stage {
        /// The selector the stage ran, as JSON.
        selector: Value,
        from: Option<Vec<SetMember>>,
        sets: Vec<StageSet>,
        strict: bool,
        /// The tree nodes the call handed over, by their DOM ids.
        handed: Vec<i32>,
    },
    Describe {
        members: Vec<SetMember>,
        handed: Vec<i32>,
    },
    /// The node pinned, by its DOM id.
    Hydrate(i32),
    Keys(Vec<Pin>),
}

/// One candidate set as the page holds it during a call.
enum Candidates {
    Members(Vec<Node>),
    Error { message: String, class: ErrorClass },
    StaleRef(String),
}

/// What one `stage` call registers: `internal:neo-from`'s elements and the candidate sets, and
/// the stale-ref bodies the call's roots reached, in the order the engines were called.
struct Scoped {
    from: Vec<Node>,
    sets: Vec<Candidates>,
    reached: RefCell<Vec<String>>,
}

/// An error thrown in the page, classified as the probe classifies it.
struct Thrown {
    message: String,
    class: ErrorClass,
}

pub(super) struct FakePage {
    /// Every element, in document order.
    elements: Vec<Element>,
    /// The element each tree node is, for the nodes a call hands over and for `hydrate`.
    by_node: HashMap<NodeId, usize>,
    /// Pin `n` names `pins[n - 1]`, as the probe never reuses a number.
    pins: Vec<usize>,
    /// The root every request is sent from: the Document, or an element handle's element.
    root: Node,
    /// The chunk a strict `stage` or a `describePins` must name, and the locator text the page
    /// prints for it.
    chunk: Option<(Value, String)>,
    /// `(n, dom)`: the element leaves the document once the run's `n`-th request returns.
    removals: Vec<(usize, i32)>,
    pub(super) sent: Vec<Sent>,
}

/// Builds the test's tree and its page from the same elements, under one `<body>`: a plain
/// element, as in a capture whose `<body>` is a pruned container.
pub(super) fn build(body: Vec<El>) -> Result<(SnapshotTree, FakePage), Box<dyn Error>> {
    let mut elements = Vec::new();
    let nodes = flatten(vec![plain(1).kids(body)], None, &mut elements);
    let apc = ApcBuilder::new(MAIN)
        .root(NodeBuilder::root().children(nodes))
        .build();
    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    let mut by_node = HashMap::new();
    for (index, element) in elements.iter().enumerate() {
        if let Some(node) = tree.node_by_key(&key(element.dom)?) {
            by_node.insert(node.id(), index);
        }
    }
    Ok((
        tree,
        FakePage {
            elements,
            by_node,
            pins: Vec::new(),
            root: Node::Document,
            chunk: None,
            removals: Vec::new(),
            sent: Vec::new(),
        },
    ))
}

/// The key of an element of the main document.
pub(super) fn key(dom: i32) -> Result<NodeKey, Box<dyn Error>> {
    let token = DocToken::new(MAIN).ok_or("a test document token is never empty")?;
    Ok(NodeKey::new(token, DomNodeId::new(dom)))
}

/// Pushes `els` in document order and returns their APC nodes, a plain element's children in its
/// place.
fn flatten(els: Vec<El>, parent: Option<usize>, elements: &mut Vec<Element>) -> Vec<NodeBuilder> {
    let mut nodes = Vec::new();
    for el in els {
        let El {
            dom,
            node,
            attrs,
            preview,
            aka,
            kids,
        } = el;
        let index = elements.len();
        elements.push(Element {
            dom,
            parent,
            end: index + 1,
            attrs,
            preview,
            aka,
            connected: true,
            destroyed: false,
        });
        let kids = flatten(kids, Some(index), elements);
        let end = elements.len();
        elements[index].end = end;
        match node {
            Some(node) => nodes.push(node.children(kids)),
            None => nodes.extend(kids),
        }
    }
    nodes
}

impl FakePage {
    /// Sends every request from the element `dom`, as an element handle's query does.
    pub(super) fn root_at(&mut self, dom: i32) {
        self.root = Node::Element(self.index(dom));
    }

    /// The chunk strict mode names, as JSON, and the locator text the page prints for it.
    pub(super) fn expect_chunk(&mut self, chunk: Value, locator: &str) {
        self.chunk = Some((chunk, locator.to_owned()));
    }

    /// Removes the element `dom`, its subtree with it, from the document now.
    pub(super) fn remove(&mut self, dom: i32) {
        let index = self.index(dom);
        let end = self.elements[index].end;
        for element in &mut self.elements[index..end] {
            element.connected = false;
        }
    }

    /// Destroys the element `dom`: it leaves the document and no longer resolves by its key.
    pub(super) fn destroy(&mut self, dom: i32) {
        self.remove(dom);
        let index = self.index(dom);
        self.elements[index].destroyed = true;
    }

    /// Removes the element `dom` once the run's `n`-th request (counting from 1) has returned.
    pub(super) fn remove_after(&mut self, n: usize, dom: i32) {
        self.removals.push((n, dom));
    }

    /// The DOM id of the element `pin` names.
    pub(super) fn dom_of(&self, pin: Pin) -> i32 {
        let index = self.pins[usize::try_from(pin.0).unwrap_or(usize::MAX) - 1];
        self.elements[index].dom
    }

    /// The DOM ids of a page stage's matches; the Document reads as 0.
    pub(super) fn doms_of(&self, matches: &[StageMatch]) -> Vec<i32> {
        matches
            .iter()
            .map(|member| match member {
                StageMatch::Pin(pin) => self.dom_of(*pin),
                StageMatch::Document => 0,
            })
            .collect()
    }

    /// How many `stage` requests the run sent.
    pub(super) fn stages(&self) -> usize {
        self.sent
            .iter()
            .filter(|sent| matches!(sent, Sent::Stage { .. }))
            .count()
    }

    fn index(&self, dom: i32) -> usize {
        self.elements
            .iter()
            .position(|element| element.dom == dom)
            .unwrap_or_else(|| panic!("the page has no element {dom}"))
    }

    fn record(&mut self, sent: Sent) {
        self.sent.push(sent);
        let count = self.sent.len();
        let due: Vec<i32> = self
            .removals
            .iter()
            .filter(|(n, _)| *n == count)
            .map(|(_, dom)| *dom)
            .collect();
        for dom in due {
            self.remove(dom);
        }
    }

    fn pin(&mut self, index: usize) -> Pin {
        self.pins.push(index);
        Pin(u32::try_from(self.pins.len()).unwrap_or(u32::MAX))
    }

    /// A member while it is live, as the probe's `held` reads it: a pin's element or a handed
    /// node while it is connected, or the Document. `handed` holds the call's nodes, as element
    /// indexes.
    fn held(&self, member: SetMember, handed: &[usize]) -> Option<Node> {
        let index = match member {
            SetMember::Document => return Some(Node::Document),
            SetMember::Pin(pin) => *self
                .pins
                .get(usize::try_from(pin.0).ok()?.checked_sub(1)?)?,
            SetMember::Node(node) => *handed
                .get(usize::try_from(node).ok()?)
                .unwrap_or_else(|| panic!("call node {node} was not handed over")),
        };
        self.elements[index]
            .connected
            .then_some(Node::Element(index))
    }

    /// The elements of the nodes a call hands over: `None` when one no longer resolves.
    fn resolve(&self, nodes: &[NodeId]) -> Option<Vec<usize>> {
        nodes
            .iter()
            .map(|node| {
                let index = *self
                    .by_node
                    .get(node)
                    .unwrap_or_else(|| panic!("{node:?} is not an element of the page"));
                (!self.elements[index].destroyed).then_some(index)
            })
            .collect()
    }

    fn doms(&self, nodes: &[NodeId]) -> Vec<i32> {
        nodes
            .iter()
            .filter_map(|node| self.by_node.get(node))
            .map(|&index| self.elements[index].dom)
            .collect()
    }

    fn stage(
        &mut self,
        selector: &probe::ParsedSelector,
        from: Option<&[SetMember]>,
        sets: &[StageSet],
        strict: Option<&probe::ParsedSelector>,
        handed: &[usize],
    ) -> ProbeReply {
        let json = serde_json::to_value(selector).unwrap_or(Value::Null);
        let mut parsed: ParsedSelector = match serde_json::from_value(json) {
            Ok(parsed) => parsed,
            Err(error) => return internal(&format!("not a parsed selector: {error}")),
        };
        let mut starts = Vec::new();
        if let Some(from) = from {
            for member in from {
                match self.held(*member, handed) {
                    Some(node) => starts.push(node),
                    None => return ProbeReply::Detached,
                }
            }
            // As the probe does: one prepended part answers `from`, whatever root reaches it.
            parsed.parts.insert(
                0,
                SelectorPart {
                    name: "internal:neo-from".to_owned(),
                    body: SelectorBody::Text(String::new()),
                    source: String::new(),
                },
            );
            parsed.capture = parsed.capture.map(|capture| capture + 1);
        }
        let scoped = Scoped {
            from: starts,
            sets: sets
                .iter()
                .map(|set| match set {
                    StageSet::Members(members) => Candidates::Members(
                        members
                            .iter()
                            .filter_map(|member| self.held(*member, handed))
                            .collect(),
                    ),
                    StageSet::Error { message, class } => Candidates::Error {
                        message: message.clone(),
                        class: *class,
                    },
                    StageSet::StaleRef(body) => Candidates::StaleRef(body.clone()),
                })
                .collect(),
            reached: RefCell::new(Vec::new()),
        };
        let matches = match self.query_all(&parsed, self.root, &scoped) {
            Ok(matches) => matches,
            Err(Thrown { message, class }) => {
                return ProbeReply::Error {
                    message,
                    class,
                    tree_depth: None,
                    resolved: None,
                };
            }
        };
        if let Some(strict) = strict
            && matches.len() > 1
        {
            let locator = self.locator_for(strict);
            return ProbeReply::Strict {
                message: self.violation(&locator, &matches),
                listing: None,
            };
        }
        let matches = matches
            .into_iter()
            .map(|node| match node {
                Node::Document => StageMatch::Document,
                Node::Element(index) => StageMatch::Pin(self.pin(index)),
            })
            .collect();
        ProbeReply::Staged {
            matches,
            stale_ref: scoped.reached.into_inner().into_iter().next(),
        }
    }

    /// The locator text for the chunk the test expects strict mode to name; any other chunk fails
    /// the test.
    fn locator_for(&self, chunk: &probe::ParsedSelector) -> String {
        let (expected, locator) = self
            .chunk
            .as_ref()
            .unwrap_or_else(|| panic!("the test names no chunk for strict mode"));
        let sent = serde_json::to_value(chunk).unwrap_or(Value::Null);
        assert_eq!(
            &sent, expected,
            "strict mode names the chunk as the prelude sent it, never a page form"
        );
        locator.clone()
    }

    /// Upstream's `strictModeViolationError` text (`injectedScript.ts:1309-1333`).
    fn violation(&self, locator: &str, matches: &[Node]) -> String {
        let mut message = format!(
            "strict mode violation: {locator} resolved to {} elements:",
            matches.len()
        );
        for (index, node) in matches.iter().take(10).enumerate() {
            let Node::Element(element) = node else {
                panic!("the tests keep the Document out of strict mode");
            };
            let element = &self.elements[*element];
            let _ = write!(
                message,
                "\n    {}) {} aka {}",
                index + 1,
                element.preview,
                element.aka
            );
        }
        if matches.len() > 10 {
            message.push_str("\n    ...");
        }
        message.push('\n');
        message
    }

    fn describe(
        &mut self,
        members: &[SetMember],
        chunk: Option<&probe::ParsedSelector>,
        handed: &[usize],
    ) -> ProbeReply {
        let mut items = Vec::new();
        for member in members {
            match self.held(*member, handed) {
                Some(Node::Element(index)) => {
                    let element = &self.elements[index];
                    items.push(PinPreview {
                        preview: element.preview.clone(),
                        aka: element.aka.clone(),
                    });
                }
                Some(Node::Document) => {
                    return internal("describePins describes elements, never the document");
                }
                None => return ProbeReply::Detached,
            }
        }
        ProbeReply::PinsDescribed {
            locator: chunk.map(|chunk| self.locator_for(chunk)),
            items,
        }
    }

    /// `querySelectorAll(selector, root)` (`injectedScript.ts:376-427`): the capture rewrite,
    /// then the parts folded over a duplicate-free, insertion-ordered set of roots.
    fn query_all(
        &self,
        selector: &ParsedSelector,
        root: Node,
        scoped: &Scoped,
    ) -> Result<Vec<Node>, Thrown> {
        if let Some(capture) = selector.capture {
            if selector.parts.iter().any(|part| part.name == "nth") {
                return Err(script(
                    "Can't query n-th element in a request with the capture.",
                ));
            }
            let mut parts: Vec<SelectorPart> =
                selector.parts.iter().take(capture + 1).cloned().collect();
            let rest: Vec<SelectorPart> =
                selector.parts.iter().skip(capture + 1).cloned().collect();
            if !rest.is_empty() {
                parts.push(SelectorPart {
                    name: "internal:has".to_owned(),
                    body: SelectorBody::Nested(NestedSelectorBody {
                        parsed: ParsedSelector {
                            parts: rest,
                            capture: None,
                        },
                        distance: None,
                    }),
                    source: String::new(),
                });
            }
            let rewritten = ParsedSelector {
                parts,
                capture: None,
            };
            return self.query_all(&rewritten, root, scoped);
        }
        let mut roots = vec![root];
        for part in &selector.parts {
            roots = match (part.name.as_str(), &part.body) {
                ("nth", SelectorBody::Text(body)) => nth(&roots, body),
                ("internal:and", SelectorBody::Nested(nested)) => self
                    .query_all(&nested.parsed, root, scoped)?
                    .into_iter()
                    .filter(|node| roots.contains(node))
                    .collect(),
                ("internal:or", SelectorBody::Nested(nested)) => {
                    let mut union = roots;
                    for node in self.query_all(&nested.parsed, root, scoped)? {
                        if !union.contains(&node) {
                            union.push(node);
                        }
                    }
                    sort_in_dom_order(union)
                }
                _ => {
                    let mut next = Vec::new();
                    for &scope in &roots {
                        for node in self.engine(part, scope, scoped)? {
                            if !next.contains(&node) {
                                next.push(node);
                            }
                        }
                    }
                    next
                }
            };
        }
        Ok(roots)
    }

    /// One engine called with one root, as `_queryEngineAll` calls it.
    fn engine(
        &self,
        part: &SelectorPart,
        root: Node,
        scoped: &Scoped,
    ) -> Result<Vec<Node>, Thrown> {
        match (part.name.as_str(), &part.body) {
            ("internal:neo-from", _) => Ok(scoped.from.clone()),
            ("internal:neo-below", SelectorBody::Text(body)) => {
                self.candidates(scoped, body, root, false)
            }
            ("internal:neo-inside", SelectorBody::Text(body)) => {
                self.candidates(scoped, body, root, true)
            }
            ("internal:attr", SelectorBody::Text(body)) => self.attr(body, root),
            // `XPathEngine` for the one path the tests send: the root's parent element.
            ("xpath", SelectorBody::Text(body)) if body == ".." => Ok(match root {
                Node::Document => Vec::new(),
                Node::Element(index) => self.elements[index]
                    .parent
                    .map(Node::Element)
                    .into_iter()
                    .collect(),
            }),
            ("internal:has", SelectorBody::Nested(nested)) => {
                self.presence(&nested.parsed, root, scoped, true)
            }
            ("internal:has-not", SelectorBody::Nested(nested)) => {
                self.presence(&nested.parsed, root, scoped, false)
            }
            ("internal:chain", SelectorBody::Nested(nested)) => {
                self.query_all(&nested.parsed, root, scoped)
            }
            ("internal:describe", _) => Ok(match root {
                Node::Document => Vec::new(),
                element @ Node::Element(_) => vec![element],
            }),
            (name @ ("role" | "internal:role" | "aria-ref"), _) => {
                panic!("a page stage carried `{name}`, a tree part Rust evaluates")
            }
            (name, _) => Err(Thrown {
                message: format!("the test page has no engine \"{name}\""),
                class: ErrorClass::Selector,
            }),
        }
    }

    /// `internal:neo-below` and `internal:neo-inside` (the probe's `scoped`): a set's members
    /// inside `root` by shadow-free containment, the root itself only when `inclusive`. An error
    /// set throws with its class, and a stale-ref set matches nothing and records its body, each
    /// only once a root reaches it.
    fn candidates(
        &self,
        scoped: &Scoped,
        body: &str,
        root: Node,
        inclusive: bool,
    ) -> Result<Vec<Node>, Thrown> {
        let set = body
            .parse::<usize>()
            .ok()
            .and_then(|index| scoped.sets.get(index))
            .ok_or_else(|| Thrown {
                message: format!("no candidate set {body}"),
                class: ErrorClass::Internal,
            })?;
        match set {
            Candidates::Error { message, class } => Err(Thrown {
                message: message.clone(),
                class: *class,
            }),
            Candidates::StaleRef(body) => {
                scoped.reached.borrow_mut().push(body.clone());
                Ok(Vec::new())
            }
            Candidates::Members(members) => Ok(members
                .iter()
                .copied()
                .filter(|&member| {
                    if member == root {
                        inclusive
                    } else {
                        self.inside(root, member)
                    }
                })
                .collect()),
        }
    }

    /// `isInsideScope(root, member)` for a member that is not the root.
    fn inside(&self, root: Node, member: Node) -> bool {
        match (root, member) {
            (_, Node::Document) => false,
            (Node::Document, Node::Element(_)) => true,
            (Node::Element(root), Node::Element(member)) => {
                root < member && member < self.elements[root].end
            }
        }
    }

    /// `internal:attr` (`_createNamedAttributeEngine`): the root's connected descendants whose
    /// attribute matches, presence or exact equality.
    fn attr(&self, body: &str, root: Node) -> Result<Vec<Node>, Thrown> {
        let parsed =
            parse_attribute_selector(body, UnquotedStrings::Allow).map_err(|error| Thrown {
                message: error.to_string(),
                class: ErrorClass::Selector,
            })?;
        let [attribute] = parsed.attributes.as_slice() else {
            return Err(script(&format!("Malformed attribute selector: {body}")));
        };
        let range = match root {
            Node::Document => 0..self.elements.len(),
            Node::Element(index) => index + 1..self.elements[index].end,
        };
        Ok(range
            .filter(|&index| {
                let element = &self.elements[index];
                element.connected
                    && element.attrs.iter().any(|(name, value)| {
                        *name == attribute.name
                            && match (&attribute.op, &attribute.value) {
                                (AttributeSelectorOperator::Truthy, _) => true,
                                (AttributeSelectorOperator::Eq, AttributeValue::String(wanted)) => {
                                    value == wanted
                                }
                                (op, _) => panic!("the test page compares with `=` only, not {op}"),
                            }
                    })
            })
            .map(Node::Element)
            .collect())
    }

    /// `internal:has` and `internal:has-not`: an element root is kept when its nested query is
    /// non-empty, or empty; a Document root is never kept.
    fn presence(
        &self,
        nested: &ParsedSelector,
        root: Node,
        scoped: &Scoped,
        has: bool,
    ) -> Result<Vec<Node>, Thrown> {
        if root == Node::Document {
            return Ok(Vec::new());
        }
        let found = !self.query_all(nested, root, scoped)?.is_empty();
        Ok(if found == has { vec![root] } else { Vec::new() })
    }
}

/// `_queryNth` for the bodies the tests send: an index from the start, or `-1` for the last.
fn nth(list: &[Node], body: &str) -> Vec<Node> {
    let index: i64 = body
        .parse()
        .unwrap_or_else(|_| panic!("the test page reads nth={body} as an integer"));
    let index = if index == -1 {
        list.len().checked_sub(1)
    } else {
        usize::try_from(index).ok()
    };
    index
        .and_then(|index| list.get(index))
        .copied()
        .into_iter()
        .collect()
}

/// `sortInDOMOrder`: elements in document order. Upstream finds no parent element for the
/// Document, so it starts its own tree, visited first only when the union met it first.
fn sort_in_dom_order(union: Vec<Node>) -> Vec<Node> {
    let document_first = union.first() == Some(&Node::Document);
    let has_document = union.contains(&Node::Document);
    let mut elements: Vec<usize> = union
        .into_iter()
        .filter_map(|node| match node {
            Node::Element(index) => Some(index),
            Node::Document => None,
        })
        .collect();
    elements.sort_unstable();
    let mut sorted: Vec<Node> = elements.into_iter().map(Node::Element).collect();
    if has_document {
        if document_first {
            sorted.insert(0, Node::Document);
        } else {
            sorted.push(Node::Document);
        }
    }
    sorted
}

fn script(message: &str) -> Thrown {
    Thrown {
        message: message.to_owned(),
        class: ErrorClass::Script,
    }
}

fn internal(message: &str) -> ProbeReply {
    ProbeReply::Error {
        message: message.to_owned(),
        class: ErrorClass::Internal,
        tree_depth: None,
        resolved: None,
    }
}

/// The page answers at once: a run's awaits on it return without suspending.
impl ChunkPage for FakePage {
    type Error = Infallible;

    fn call(
        &mut self,
        request: &ProbeRequest,
        nodes: &[NodeId],
    ) -> impl Future<Output = Result<Option<ProbeReply>, Infallible>> {
        let handed_doms = self.doms(nodes);
        let resolved = self.resolve(nodes);
        let (reply, sent) = match request {
            ProbeRequest::Stage {
                selector,
                from,
                sets,
                strict,
            } => (
                resolved.map(|handed| {
                    self.stage(selector, from.as_deref(), sets, strict.as_ref(), &handed)
                }),
                Sent::Stage {
                    selector: serde_json::to_value(selector).unwrap_or(Value::Null),
                    from: from.clone(),
                    sets: sets.clone(),
                    strict: strict.is_some(),
                    handed: handed_doms,
                },
            ),
            ProbeRequest::DescribePins { members, chunk } => (
                resolved.map(|handed| self.describe(members, chunk.as_ref(), &handed)),
                Sent::Describe {
                    members: members.clone(),
                    handed: handed_doms,
                },
            ),
            other => panic!("a run sends stage and describePins only, not {other:?}"),
        };
        self.record(sent);
        future::ready(Ok(reply))
    }

    fn hydrate(&mut self, node: NodeId) -> impl Future<Output = Result<Option<Pin>, Infallible>> {
        let index = *self
            .by_node
            .get(&node)
            .unwrap_or_else(|| panic!("{node:?} is not an element of the page"));
        let element = &self.elements[index];
        let live = element.connected && !element.destroyed;
        let dom = element.dom;
        let pin = live.then(|| self.pin(index));
        self.record(Sent::Hydrate(dom));
        future::ready(Ok(pin))
    }

    fn keys(
        &mut self,
        pins: &[Pin],
    ) -> impl Future<Output = Result<Option<Vec<NodeKey>>, Infallible>> {
        let keys: Option<Vec<NodeKey>> = pins
            .iter()
            .map(|pin| match self.held(SetMember::Pin(*pin), &[]) {
                Some(Node::Element(index)) => key(self.elements[index].dom).ok(),
                Some(Node::Document) | None => None,
            })
            .collect();
        self.record(Sent::Keys(pins.to_vec()));
        future::ready(Ok(keys))
    }
}
