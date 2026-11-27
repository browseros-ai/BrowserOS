use std::collections::HashSet;

use thiserror::Error;

use neo_snapshot::template::AriaTemplateNode;

use crate::tree::{AriaRoot, DocId, NameCache, NodeId, SnapshotTree, TreeDoc, TreeNode};

use super::accessible_text::AccessibleText;
use super::aria_template::{self, AriaTemplateError};
use super::attribute_selector;
use super::parsed_selector::{ParsedSelector, SelectorBody, SelectorPart};
use super::ref_resolver::RefResolver;
use super::role_match::{RoleEngine, RoleMatch};
use super::role_query::{RoleQuery, RoleQueryError};
use super::tree_matches::TreeMatches;

/// A selector chunk that Rust evaluates entirely over one [`SnapshotTree`]: `role`,
/// `internal:role`, `aria-ref`, `aria-template` and `internal:describe` parts, `nth`, and the
/// combinators `internal:has`, `internal:has-not`, `internal:and`, `internal:or` and
/// `internal:chain` whose nested selectors hold only such parts (design 5.2, 5.3).
///
/// It is the tree half of the locator engine. Planning a chunk that mixes tree and page parts,
/// and moving sets between the two, is `ChunkPlan`'s and `ChunkRun`'s (design 5.4). Evaluation
/// ports `InjectedScript.querySelectorAll` and the engines it calls, with one accepted difference:
/// order is the tree's flat-tree order, where Playwright lists light DOM before shadow content.
#[derive(Clone, Debug)]
pub struct TreeStage {
    body: StageBody,
}

/// A planned selector, as `querySelectorAll` would run it.
#[derive(Clone, Debug)]
enum StageBody {
    /// The parts in order, a capture (`*`) already rewritten into a trailing `internal:has`
    /// (`injectedScript.ts:377-387`).
    Steps(Vec<Step>),
    /// A captured selector that also has an `nth` part. Upstream refuses it when the selector
    /// runs, not when it is parsed, so the error waits for evaluation.
    NthWithCapture,
}

/// One part of a planned selector.
#[derive(Clone, Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "a selector has a handful of steps, nearly all role parts; boxing them would add an \
              allocation per part and save nothing"
)]
enum Step {
    /// `role` or `internal:role`. Upstream parses and validates the body each time the engine
    /// runs, so a bad body fails only once a root reaches it.
    Role(Result<RoleMatch, RoleQueryError>),
    /// `aria-ref`, by its raw body.
    AriaRef(String),
    /// `aria-template`. Upstream parses the body each time the engine runs, so a body that is
    /// no template fails only once a root reaches it. A page form's stand-in for a template
    /// leaf holds [`AriaTemplateError::OutsideTree`] here ([`TreeStage::template_outside_tree`]).
    AriaTemplate(Result<AriaTemplateNode, AriaTemplateError>),
    /// `internal:describe`: each element root itself.
    Describe,
    /// `nth`, its body already converted as JavaScript's unary `+` does.
    Nth(f64),
    /// `internal:has`.
    Has(TreeStage),
    /// `internal:has-not`.
    HasNot(TreeStage),
    /// `internal:and`.
    And(TreeStage),
    /// `internal:or`.
    Or(TreeStage),
    /// `internal:chain`.
    Chain(TreeStage),
}

/// Where a stage starts, as Playwright's `querySelectorAll(selector, root)` takes it.
#[derive(Clone, Copy, Debug)]
pub enum Scope<'a> {
    /// A whole chunk, run from its query root: a document's root node, which stands for the
    /// `Document`, or an element, as an element handle's query starts.
    Root(NodeId),
    /// The parts after an earlier stage of the same chunk. `roots` is the set that stage
    /// produced, in its order. `root` is the chunk's query root, which `internal:and` and
    /// `internal:or` evaluate their nested selectors against.
    After {
        /// The chunk's query root.
        root: NodeId,
        /// The earlier stage's matches, in order.
        roots: &'a [NodeId],
    },
}

impl Scope<'_> {
    /// The whole of `doc`, or `None` when the capture holds no node of it.
    #[must_use]
    pub fn document(tree: &SnapshotTree, doc: DocId) -> Option<Scope<'static>> {
        tree.document(doc)?.root_node().map(Scope::Root)
    }
}

/// A part that only the page evaluates, found while planning a chunk as one tree stage.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("selector part \"{part}\" is evaluated in the page, not over the snapshot tree")]
pub struct NotTreeStage {
    part: String,
}

