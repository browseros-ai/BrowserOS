//! Randomized differential test of `TreeStage` against the query code Neo ships (`injected.js`, Playwright 1.64.1) run in QuickJS.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use neo_snapshot_apc::proto::BrowserOsTriState;
use neo_snapshot_apc::select::{
    AccessibleText, IncludeHidden, ParsedSelector, RefResolver, Scope, TreeStage,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocId, DocToken, DomNodeId, NodeBuilder, NodeId, NodeKey, Role,
    SnapshotTree, TreeDoc, TreeNode, TriState,
};
use rquickjs::{CatchResultExt, Context, Function, Runtime};
use serde::Deserialize;
use serde_json::{Value, json};

const MAIN_DOC: &str = "doc-main";

/// The fixed seed of the randomized run; tree `i` draws from `SEED + i`, so a reported tree
/// index reproduces alone.
const SEED: u64 = 0x5e1e_c7ed_7ee5_0001;
const TREES: u64 = 300;
const SELECTORS_PER_TREE: usize = 40;

/// The body of a ref registered on both sides whose node is not in the capture (design D2: an
/// element with no box). See [`RefTarget::Ghost`].
const GHOST_REF: &str = "e900";
const GHOST_DOM_ID: i32 = 900_000;

/// The last statement of `assets/injected.js`, which the glue replaces.
const PROBE_TAIL: &str = "globalThis.__neoProbe ??= createProbe();\n})();";

/// Runs inside the bundle's IIFE in place of [`PROBE_TAIL`]. It rebinds roleUtils' ten fact
/// getters to read `element.facts`, which the test fills from the built tree, so the oracle judges
/// query semantics and never role computation. Everything the getters feed (`queryRole`,
/// `createRoleEngine`, `matchesAttributePart`, `normalizeWhiteSpace`, `querySelectorAll`, the
/// combinator engines, `sortInDOMOrder`, Neo's `RefMap`) runs as shipped.
const PROBE_GLUE: &str = r"
  getAriaRole = (element) => element.facts.role ?? null;
  getAriaSelected = (element) => element.facts.selected === true;
  getAriaChecked = (element) => element.facts.checked ?? false;
  getAriaPressed = (element) => element.facts.pressed ?? false;
  getAriaExpanded = (element) => element.facts.expanded;
  getAriaLevel = (element) => element.facts.level ?? 0;
  getAriaDisabled = (element) => element.facts.disabled === true;
  isElementHiddenForAria = (element) => element.facts.hidden === true;
  getElementAccessibleNameText = (element, includeHidden) =>
    (includeHidden ? element.facts.nameHidden : element.facts.name) ?? '';
  getElementAccessibleDescription = (element, includeHidden) => ({
    text: (includeHidden ? element.facts.descHidden : element.facts.desc) ?? '',
    derivedFromContent: false,
  });
  globalThis.__probe = { InjectedScript, RefMap, isInsideScope, createRoleEngine };
})();";

/// The mock DOM and the per-tree runner.
///
/// The DOM implements only the members the vendored query path reads. One `Document` holds a
/// single `html` element, as a real page does: `sortInDOMOrder` treats every parentless element
/// as a separate root and keeps roots in insertion order, so several top-level elements would make
/// `internal:or` disagree with document order for that reason alone. Shadow roots stay null and
/// every element is connected.
///
/// The runner builds the engine table the way the probe does (upstream's role, has, has-not,
/// chain and describe engines, then Neo's `RefMap` installs `aria-ref`) on a bare
/// `InjectedScript` prototype, and calls the real `querySelectorAll` with the `Document` as root.
const MOCK_DOM_JS: &str = r"
(() => {
  class MockElement {
    constructor(id, facts) {
      this.id = id;
      this.facts = facts;
      this.children = [];
      this.parentNode = null;
      this.indexInParent = -1;
      this.nodeType = 1;
      this.nodeName = 'DIV';
      this.shadowRoot = null;
      this.isConnected = true;
    }
    get parentElement() {
      return this.parentNode !== null && this.parentNode.nodeType === 1 ? this.parentNode : null;
    }
    get firstElementChild() { return this.children[0] ?? null; }
    get nextElementSibling() {
      return this.parentNode === null ? null : this.parentNode.children[this.indexInParent + 1] ?? null;
    }
    get childElementCount() { return this.children.length; }
    appendChild(child) {
      child.parentNode = this;
      child.indexInParent = this.children.length;
      this.children.push(child);
    }
    querySelectorAll() {
      const out = [];
      const stack = this.children.slice().reverse();
      while (stack.length > 0) {
        const element = stack.pop();
        out.push(element);
        for (let i = element.children.length - 1; i >= 0; i--) stack.push(element.children[i]);
      }
      return out;
    }
    contains(other) {
      for (let node = other; node !== null; node = node.parentNode) if (node === this) return true;
      return false;
    }
    getRootNode() {
      let node = this;
      while (node.parentNode !== null) node = node.parentNode;
      return node;
    }
  }

  class MockDocument extends MockElement {
    constructor(id) {
      super(id, {});
      this.nodeType = 9;
      this.nodeName = '#document';
    }
  }

  globalThis.__runTree = (inputJson) => {
    const input = JSON.parse(inputJson);
    const { InjectedScript, RefMap, isInsideScope, createRoleEngine } = globalThis.__probe;
    const { splitSelectorByFrame } = globalThis.__prelude;

    const document = new MockDocument(input.root);
    const byId = new Map([[input.root, document]]);
    for (const [id, parent, facts] of input.elements) {
      const element = new MockElement(id, facts);
      byId.get(parent).appendChild(element);
      byId.set(id, element);
    }
    // RefMap holds elements weakly; the ghost must outlive every query of this tree.
    const ghost = new MockElement(-1, {});

    const proto = InjectedScript.prototype;
    const injected = Object.create(proto);
    injected._evaluator = { begin() {}, end() {} };
    injected.createStacklessError = (message) => new Error(message);
    const engines = new Map();
    engines.set('role', createRoleEngine(false));
    engines.set('internal:role', createRoleEngine(true));
    engines.set('internal:has', proto._createHasEngine.call(injected));
    engines.set('internal:has-not', proto._createHasNotEngine.call(injected));
    engines.set('internal:chain', proto._createInternalChainEngine.call(injected));
    engines.set('internal:describe', proto._createDescribeEngine.call(injected));
    const refs = new RefMap();
    refs.remember(new Map(input.refs.map(([ref, id]) =>
      [ref, { element: id === null ? ghost : byId.get(id) }])));
    refs.install(engines, { isInsideScope });
    injected._engines = engines;

    const answers = input.selectors.map((selector) => {
      let chunks;
      try {
        chunks = splitSelectorByFrame(selector).chunks;
      } catch (error) {
        return { kind: 'skip', message: String(error?.message) };
      }
      if (chunks.length !== 1)
        return { kind: 'harness', message: `expected one chunk, got ${chunks.length}` };
      const chunk = JSON.stringify(chunks[0]);
      try {
        const { elements, staleRef } =
          refs.query(() => injected.querySelectorAll(chunks[0], document));
        return { kind: 'ok', chunk, ids: elements.map((element) => element.id), staleRef: staleRef ?? null };
      } catch (error) {
        return { kind: 'error', chunk, message: String(error?.message) };
      }
    });
    if (ghost.id !== -1) throw new Error('ghost element was replaced');
    return JSON.stringify(answers);
  };
})();
";