impl NotTreeStage {
    /// The engine name of the first such part, such as `css` or `internal:text`.
    #[must_use]
    pub fn part(&self) -> &str {
        &self.part
    }
}

/// Why a tree stage stopped, with upstream Playwright 1.64.1's text.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TreeStageError {
    /// A `role` or `internal:role` body failed to parse or validate when a root first reached it.
    #[error(transparent)]
    Role(#[from] RoleQueryError),
    /// A captured selector (`*`) also held an `nth` part (`injectedScript.ts:378-379`).
    #[error("Can't query n-th element in a request with the capture.")]
    NthWithCapture,
    /// An `aria-template` part's body, its match, or the root it was reached from.
    #[error(transparent)]
    AriaTemplate(#[from] AriaTemplateError),
}

impl TreeStage {
    /// Plans `selector` as one tree stage, or names the first part, nested ones included, that
    /// only the page evaluates.
    pub fn new(selector: &ParsedSelector) -> Result<Self, NotTreeStage> {
        let planned: Result<Vec<Step>, NotTreeStage> =
            selector.parts.iter().map(plan_part).collect();
        let mut steps = planned?;
        let body = match selector.capture {
            None => StageBody::Steps(steps),
            // Only this selector's own parts count: a nested `nth` is its nested selector's
            // business (`injectedScript.ts:378`).
            Some(_) if selector.parts.iter().any(|part| part.name == "nth") => {
                StageBody::NthWithCapture
            }
            Some(capture) => {
                // `*a >> b >> c` runs as `a >> internal:has="b >> c"`: the captured part's
                // elements, kept when the rest matches inside them.
                let rest = steps.split_off(capture.saturating_add(1).min(steps.len()));
                if !rest.is_empty() {
                    steps.push(Step::Has(Self {
                        body: StageBody::Steps(rest),
                    }));
                }
                StageBody::Steps(steps)
            }
        };
        Ok(Self { body })
    }

    /// The candidate a page form holds for an `aria-template` leaf: one step that refuses the
    /// query as soon as a root reaches it, since a template's matches depend on its root and no
    /// candidate set can stand for them. The page throws the refusal where upstream's engine
    /// would have run, and never sees the template.
    pub(super) fn template_outside_tree() -> Self {
        Self {
            body: StageBody::Steps(vec![Step::AriaTemplate(Err(
                AriaTemplateError::OutsideTree,
            ))]),
        }
    }

    /// Evaluates the stage over `tree` from `scope`, reading names and descriptions through
    /// `text` and the targets of `aria-ref` parts through `refs`.
    pub fn select(
        &self,
        tree: &SnapshotTree,
        scope: Scope<'_>,
        text: &dyn AccessibleText,
        refs: &dyn RefResolver,
    ) -> Result<TreeMatches, TreeStageError> {
        let mut evaluator = Evaluator {
            tree,
            text,
            refs,
            stale_ref: None,
        };
        let nodes = match scope {
            Scope::Root(root) if tree.node(root).is_some() => evaluator.query(self, root)?,
            Scope::After { root, roots } if tree.node(root).is_some() => {
                // An earlier stage hands over a set; keep it one, in its order, over this tree.
                let mut set = NodeSet::default();
                for &id in roots.iter().filter(|&&id| tree.node(id).is_some()) {
                    set.insert(id);
                }
                evaluator.run(self, root, set.into_nodes())?
            }
            Scope::Root(_) | Scope::After { .. } => Vec::new(),
        };
        Ok(TreeMatches::new(nodes, evaluator.stale_ref))
    }
}

/// Maps one selector part to its step, or refuses a part the page evaluates. A body of the wrong
/// shape for its engine is refused too: the page would reject it, not the tree.
fn plan_part(part: &SelectorPart) -> Result<Step, NotTreeStage> {
    let step = match (part.name.as_str(), &part.body) {
        ("role", SelectorBody::Text(body)) => Step::Role(role_match(body, RoleEngine::Role)),
        ("internal:role", SelectorBody::Text(body)) => {
            Step::Role(role_match(body, RoleEngine::InternalRole))
        }
        ("aria-ref", SelectorBody::Text(body)) => Step::AriaRef(body.clone()),
        ("aria-template", SelectorBody::Text(body)) => {
            Step::AriaTemplate(aria_template::parse(body))
        }
        ("internal:describe", SelectorBody::Text(_)) => Step::Describe,
        ("nth", SelectorBody::Text(body)) => {
            Step::Nth(attribute_selector::js_string_to_number(body))
        }
        ("internal:has", SelectorBody::Nested(nested)) => {
            Step::Has(TreeStage::new(&nested.parsed)?)
        }
        ("internal:has-not", SelectorBody::Nested(nested)) => {
            Step::HasNot(TreeStage::new(&nested.parsed)?)
        }
        ("internal:and", SelectorBody::Nested(nested)) => {
            Step::And(TreeStage::new(&nested.parsed)?)
        }
        ("internal:or", SelectorBody::Nested(nested)) => Step::Or(TreeStage::new(&nested.parsed)?),
        ("internal:chain", SelectorBody::Nested(nested)) => {
            Step::Chain(TreeStage::new(&nested.parsed)?)
        }
        _ => {
            return Err(NotTreeStage {
                part: part.name.clone(),
            });
        }
    };
    Ok(step)
}

fn role_match(body: &str, engine: RoleEngine) -> Result<RoleMatch, RoleQueryError> {
    RoleQuery::parse(body).map(|query| RoleMatch::new(query, engine))
}

/// One `select` call: the tree and the two seams it reads, and the first `aria-ref` that named no
/// live node. Nested selectors run through the same evaluator, so a ref that goes stale inside an
/// `internal:has` reports into the same run, as the page's ref engine collects them per query
/// (`aria-ref.ts` `RefMap.query`).
struct Evaluator<'t> {
    tree: &'t SnapshotTree,
    text: &'t dyn AccessibleText,
    refs: &'t dyn RefResolver,
    stale_ref: Option<String>,
}