/// Neo's query code in QuickJS: `assets/injected.js` with [`PROBE_GLUE`], `assets/prelude.js`
/// for `splitSelectorByFrame`, and [`MOCK_DOM_JS`].
///
/// QuickJS stands in for V8. `oracle_answers_hand_derived_cases` pins the engine behaviors the
/// role filters depend on (full Unicode upper-casing, ECMAScript whitespace) to V8's answers.
struct Oracle {
    context: Context,
}

/// One selector's answer from the oracle, as `__runTree` reports it.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum OracleAnswer {
    /// The prelude refused the selector string, so Rust would never see it.
    Skip { message: String },
    /// The runner itself failed: the harness is broken.
    Harness { message: String },
    /// `querySelectorAll` returned: the matched element ids and the first unresolved ref.
    Ok {
        chunk: String,
        ids: Vec<u32>,
        #[serde(rename = "staleRef")]
        stale_ref: Option<String>,
    },
    /// `querySelectorAll` threw.
    Error { chunk: String, message: String },
}

impl Oracle {
    fn load() -> Result<Self, Box<dyn Error>> {
        let probe = probe_script()?;
        let prelude = prelude_script()?;
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> Result<(), String> {
            for (label, source) in [
                ("injected.js", probe.as_str()),
                ("prelude.js", prelude.as_str()),
                ("mock DOM", MOCK_DOM_JS),
            ] {
                ctx.eval::<(), _>(source)
                    .catch(&ctx)
                    .map_err(|caught| format!("{label} failed to load: {caught}"))?;
            }
            Ok(())
        })?;
        Ok(Self { context })
    }

    /// Runs every selector of `selectors` against `page`, in order, in one mock DOM.
    fn run(&self, page: &Page, selectors: &[String]) -> Result<Vec<OracleAnswer>, Box<dyn Error>> {
        let input = serde_json::to_string(&page.oracle_input(selectors)?)?;
        let output: String = self.context.with(|ctx| -> Result<String, String> {
            let run: Function<'_> = ctx
                .globals()
                .get("__runTree")
                .catch(&ctx)
                .map_err(|caught| caught.to_string())?;
            run.call((input,))
                .catch(&ctx)
                .map_err(|caught| format!("__runTree threw: {caught}"))
        })?;
        let answers: Vec<OracleAnswer> = serde_json::from_str(&output)?;
        if answers.len() != selectors.len() {
            return Err(format!(
                "oracle answered {} of {} selectors",
                answers.len(),
                selectors.len()
            )
            .into());
        }
        Ok(answers)
    }

    fn eval_json(&self, script: &str) -> Result<Value, Box<dyn Error>> {
        let output: String = self.context.with(|ctx| -> Result<String, String> {
            ctx.eval::<String, _>(script)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())
        })?;
        Ok(serde_json::from_str(&output)?)
    }
}

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../neo-playwright/assets")
}

fn probe_script() -> Result<String, Box<dyn Error>> {
    let source = fs::read_to_string(assets_dir().join("injected.js"))?;
    let Some(body) = source.trim_end().strip_suffix(PROBE_TAIL) else {
        return Err("injected.js no longer ends with `globalThis.__neoProbe ??= createProbe();\\n})();`, so the oracle glue cannot replace it".into());
    };
    // The glue assigns to these bindings; an esbuild rename (`getAriaRole2`) would leave the real
    // roleUtils getter in place and the reassignment would only create a stray global.
    for name in [
        "getAriaRole",
        "getAriaSelected",
        "getAriaChecked",
        "getAriaPressed",
        "getAriaExpanded",
        "getAriaLevel",
        "getAriaDisabled",
        "isElementHiddenForAria",
        "getElementAccessibleNameText",
        "getElementAccessibleDescription",
    ] {
        let declarations = body.matches(&format!("function {name}(")).count();
        if declarations != 1 || body.contains(&format!("{name}2")) {
            return Err(format!(
                "injected.js must declare `function {name}(` exactly once, unrenamed, for the oracle glue to rebind it"
            )
            .into());
        }
    }
    Ok(format!("{body}\n{PROBE_GLUE}"))
}

fn prelude_script() -> Result<String, Box<dyn Error>> {
    let source = fs::read_to_string(assets_dir().join("prelude.js"))?;
    let Some(body) = source.trim_end().strip_suffix("})();") else {
        return Err("prelude.js no longer ends with `})();`".into());
    };
    Ok(format!(
        "globalThis.__neo = {{ brands: {{ register() {{}} }}, onReply() {{}}, onSettle() {{}}, provide() {{}} }};\n\
         {body}\n\
         globalThis.__prelude = {{ splitSelectorByFrame }};\n\
         }})();"
    ))
}

/// One capture as both sides see it: the built tree, its element nodes in preorder (the mock
/// DOM's elements, every node but the root and text), the test's names, and the registered refs.
struct Page {
    tree: SnapshotTree,
    root: NodeId,
    elements: Vec<NodeId>,
    names: Names,
    refs: Refs,
}

/// The test's accessible names and descriptions, standing in for the name pass (engine chunk 4).
#[derive(Default)]
struct Names(HashMap<NodeId, NodeText>);

/// One element's text, one value per [`IncludeHidden`] mode.
#[derive(Clone, Debug, Default)]
struct NodeText {
    name: String,
    name_hidden: String,
    description: String,
    description_hidden: String,
}

impl AccessibleText for Names {
    fn name(&self, node: NodeId, hidden: IncludeHidden) -> Cow<'_, str> {
        Cow::Borrowed(self.0.get(&node).map_or("", |text| match hidden {
            IncludeHidden::No => text.name.as_str(),
            IncludeHidden::Yes => text.name_hidden.as_str(),
        }))
    }

    fn description(&self, node: NodeId, hidden: IncludeHidden) -> Cow<'_, str> {
        Cow::Borrowed(self.0.get(&node).map_or("", |text| match hidden {
            IncludeHidden::No => text.description.as_str(),
            IncludeHidden::Yes => text.description_hidden.as_str(),
        }))
    }
}