impl Evaluator<'_> {
    /// `querySelectorAll(stage, root)`.
    fn query(&mut self, stage: &TreeStage, root: NodeId) -> Result<Vec<NodeId>, TreeStageError> {
        self.run(stage, root, vec![root])
    }

    /// `querySelectorAll`'s loop over the parts (`injectedScript.ts:403-424`), starting from
    /// `roots` with `root` as the query root. Every set is insertion-ordered and duplicate-free,
    /// as a JavaScript `Set<Element>` is, and an error leaves at once, as a thrown one does.
    fn run(
        &mut self,
        stage: &TreeStage,
        root: NodeId,
        mut roots: Vec<NodeId>,
    ) -> Result<Vec<NodeId>, TreeStageError> {
        let StageBody::Steps(steps) = &stage.body else {
            return Err(TreeStageError::NthWithCapture);
        };
        for step in steps {
            roots = match step {
                Step::Nth(index) => nth(&roots, *index),
                // The nested selector runs from the query root, and its order wins.
                Step::And(nested) => {
                    let current: HashSet<NodeId> = roots.into_iter().collect();
                    let mut and = self.query(nested, root)?;
                    and.retain(|id| current.contains(id));
                    and
                }
                Step::Or(nested) => {
                    roots.extend(self.query(nested, root)?);
                    self.sort_in_dom_order(roots)
                }
                Step::Role(role) => self.role(role, &roots)?,
                Step::AriaTemplate(template) => self.aria_template(template, &roots)?,
                Step::AriaRef(body) => {
                    let mut next = NodeSet::default();
                    for &scope in &roots {
                        if let Some(target) = self.aria_ref(body, scope) {
                            next.insert(target);
                        }
                    }
                    next.into_nodes()
                }
                Step::Describe => {
                    roots.retain(|&id| self.is_element(id));
                    roots
                }
                Step::Has(nested) => self.filter_by_presence(roots, nested, Presence::Has)?,
                Step::HasNot(nested) => self.filter_by_presence(roots, nested, Presence::HasNot)?,
                Step::Chain(nested) => {
                    let mut next = NodeSet::default();
                    for &scope in &roots {
                        for id in self.query(nested, scope)? {
                            next.insert(id);
                        }
                    }
                    next.into_nodes()
                }
            };
        }
        Ok(roots)
    }

    /// The role engine over every root: each root's strict descendants in its own document, in
    /// arena order (`roleSelectorEngine.ts:190-201`).
    ///
    /// A root inside the last root scanned in the same document is skipped: every match under it
    /// is already in the set, so inserting them again would change nothing. That keeps
    /// `list >> listitem` on nested lists linear instead of quadratic.
    fn role(
        &self,
        role: &Result<RoleMatch, RoleQueryError>,
        roots: &[NodeId],
    ) -> Result<Vec<NodeId>, TreeStageError> {
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let role = match role {
            Ok(role) => role,
            Err(error) => return Err(error.clone().into()),
        };
        let mut next = NodeSet::default();
        let mut scanned: Option<&TreeNode> = None;
        for root in roots.iter().filter_map(|&id| self.tree.node(id)) {
            if scanned.is_some_and(|outer| is_strict_descendant_in_doc(outer, root)) {
                continue;
            }
            scanned = Some(root);
            let nodes = self.tree.nodes();
            let end = root.subtree_end().index();
            let mut cursor = next_in_doc(root);
            while cursor < end {
                let Some(node) = nodes.get(cursor) else {
                    break;
                };
                if role.matches(node, self.text) {
                    next.insert(node.id());
                }
                cursor = next_in_doc(node);
            }
        }
        Ok(next.into_nodes())
    }

    /// The `aria-template` engine over every root, each matched from its own tree as upstream's
    /// engine runs per root; a root inside another root is matched again, since its top-most
    /// matches can lie inside the outer root's. A document root's tree is its document's.
    fn aria_template(
        &self,
        template: &Result<AriaTemplateNode, AriaTemplateError>,
        roots: &[NodeId],
    ) -> Result<Vec<NodeId>, TreeStageError> {
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let template = template.as_ref().map_err(Clone::clone)?;
        // Emission needs the tree's own name cache, not the `AccessibleText` seam, so the step
        // builds one and every root's preparation shares it.
        let names = NameCache::new(self.tree);
        let mut next = NodeSet::default();
        for &root in roots {
            let from = match self.tree.node(root) {
                Some(node) if !self.is_element(root) => AriaRoot::Document(node.doc()),
                Some(_) => AriaRoot::Subtree(root),
                None => continue,
            };
            for id in aria_template::matches(self.tree, &names, from, root, template)? {
                next.insert(id);
            }
        }
        Ok(next.into_nodes())
    }

    /// Neo's `aria-ref` engine for one root (`aria-ref.ts` `install`): the ref's node when it is
    /// inside `scope`, the root itself included, and in its document (`isInsideScope`).
    fn aria_ref(&mut self, body: &str, scope: NodeId) -> Option<NodeId> {
        let Some(key) = self.refs.resolve(body) else {
            self.stale_ref.get_or_insert_with(|| body.to_owned());
            return None;
        };
        // A live ref whose node this capture lacks is not stale: only hydration can tell a
        // removed node from one with no box (design 6).
        let target = self.tree.node_by_key(&key)?;
        let scope = self.tree.node(scope)?;
        (target.doc() == scope.doc() && scope.subtree_range().contains(target.id()))
            .then(|| target.id())
    }

    /// `internal:has` and `internal:has-not` (`injectedScript.ts:587-605`): an element root is
    /// kept when its nested query is non-empty, or empty. A document root is never kept, and its
    /// nested query never runs.
    fn filter_by_presence(
        &mut self,
        roots: Vec<NodeId>,
        nested: &TreeStage,
        presence: Presence,
    ) -> Result<Vec<NodeId>, TreeStageError> {
        let mut kept = Vec::with_capacity(roots.len());
        for id in roots {
            if !self.is_element(id) {
                continue;
            }
            let has = !self.query(nested, id)?.is_empty();
            if has == (presence == Presence::Has) {
                kept.push(id);
            }
        }
        Ok(kept)
    }

    /// `sortInDOMOrder(new Set(union))` (`selectorEvaluator.ts:555-604`) over a union in insertion
    /// order. Arena order is the flat tree's document order for elements. The `Document` is the
    /// exception: upstream finds no parent for it nor for `<html>`, so each starts its own tree,
    /// and the trees are visited in the order they were first appended. The `Document`, which a
    /// parsed selector brings in through an `nth` that keeps the query root, therefore leads only
    /// when it was the union's first member, and otherwise follows every element.
    fn sort_in_dom_order(&self, mut union: Vec<NodeId>) -> Vec<NodeId> {
        let first = union.first().copied();
        union.sort_unstable();
        union.dedup();
        if let Some(position) = union.iter().position(|&id| !self.is_element(id))
            && union.get(position).copied() != first
        {
            let document = union.remove(position);
            union.push(document);
        }
        union
    }

    /// `nodeType === 1`: every node but a document's root node, which stands for the `Document`.
    fn is_element(&self, id: NodeId) -> bool {
        self.tree
            .node(id)
            .and_then(|node| self.tree.document(node.doc()))
            .and_then(TreeDoc::root_node)
            .is_some_and(|doc_root| doc_root != id)
    }
}

/// Which outcome of its nested query keeps a root.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Has,
    HasNot,
}

/// A set under construction that keeps first-insertion order, as a JavaScript `Set` does.
#[derive(Default)]
struct NodeSet {
    nodes: Vec<NodeId>,
    seen: HashSet<NodeId>,
}

impl NodeSet {
    fn insert(&mut self, id: NodeId) {
        if self.seen.insert(id) {
            self.nodes.push(id);
        }
    }

    fn into_nodes(self) -> Vec<NodeId> {
        self.nodes
    }
}

/// The arena index after `node` in its own document: a stitched child document lives in its
/// `<iframe>` node's subtree, and the role engine's `querySelectorAll('*')` never enters it.
fn next_in_doc(node: &TreeNode) -> usize {
    if node.child_doc().is_some() {
        node.subtree_end().index()
    } else {
        node.id().index() + 1
    }
}

fn is_strict_descendant_in_doc(ancestor: &TreeNode, node: &TreeNode) -> bool {
    ancestor.doc() == node.doc() && ancestor.id() < node.id() && node.id() < ancestor.subtree_end()
}

/// `_queryNth` (`injectedScript.ts:297-303`): `-1` is the last element, and any other index goes
/// through `list.slice(n, n + 1)`, with `n + 1` added before truncation.
#[expect(
    clippy::float_cmp,
    reason = "`nth === -1` is JavaScript's exact comparison; -1.5 must not count as -1"
)]
fn nth(list: &[NodeId], index: f64) -> Vec<NodeId> {
    let index = if index == -1.0 {
        js_length(list.len()) - 1.0
    } else {
        index
    };
    let start = slice_bound(index, list.len());
    let end = slice_bound(index + 1.0, list.len());
    list.get(start..end)
        .map_or_else(Vec::new, <[NodeId]>::to_vec)
}