/// Refs registered on both sides: Rust resolves a body to a key, the oracle's `RefMap` holds the
/// matching element (`None` for the ghost).
#[derive(Default)]
struct Refs {
    keys: HashMap<String, NodeKey>,
    targets: Vec<(String, Option<u32>)>,
}

impl RefResolver for Refs {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        self.keys.get(body).cloned()
    }
}

#[derive(Clone, Copy)]
enum RefTarget {
    Node(NodeId),
    /// A ref whose element lives in the page but not in this capture. Rust resolves it to a key
    /// the tree lacks; the oracle holds a connected element outside the document, which no query
    /// root contains. Both must answer "no match, not stale". This models the accepted gap, not
    /// Playwright, which would find such an element.
    Ghost,
}

impl Page {
    fn new(tree: SnapshotTree, text_dom_ids: &HashSet<i32>) -> Result<Self, Box<dyn Error>> {
        if tree.documents().len() != 1 {
            return Err("the oracle models one document; the tree must hold no iframes".into());
        }
        let root = tree
            .document(DocId::MAIN)
            .and_then(TreeDoc::root_node)
            .ok_or("the main document has no root node")?;
        let elements: Vec<NodeId> = tree
            .nodes()
            .iter()
            .filter(|node| {
                node.id() != root
                    && !node
                        .dom_node_id()
                        .is_some_and(|dom| text_dom_ids.contains(&dom.raw()))
            })
            .map(TreeNode::id)
            .collect();
        Ok(Self {
            tree,
            root,
            elements,
            names: Names::default(),
            refs: Refs::default(),
        })
    }

    fn register_ref(&mut self, body: &str, target: RefTarget) -> Result<(), Box<dyn Error>> {
        let (key, element) = match target {
            RefTarget::Node(id) => (
                self.tree.node_key(id).ok_or("a ref target has no key")?,
                Some(id.raw()),
            ),
            RefTarget::Ghost => (
                NodeKey::new(
                    DocToken::new(MAIN_DOC).ok_or("empty doc token")?,
                    DomNodeId::new(GHOST_DOM_ID),
                ),
                None,
            ),
        };
        if self.tree.node_by_key(&key).is_some() != element.is_some() {
            return Err(format!(
                "ref {body} must resolve in the tree exactly when it names a node"
            )
            .into());
        }
        self.refs.keys.insert(body.to_owned(), key);
        self.refs.targets.push((body.to_owned(), element));
        Ok(())
    }

    fn node_by_dom(&self, dom: i32) -> Result<NodeId, Box<dyn Error>> {
        let key = NodeKey::new(
            DocToken::new(MAIN_DOC).ok_or("empty doc token")?,
            DomNodeId::new(dom),
        );
        Ok(self
            .tree
            .node_by_key(&key)
            .ok_or_else(|| format!("no node has DOM id {dom}"))?
            .id())
    }

    /// The facts the oracle's getters read, taken from the built tree (never from the generator's
    /// inputs) plus the test's names.
    fn facts(&self, node: &TreeNode) -> Value {
        let state = node.state();
        let tristate = |value: Option<TriState>| match value {
            Some(TriState::True) => json!(true),
            Some(TriState::Mixed) => json!("mixed"),
            Some(TriState::False) | None => json!(false),
        };
        let text = self.names.0.get(&node.id()).cloned().unwrap_or_default();
        let mut facts = json!({
            "role": node.role().map(Role::as_str),
            "selected": state.selected().unwrap_or(false),
            "checked": tristate(state.checked()),
            "pressed": tristate(state.pressed()),
            "level": state.level().unwrap_or(0),
            "disabled": state.is_disabled(),
            "hidden": node.hidden().is_hidden_for_aria(),
            "name": text.name,
            "nameHidden": text.name_hidden,
            "desc": text.description,
            "descHidden": text.description_hidden,
        });
        if let (Some(expanded), Some(object)) = (state.expanded(), facts.as_object_mut()) {
            object.insert("expanded".to_owned(), json!(expanded));
        }
        facts
    }

    fn oracle_input(&self, selectors: &[String]) -> Result<Value, Box<dyn Error>> {
        let mut elements = Vec::with_capacity(self.elements.len());
        for &id in &self.elements {
            let node = self.tree.node(id).ok_or("element id outside the tree")?;
            let parent = node.parent().ok_or("an element has no parent")?;
            elements.push(json!([id.raw(), parent.raw(), self.facts(node)]));
        }
        Ok(json!({
            "root": self.root.raw(),
            "elements": elements,
            "refs": self.refs.targets,
            "selectors": selectors,
        }))
    }

    /// One line per element (indented by depth) with every fact both sides read, then the refs.
    fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "  #{} Document", self.root.raw());
        for &id in &self.elements {
            let Some(node) = self.tree.node(id) else {
                continue;
            };
            let depth = self.tree.ancestors(id).count();
            let state = node.state();
            let text = self.names.0.get(&id).cloned().unwrap_or_default();
            let _ = writeln!(
                out,
                "  {:indent$}#{} {} sel={} chk={:?} prs={:?} exp={:?} lvl={} dis={} hid={} name={:?}|{:?} desc={:?}|{:?}",
                "",
                id.raw(),
                node.role().map_or("-", Role::as_str),
                state.selected().unwrap_or(false),
                state.checked(),
                state.pressed(),
                state.expanded(),
                state.level().unwrap_or(0),
                state.is_disabled(),
                node.hidden().is_hidden_for_aria(),
                text.name,
                text.name_hidden,
                text.description,
                text.description_hidden,
                indent = depth * 2,
            );
        }
        let _ = writeln!(out, "  refs: {:?}", self.refs.targets);
        out
    }
}

/// What one side answered for one chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Matched {
        ids: Vec<u32>,
        stale_ref: Option<String>,
    },
    Failed(String),
}

impl Outcome {
    fn matched(ids: Vec<u32>, stale_ref: Option<&str>) -> Self {
        Self::Matched {
            ids,
            stale_ref: stale_ref.map(str::to_owned),
        }
    }

    /// A regex syntax error ends with the regex engine's own words after upstream's prefix, so it
    /// compares by that prefix, as `tests/select_parse.rs` does. Every other message compares byte
    /// for byte.
    fn comparable(self) -> Self {
        let Self::Failed(message) = self else {
            return self;
        };
        if message.starts_with("Error while parsing selector `")
            && let Some(prefix) = message.find("`: ").and_then(|at| message.get(..at))
        {
            return Self::Failed(format!("{prefix}`: <regex syntax error>"));
        }
        Self::Failed(message)
    }
}