/// One bound of `Array.prototype.slice`: `ToIntegerOrInfinity` (NaN is 0, others truncate toward
/// zero), then a negative bound counts from the end, clamped into `[0, len]`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the bound is a whole number clamped into [0, len]"
)]
fn slice_bound(relative: f64, len: usize) -> usize {
    let len_f = js_length(len);
    let integer = if relative.is_nan() {
        0.0
    } else {
        relative.trunc()
    };
    let bound = if integer < 0.0 {
        (len_f + integer).max(0.0)
    } else {
        integer.min(len_f)
    };
    bound as usize
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a set holds at most the arena's u32 node count, which an f64 represents exactly"
)]
fn js_length(len: usize) -> f64 {
    len as f64
}

#[cfg(test)]
mod tests {
    use crate::tree::NodeId;

    #[test]
    fn nth_follows_array_slice_rules() {
        let list: Vec<NodeId> = (10..15).map(NodeId::new).collect();
        let cases: [(f64, &[u32], &str); 13] = [
            (0.0, &[10], "0 is the first element"),
            (4.0, &[14], "the last index is in range"),
            (5.0, &[], "an index past the end is empty"),
            (-1.0, &[14], "-1 is the last element"),
            (
                -2.0,
                &[13],
                "-2 counts from the end through `slice(-2, -1)`",
            ),
            (-5.0, &[10], "-len is the first element"),
            (-6.0, &[], "`slice(-6, -5)` clamps to `slice(0, 0)`"),
            (1.5, &[11], "`slice(1.5, 2.5)` truncates to `slice(1, 2)`"),
            (-0.5, &[], "`slice(-0.5, 0.5)` truncates to `slice(0, 0)`"),
            (
                -1.5,
                &[],
                "`slice(-1.5, -0.5)` is `slice(-1, 0)`: -1 only means last when exact",
            ),
            (f64::NAN, &[], "NaN (`nth=abc`) makes both bounds 0"),
            (
                f64::INFINITY,
                &[],
                "Infinity clamps both bounds to the length",
            ),
            (f64::NEG_INFINITY, &[], "-Infinity clamps both bounds to 0"),
        ];
        for (index, expected, why) in cases {
            let expected: Vec<NodeId> = expected.iter().copied().map(NodeId::new).collect();
            assert_eq!(super::nth(&list, index), expected, "nth={index}: {why}");
        }
        assert_eq!(
            super::nth(&[], -1.0),
            Vec::<NodeId>::new(),
            "-1 on an empty set is `slice(-1, 0)`, which is empty"
        );
    }
}