/// Runs `chunk` through `TreeStage` from the main document. `Err` is a defect in the stage's
/// contract (a tree-only chunk refused), reported like a mismatch.
fn rust_outcome(page: &Page, chunk: &str) -> Result<Outcome, String> {
    let parsed: ParsedSelector = serde_json::from_str(chunk)
        .map_err(|error| format!("the prelude's chunk does not decode: {error}"))?;
    let stage = TreeStage::new(&parsed)
        .map_err(|error| format!("TreeStage::new refused a tree-only chunk: {error}"))?;
    let scope = Scope::document(&page.tree, DocId::MAIN).ok_or("no scope for the main document")?;
    Ok(
        match stage.select(&page.tree, scope, &page.names, &page.refs) {
            Ok(matches) => Outcome::matched(
                matches.nodes().iter().map(|id| id.raw()).collect(),
                matches.stale_ref(),
            ),
            Err(error) => Outcome::Failed(error.to_string()),
        },
    )
}

/// What the differential saw, so a reviewer can tell that it compared something.
#[derive(Debug, Default)]
struct Tally {
    compared: usize,
    skipped_by_prelude: usize,
    errors_compared: usize,
    with_matches: usize,
    stale_compared: usize,
    mismatches: usize,
}

const DETAILED_MISMATCHES: usize = 8;

/// `SplitMix64`: a tiny deterministic generator, so a failing case reproduces from its seed alone.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `0` when `n` is `0`.
    fn below(&mut self, n: usize) -> usize {
        let Ok(bound) = u64::try_from(n) else {
            return 0;
        };
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % bound).unwrap_or(0)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        items.get(self.below(items.len()))
    }

    fn pick_str(&mut self, items: &[&'static str]) -> &'static str {
        self.pick(items).copied().unwrap_or("")
    }
}

/// Words for names, descriptions and filter values: case variants, `ß` (upper-cased to `SS`),
/// hyphenated words for `|=`, repeats for `~=`.
const WORDS: &[&str] = &[
    "Pay", "pay", "PAY", "now", "Now", "Save", "item", "Item", "2", "item-2", "Straße", "STRASSE",
    "strasse", "go", "Go", "go-on", "x", "ok",
];

/// ECMAScript whitespace that normalization collapses (tab, newline, U+00A0, U+2003, U+3000,
/// U+FEFF), U+200B and U+00AD that it deletes, U+0085 that it keeps, a hyphen, and nothing.
const SEPARATORS: &[&str] = &[
    " ", " ", " ", "  ", "\t", "\n", "\u{a0}", "\u{2003}", "\u{3000}", "\u{feff}", "\u{200b}",
    "\u{ad}", "\u{85}", "-", "",
];

/// Element tags: ones with implicit roles the selectors ask for, and role-less ones.
const TAGS: &[&str] = &[
    "div", "div", "span", "button", "li", "li", "ul", "ol", "h2", "h3", "a", "option", "section",
    "p",
];

/// Explicit `role` values: the handful the selectors use, a few others, invalid and multi-token
/// ones.
const EXPLICIT_ROLES: &[&str] = &[
    "button",
    "button",
    "checkbox",
    "option",
    "treeitem",
    "heading",
    "listitem",
    "listitem",
    "list",
    "link",
    "radio",
    "switch",
    "tab",
    "row",
    "gridcell",
    "menuitemcheckbox",
    "group",
    "none",
    "bogus",
    "BUTTON",
    "bogus checkbox",
];

const TRISTATES: &[BrowserOsTriState] = &[
    BrowserOsTriState::Unset,
    BrowserOsTriState::False,
    BrowserOsTriState::True,
    BrowserOsTriState::Mixed,
];

/// Draws one random page: `root > html > ...` with explicit DOM ids on every node, so the test
/// knows which nodes are text.
struct PageGen<'r> {
    rng: &'r mut SplitMix64,
    next_dom_id: i32,
    budget: usize,
    text_dom_ids: HashSet<i32>,
}

impl PageGen<'_> {
    const MAX_DEPTH: usize = 6;

    fn take_dom_id(&mut self) -> i32 {
        let id = self.next_dom_id;
        self.next_dom_id += 1;
        id
    }

    fn element(&mut self, depth: usize) -> NodeBuilder {
        self.budget = self.budget.saturating_sub(1);
        let tag = self.rng.pick_str(TAGS);
        let mut node = NodeBuilder::element(tag).dom_id(self.take_dom_id());
        if tag == "a" && self.rng.chance(70) {
            node = node.href("#");
        }
        if self.rng.chance(60) {
            node = node.role(self.rng.pick_str(EXPLICIT_ROLES));
        }
        node = self.states(node);
        let max_children = match depth {
            0..=2 => 4,
            3..=4 => 3,
            _ => 2,
        };
        let children = if depth < Self::MAX_DEPTH {
            self.rng.below(max_children + 1)
        } else {
            0
        };
        for _ in 0..children {
            if self.budget == 0 {
                break;
            }
            node = node.child(self.element(depth + 1));
        }
        if self.rng.chance(15) {
            let id = self.take_dom_id();
            self.text_dom_ids.insert(id);
            node = node.child(NodeBuilder::text("text").dom_id(id));
        }
        node
    }

    fn states(&mut self, mut node: NodeBuilder) -> NodeBuilder {
        if self.rng.chance(35) {
            node =
                node.checked_state(*self.rng.pick(TRISTATES).unwrap_or(&BrowserOsTriState::True));
        }
        if self.rng.chance(30) {
            node =
                node.pressed_state(*self.rng.pick(TRISTATES).unwrap_or(&BrowserOsTriState::True));
        }
        if self.rng.chance(35) {
            node =
                node.expanded_state(*self.rng.pick(TRISTATES).unwrap_or(&BrowserOsTriState::True));
        }
        if self.rng.chance(30) {
            node = node.selected(self.rng.chance(60));
        }
        if self.rng.chance(30) {
            let level = u32::try_from(self.rng.below(5)).unwrap_or(0);
            node = node.level(level);
        }
        if self.rng.chance(8) {
            node = node.contract_disabled(self.rng.chance(70));
        }
        if self.rng.chance(10) {
            node = node.aria_disabled(self.rng.chance(70));
        }
        if self.rng.chance(9) {
            node = node.aria_hidden(self.rng.chance(70));
        }
        if self.rng.chance(5) {
            node = node.visibility_hidden(true);
        }
        node
    }
}

fn random_text(rng: &mut SplitMix64) -> String {
    let mut text = String::new();
    if rng.chance(15) {
        text.push_str(rng.pick_str(SEPARATORS));
    }
    for index in 0..rng.below(4) {
        if index > 0 {
            text.push_str(rng.pick_str(SEPARATORS));
        }
        text.push_str(rng.pick_str(WORDS));
    }
    if rng.chance(15) {
        text.push_str(rng.pick_str(SEPARATORS));
    }
    text
}

fn random_node_text(rng: &mut SplitMix64) -> NodeText {
    let name = random_text(rng);
    let name_hidden = if rng.chance(25) {
        random_text(rng)
    } else {
        name.clone()
    };
    let description = if rng.chance(50) {
        random_text(rng)
    } else {
        String::new()
    };
    let description_hidden = if rng.chance(25) {
        random_text(rng)
    } else {
        description.clone()
    };
    NodeText {
        name,
        name_hidden,
        description,
        description_hidden,
    }
}

fn random_page(rng: &mut SplitMix64) -> Result<Page, Box<dyn Error>> {
    let budget = 15 + rng.below(55);
    let mut generator = PageGen {
        rng,
        next_dom_id: 3,
        budget,
        text_dom_ids: HashSet::new(),
    };
    let mut html = NodeBuilder::element("html").dom_id(2);
    for _ in 0..=generator.rng.below(4) {
        if generator.budget == 0 {
            break;
        }
        html = html.child(generator.element(1));
    }
    let PageGen {
        rng, text_dom_ids, ..
    } = generator;
    let apc = ApcBuilder::new(MAIN_DOC)
        .root(NodeBuilder::root().dom_id(1).child(html))
        .build();
    let mut page = Page::new(
        SnapshotTree::build(apc, BuildOptions::default())?,
        &text_dom_ids,
    )?;

    for &id in &page.elements {
        page.names.0.insert(id, random_node_text(rng));
    }
    for index in 1..=rng.below(6) {
        let target = *rng.pick(&page.elements).ok_or("a page has no elements")?;
        page.register_ref(&format!("e{index}"), RefTarget::Node(target))?;
    }
    page.register_ref(GHOST_REF, RefTarget::Ghost)?;
    Ok(page)
}

const QUERY_ROLES: &[&str] = &[
    "button",
    "button",
    "checkbox",
    "option",
    "treeitem",
    "heading",
    "listitem",
    "listitem",
    "list",
    "link",
    "radio",
    "switch",
    "tab",
    "row",
    "gridcell",
    "menuitemcheckbox",
    "document",
    "group",
    "generic",
    "Button",
    "LISTITEM",
    "nothing",
    "",
];

/// Upstream's role tables for the state attributes (`roleUtils.ts`), so most generated state
/// filters pass validation and reach matching.
const CHECKED_ROLES: &[&str] = &[
    "checkbox",
    "menuitemcheckbox",
    "option",
    "radio",
    "switch",
    "menuitemradio",
    "treeitem",
];
const SELECTED_ROLES: &[&str] = &[
    "gridcell",
    "option",
    "row",
    "tab",
    "rowheader",
    "columnheader",
    "treeitem",
];
const EXPANDED_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "gridcell",
    "link",
    "row",
    "tab",
    "treeitem",
    "menuitemcheckbox",
    "switch",
];
const LEVEL_ROLES: &[&str] = &["heading", "listitem", "row", "treeitem"];

/// Attributes that fail parsing or validation (or, for the last, pass with odd spacing).
const BAD_ATTRIBUTES: &[&str] = &[
    "[foo]",
    "[name]",
    "[description]",
    "[checked=maybe]",
    "[level=abc]",
    "[level>2]",
    "[pressed=yes]",
    "[selected=\"true\"]",
    "[expanded=mixed]",
    "[disabled=1]",
    "[include-hidden=yes]",
    "[name=",
    "[name=/(/]",
    "[ name = \"Pay\" ]",
];

const NAME_OPS: &[&str] = &["=", "=", "=", "*=", "^=", "$=", "|=", "~="];
const REGEX_PATTERNS: &[&str] = &[
    "pay",
    "^pay",
    "now$",
    "p.y",
    "stra(ss|ß)e",
    "\\s",
    "item.2",
    "go go",
    "^$",
    "Pay now",
    "[A-Z]{3}",
    "^\\S+$",
];
const REGEX_FLAGS: &[&str] = &["", "", "i", "i", "iu", "s", "y", "g", "gi", "v", "m", "u"];
const NTH_BODIES: &[&str] = &[
    "0",
    "0",
    "1",
    "2",
    "-1",
    "-1",
    "-2",
    "-3",
    "7",
    "1.5",
    "-0.5",
    "abc",
    "",
    "-0",
    "1e0",
    "0x1",
    "Infinity",
    "-Infinity",
];
const UNKNOWN_REFS: &[&str] = &["e99", "f1e1", "n1", "E1", ""];
const COMBINATORS: &[&str] = &[
    "internal:has",
    "internal:has-not",
    "internal:and",
    "internal:or",
    "internal:chain",
];

/// Draws selector strings over one page, with filter values taken from the page's own names so
/// that filters often match.
struct SelectorGen<'r> {
    rng: &'r mut SplitMix64,
    roles: Vec<&'static str>,
    texts: Vec<String>,
    refs: Vec<String>,
}

impl SelectorGen<'_> {
    const MAX_NESTING: usize = 3;

    fn selector(&mut self, nesting: usize) -> String {
        let len = 1 + self.rng.below(if nesting == 0 { 4 } else { 3 });
        let mut parts: Vec<String> = Vec::with_capacity(len);
        parts.push(self.leading_part());
        while parts.len() < len {
            parts.push(self.part(nesting));
        }
        if self.rng.chance(if nesting == 0 { 15 } else { 8 }) {
            let at = self.rng.below(parts.len());
            if let Some(part) = parts.get_mut(at) {
                part.insert(0, '*');
            }
        }
        parts.join(" >> ")
    }

    /// The parser refuses a combinator as the first part, so a selector opens with anything else.
    fn leading_part(&mut self) -> String {
        match self.rng.below(100) {
            0..=79 => self.role_part(),
            80..=87 => self.ref_part(),
            88..=91 => self.describe_part(),
            _ => self.nth_part(),
        }
    }

    fn part(&mut self, nesting: usize) -> String {
        match self.rng.below(100) {
            0..=34 => self.role_part(),
            35..=46 => self.nth_part(),
            47..=54 => self.ref_part(),
            55..=59 => self.describe_part(),
            // Unquoted nested bodies are malformed: the prelude refuses them, which keeps the
            // skip path exercised.
            60 => format!("{}=role=button", self.rng.pick_str(COMBINATORS)),
            _ if nesting < Self::MAX_NESTING => {
                let name = self.rng.pick_str(COMBINATORS);
                let nested = self.selector(nesting + 1);
                let body = serde_json::to_string(&nested).unwrap_or_default();
                format!("{name}={body}")
            }
            _ => self.role_part(),
        }
    }

    fn role_part(&mut self) -> String {
        let engine = if self.rng.chance(55) {
            "internal:role"
        } else {
            "role"
        };
        let role = if self.rng.chance(65) {
            self.rng.pick_str(&self.roles)
        } else {
            self.rng.pick_str(QUERY_ROLES)
        };
        let mut out = format!("{engine}={role}");
        let attributes = match self.rng.below(20) {
            0..=9 => 0,
            10..=16 => 1,
            17..=18 => 2,
            _ => 3,
        };
        for _ in 0..attributes {
            let attribute = self.attribute(role);
            out.push_str(&attribute);
        }
        out
    }

    fn attribute(&mut self, role: &str) -> String {
        let lower = role.to_lowercase();
        let compatible = |table: &[&str]| table.contains(&lower.as_str());
        match self.rng.below(100) {
            0..=37 => {
                let op = self.rng.pick_str(NAME_OPS);
                let value = self.text_value(op);
                format!("[name{op}{value}]")
            }
            38..=47 => {
                let op = self.rng.pick_str(NAME_OPS);
                let value = self.text_value(op);
                format!("[description{op}{value}]")
            }
            48..=79 => {
                let mut candidates: Vec<&str> = Vec::new();
                let any = self.rng.chance(15);
                if any || compatible(CHECKED_ROLES) {
                    candidates.push("checked");
                }
                if any || lower == "button" {
                    candidates.push("pressed");
                }
                if any || compatible(SELECTED_ROLES) {
                    candidates.push("selected");
                }
                if any || compatible(EXPANDED_ROLES) {
                    candidates.push("expanded");
                }
                if any || compatible(LEVEL_ROLES) {
                    candidates.push("level");
                }
                candidates.push("disabled");
                let name = self.rng.pick(&candidates).copied().unwrap_or("disabled");
                self.state_attribute(name)
            }
            80..=87 => self.state_attribute("disabled"),
            88..=95 => self.state_attribute("include-hidden"),
            _ => self.rng.pick_str(BAD_ATTRIBUTES).to_owned(),
        }
    }

    fn state_attribute(&mut self, name: &str) -> String {
        let value = match name {
            "checked" | "pressed" => {
                self.rng
                    .pick_str(&["", "=true", "=false", "=mixed", "=\"mixed\"", "=false"])
            }
            "level" => self
                .rng
                .pick_str(&["=0", "=1", "=2", "=2", "=3", "=4", "=\"2\"", "=2.0", "=1e0"]),
            _ => self.rng.pick_str(&["", "", "=true", "=false"]),
        };
        format!("[{name}{value}]")
    }

    /// A quoted, unquoted or regex filter value. `~=` gets single words and `|=` hyphen prefixes
    /// often enough to match.
    fn text_value(&mut self, op: &str) -> String {
        if op == "=" && self.rng.chance(12) {
            let pattern = self.rng.pick_str(REGEX_PATTERNS);
            let flags = self.rng.pick_str(REGEX_FLAGS);
            return format!("/{pattern}/{flags}");
        }
        let raw = match op {
            "~=" if self.rng.chance(60) => self.rng.pick_str(WORDS).to_owned(),
            "|=" if self.rng.chance(50) => self.rng.pick_str(&["item", "go", "Item"]).to_owned(),
            _ => self.page_text(),
        };
        let simple = !raw.is_empty() && raw.chars().all(|ch| ch.is_ascii_alphanumeric());
        match self.rng.below(100) {
            0..=9 if simple => raw,
            0..=39 => quote(&raw, ""),
            40..=74 => quote(&raw, "i"),
            _ => quote(&raw, "s"),
        }
    }

    /// One of the page's names or descriptions, often reshaped so that only some filters match.
    fn page_text(&mut self) -> String {
        let text = if self.rng.chance(80) {
            self.rng.pick(&self.texts).cloned().unwrap_or_default()
        } else {
            random_text(self.rng)
        };
        match self.rng.below(100) {
            0..=39 => text,
            40..=54 => text.to_uppercase(),
            55..=64 => text.to_lowercase(),
            65..=79 => text.split_whitespace().next().unwrap_or("").to_owned(),
            _ => {
                let chars: Vec<char> = text.chars().collect();
                let start = self.rng.below(chars.len() + 1);
                let len = self.rng.below(chars.len().saturating_sub(start) + 1);
                chars.iter().skip(start).take(len).collect()
            }
        }
    }

    fn nth_part(&mut self) -> String {
        format!("nth={}", self.rng.pick_str(NTH_BODIES))
    }

    fn ref_part(&mut self) -> String {
        let body = if !self.refs.is_empty() && self.rng.chance(75) {
            self.rng.pick(&self.refs).cloned().unwrap_or_default()
        } else {
            self.rng.pick_str(UNKNOWN_REFS).to_owned()
        };
        format!("aria-ref={body}")
    }

    fn describe_part(&mut self) -> String {
        let word = self.rng.pick_str(WORDS);
        format!(
            "internal:describe={}",
            serde_json::to_string(word).unwrap_or_default()
        )
    }
}

/// A Playwright attribute-selector string literal (`escapeForAttributeSelector`), with an
/// optional `i` or `s` suffix.
fn quote(value: &str, suffix: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"{suffix}")
}

/// A page whose answers are derived by hand from the vendored source:
///
/// ```text
/// Document #1 > html #2
///   button "Go" #10
///   list #20
///     listitem "Parent list" #21
///       list #22
///         listitem "child 1" #23
///         listitem "child 2" #24
///     listitem "Last" #25
///   button "Stop" #30 (aria-hidden)
///   checkbox "Accept" #40 (checked=mixed)
///   h2 "Straße\u{a0}\u{200b}Plan" #41
///   treeitem "Node" #42 (expanded=false, selected)
///   button "Off" / hidden-mode "Off switch" #43 (aria-disabled, description "Turned  off")
/// ```
fn hand_page() -> Result<Page, Box<dyn Error>> {
    let element = |dom: i32, role: &str| NodeBuilder::element("div").dom_id(dom).role(role);
    let html = NodeBuilder::element("html").dom_id(2).children(vec![
        element(10, "button"),
        element(20, "list").children(vec![
            element(21, "listitem").child(
                element(22, "list")
                    .children(vec![element(23, "listitem"), element(24, "listitem")]),
            ),
            element(25, "listitem"),
        ]),
        element(30, "button").aria_hidden(true),
        element(40, "checkbox").checked_state(BrowserOsTriState::Mixed),
        NodeBuilder::element("h2").dom_id(41),
        element(42, "treeitem")
            .expanded_state(BrowserOsTriState::False)
            .selected(true),
        element(43, "button").aria_disabled(true),
    ]);
    let apc = ApcBuilder::new(MAIN_DOC)
        .root(NodeBuilder::root().dom_id(1).child(html))
        .build();
    let mut page = Page::new(
        SnapshotTree::build(apc, BuildOptions::default())?,
        &HashSet::new(),
    )?;
    let plain = |name: &str| NodeText {
        name: name.to_owned(),
        name_hidden: name.to_owned(),
        ..NodeText::default()
    };
    for (dom, text) in [
        (10, plain("Go")),
        (21, plain("Parent list")),
        (23, plain("child 1")),
        (24, plain("child 2")),
        (25, plain("Last")),
        (30, plain("Stop")),
        (40, plain("Accept")),
        (41, plain("Straße\u{a0}\u{200b}Plan")),
        (42, plain("Node")),
        (
            43,
            NodeText {
                name: "Off".to_owned(),
                name_hidden: "Off switch".to_owned(),
                description: "Turned  off".to_owned(),
                description_hidden: "Turned  off".to_owned(),
            },
        ),
    ] {
        let id = page.node_by_dom(dom)?;
        page.names.0.insert(id, text);
    }
    let li1 = page.node_by_dom(21)?;
    page.register_ref("e1", RefTarget::Node(li1))?;
    page.register_ref(GHOST_REF, RefTarget::Ghost)?;
    Ok(page)
}

/// The expected answer of a hand case, by DOM id (`1` is the `Document`).
enum Expected {
    Matched(&'static [i32], Option<&'static str>),
    Failed(&'static str),
}

const HAND_CASES: &[(&str, Expected)] = &[
    // The role engine scans strict descendants, so the scope is never its own match.
    (
        "internal:role=listitem[name=\"Parent list\"i] >> internal:role=listitem",
        Expected::Matched(&[23, 24], None),
    ),
    ("internal:role=button", Expected::Matched(&[10, 43], None)),
    (
        "internal:role=button[include-hidden]",
        Expected::Matched(&[10, 30, 43], None),
    ),
    // `internal:or` sorts the union into document order.
    (
        "internal:role=listitem >> internal:or=\"internal:role=button\"",
        Expected::Matched(&[10, 21, 23, 24, 25, 43], None),
    ),
    // Each root contributes its own last listitem: #20 gives #25, then #22 gives #24.
    (
        "internal:role=list >> internal:chain=\"internal:role=listitem >> nth=-1\"",
        Expected::Matched(&[25, 24], None),
    ),
    // `internal:and` keeps the nested result's order, not the roots'.
    (
        "internal:role=listitem >> internal:and=\"internal:role=list >> internal:chain=\\\"internal:role=listitem >> nth=-1\\\"\"",
        Expected::Matched(&[25, 24], None),
    ),
    (
        "internal:role=list >> internal:chain=\"internal:role=listitem >> nth=-1\" >> internal:or=\"internal:role=button[name=\\\"Go\\\"]\"",
        Expected::Matched(&[10, 24, 25], None),
    ),
    (
        "internal:role=list >> internal:has=\"internal:role=listitem[name=\\\"child 2\\\"s]\"",
        Expected::Matched(&[20, 22], None),
    ),
    (
        "internal:role=list >> internal:has-not=\"internal:role=listitem[name=\\\"Parent\\\"i]\"",
        Expected::Matched(&[22], None),
    ),
    (
        "internal:role=listitem >> nth=-1",
        Expected::Matched(&[25], None),
    ),
    (
        "internal:role=listitem >> nth=-2",
        Expected::Matched(&[24], None),
    ),
    (
        "internal:role=listitem >> nth=1.5",
        Expected::Matched(&[23], None),
    ),
    (
        "internal:role=listitem >> nth=-0.5",
        Expected::Matched(&[], None),
    ),
    (
        "internal:role=listitem >> nth=abc",
        Expected::Matched(&[], None),
    ),
    (
        "*internal:role=list >> internal:role=listitem[name=\"child 1\"]",
        Expected::Matched(&[20, 22], None),
    ),
    (
        "*internal:role=list >> nth=0",
        Expected::Failed("Can't query n-th element in a request with the capture."),
    ),
    (
        "aria-ref=e1 >> internal:role=listitem",
        Expected::Matched(&[23, 24], None),
    ),
    ("aria-ref=e1", Expected::Matched(&[21], None)),
    (
        "internal:role=button >> aria-ref=e1",
        Expected::Matched(&[], None),
    ),
    (
        "internal:role=list >> internal:has=\"aria-ref=e9\"",
        Expected::Matched(&[], Some("e9")),
    ),
    (
        "internal:role=list >> internal:has-not=\"aria-ref=e900\"",
        Expected::Matched(&[20, 22], None),
    ),
    ("role=[bar]", Expected::Failed("Role must not be empty")),
    // The second role part never runs, so its bad body never throws.
    ("role=nothing >> role=[bar]", Expected::Matched(&[], None)),
    ("nth=0", Expected::Matched(&[1], None)),
    // `sortInDOMOrder` gives the `Document` and `html` no common parent: each is a root of its
    // own, visited in insertion order, so the `Document` sorts first only when it came first.
    (
        "internal:role=button[name=\"Go\"] >> internal:or=\"nth=0\"",
        Expected::Matched(&[10, 1], None),
    ),
    (
        "nth=0 >> internal:or=\"internal:role=button[name=\\\"Go\\\"]\"",
        Expected::Matched(&[1, 10], None),
    ),
    ("internal:describe=\"x\"", Expected::Matched(&[], None)),
    (
        "internal:role=button[name=\"Go\"] >> internal:describe=\"x\"",
        Expected::Matched(&[10], None),
    ),
    (
        "role=checkbox[checked=mixed]",
        Expected::Matched(&[40], None),
    ),
    ("role=checkbox[checked]", Expected::Matched(&[], None)),
    ("role=heading[level=2]", Expected::Matched(&[41], None)),
    (
        "role=treeitem[expanded=false]",
        Expected::Matched(&[42], None),
    ),
    // An unset `expanded` differs from `false`.
    ("role=button[expanded=false]", Expected::Matched(&[], None)),
    ("role=treeitem[selected]", Expected::Matched(&[42], None)),
    ("role=button[disabled]", Expected::Matched(&[43], None)),
    (
        "role=button[disabled=false]",
        Expected::Matched(&[10], None),
    ),
    (
        "internal:role=button[name=\"Stop\"][include-hidden]",
        Expected::Matched(&[30], None),
    ),
    (
        "internal:role=button[name=\"Stop\"]",
        Expected::Matched(&[], None),
    ),
    (
        "role=button[name=\"Off switch\"][include-hidden]",
        Expected::Matched(&[43], None),
    ),
    (
        "role=button[name=\"Off switch\"]",
        Expected::Matched(&[], None),
    ),
    (
        "role=button[description=\"turned off\"i]",
        Expected::Matched(&[43], None),
    ),
    // U+200B is deleted, U+00A0 collapses to a space, and `ß` upper-cases to `SS`.
    (
        "role=heading[name=\"STRASSE plan\"i]",
        Expected::Matched(&[41], None),
    ),
    (
        "role=button[checked]",
        Expected::Failed(
            "\"checked\" attribute is only supported for roles: \"checkbox\", \"menuitemcheckbox\", \"menuitemradio\", \"option\", \"radio\", \"switch\", \"treeitem\"",
        ),
    ),
];

/// Proves the harness before it judges Rust: Neo's bundle, with the glue and the mock DOM,
/// answers what the vendored source says, case by case. `TreeStage` must give the same answers,
/// since several cases pin orders the random generator rarely builds, such as an `internal:and`
/// whose nested result is out of document order.
#[test]
fn oracle_and_tree_stage_answer_hand_derived_cases() -> Result<(), Box<dyn Error>> {
    let oracle = Oracle::load()?;
    let engine = oracle.eval_json(
        r#"JSON.stringify(["ß".toUpperCase(), "﻿ x　".trim(), /\s/.test("\u0085"), /\s/.test("﻿"), typeof WeakRef])"#,
    )?;
    assert_eq!(
        engine,
        json!(["SS", "x", false, true, "function"]),
        "QuickJS must agree with V8 on the string semantics role filters use, and have WeakRef for RefMap"
    );

    let page = hand_page()?;
    let selectors: Vec<String> = HAND_CASES
        .iter()
        .map(|(selector, _)| (*selector).to_owned())
        .collect();
    let answers = oracle.run(&page, &selectors)?;
    for ((selector, expected), answer) in HAND_CASES.iter().zip(answers) {
        let expected = match expected {
            Expected::Matched(doms, stale) => {
                let mut ids = Vec::with_capacity(doms.len());
                for &dom in *doms {
                    ids.push(page.node_by_dom(dom)?.raw());
                }
                Outcome::matched(ids, *stale)
            }
            Expected::Failed(message) => Outcome::Failed((*message).to_owned()),
        };
        let (chunk, actual) = match answer {
            OracleAnswer::Ok {
                chunk,
                ids,
                stale_ref,
            } => (chunk, Outcome::Matched { ids, stale_ref }),
            OracleAnswer::Error { chunk, message } => (chunk, Outcome::Failed(message)),
            OracleAnswer::Skip { message } | OracleAnswer::Harness { message } => {
                return Err(format!("hand case {selector:?} did not run: {message}").into());
            }
        };
        assert_eq!(
            actual, expected,
            "the oracle must answer hand case {selector:?} as the vendored source does (node ids are arena ids)"
        );
        assert_eq!(
            rust_outcome(&page, &chunk)?,
            expected,
            "TreeStage must answer hand case {selector:?} as the vendored source does"
        );
    }
    Ok(())
}

/// Runs thousands of random selectors over hundreds of random single-document pages through both
/// `TreeStage` and the shipped query code, and requires the same matches, in the same order, the
/// same stale ref, and the same error text.
#[test]
fn tree_stage_matches_shipped_query_code_on_random_pages() -> Result<(), Box<dyn Error>> {
    let oracle = Oracle::load()?;
    let mut tally = Tally::default();
    let mut report = String::new();
    for tree_index in 0..TREES {
        let seed = SEED.wrapping_add(tree_index);
        let mut rng = SplitMix64(seed);
        let page = random_page(&mut rng)?;
        let mut texts: Vec<String> = page
            .names
            .0
            .values()
            .flat_map(|text| [text.name.clone(), text.description.clone()])
            .filter(|text| !text.is_empty())
            .collect();
        texts.sort();
        let mut roles: Vec<&'static str> = page
            .elements
            .iter()
            .filter_map(|&id| page.tree.node(id)?.role().map(Role::as_str))
            .collect();
        roles.sort_unstable();
        roles.dedup();
        let mut generator = SelectorGen {
            rng: &mut rng,
            roles,
            texts,
            refs: page
                .refs
                .targets
                .iter()
                .map(|(body, _)| body.clone())
                .collect(),
        };
        let selectors: Vec<String> = (0..SELECTORS_PER_TREE)
            .map(|_| generator.selector(0))
            .collect();
        let answers = oracle.run(&page, &selectors)?;
        for (selector, answer) in selectors.iter().zip(answers) {
            let (chunk, expected) = match answer {
                OracleAnswer::Skip { .. } => {
                    tally.skipped_by_prelude += 1;
                    continue;
                }
                OracleAnswer::Harness { message } => {
                    return Err(
                        format!("the oracle harness failed on {selector:?}: {message}").into(),
                    );
                }
                OracleAnswer::Ok {
                    chunk,
                    ids,
                    stale_ref,
                } => (chunk, Outcome::Matched { ids, stale_ref }),
                OracleAnswer::Error { chunk, message } => (chunk, Outcome::Failed(message)),
            };
            tally.compared += 1;
            match &expected {
                Outcome::Failed(_) => tally.errors_compared += 1,
                Outcome::Matched { ids, stale_ref } => {
                    tally.with_matches += usize::from(!ids.is_empty());
                    tally.stale_compared += usize::from(stale_ref.is_some());
                }
            }
            let actual = rust_outcome(&page, &chunk);
            let agrees = actual
                .as_ref()
                .is_ok_and(|actual| actual.clone().comparable() == expected.clone().comparable());
            if !agrees {
                tally.mismatches += 1;
                if tally.mismatches <= DETAILED_MISMATCHES {
                    let _ = write!(
                        report,
                        "\n--- mismatch {}: seed {seed:#x} (tree {tree_index})\n  selector: {selector:?}\n  chunk:    {chunk}\n  rust:     {actual:?}\n  oracle:   {expected:?}\n  page:\n{}",
                        tally.mismatches,
                        page.dump(),
                    );
                }
            }
        }
    }

    #[expect(
        clippy::print_stderr,
        reason = "the counts show a reviewer what the differential compared, also when it passes"
    )]
    {
        eprintln!("select_match_oracle: {tally:?}");
    }
    assert!(
        tally.mismatches == 0,
        "TreeStage must answer every chunk as the shipped query code does; {tally:?}{report}"
    );
    assert!(
        tally.compared >= 8_000
            && tally.errors_compared >= 300
            && tally.with_matches >= 1_500
            && tally.stale_compared >= 100,
        "the generator must keep exercising matches, errors and stale refs, or a pass proves little: {tally:?}"
    );
    Ok(())
}
