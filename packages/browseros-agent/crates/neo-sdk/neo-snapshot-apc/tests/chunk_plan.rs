//! `ChunkPlan` over chunks the real prelude builds: how parts are classified and cut into maximal
//! runs, the page form of a run at every depth, the capture rewrite, the lazy nth-with-capture
//! failure, the engine check, and an all-tree chunk selecting as its tree stage does.

#[path = "support/prelude_chunks.rs"]
#[expect(
    dead_code,
    reason = "a plan prints no locator, so the helper's locator text goes unused in this file"
)]
mod prelude_chunks;

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use neo_snapshot_apc::select::{
    ChunkPlan, ChunkPlanError, ParsedSelector, PlannedStage, RefResolver, Scope, SelectorBody,
    TreeStage, TreeStageError,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocId, DocToken, DomNodeId, NodeBuilder, NodeId, NodeKey,
    SnapshotTree,
};
use prelude_chunks::Prelude;
use serde_json::{Value, json};

const MAIN: &str = "doc-main";

/// The tab's ref table, standing in for `TabRefTable` (engine chunk 5).
#[derive(Default)]
struct Refs(HashMap<String, NodeKey>);

impl RefResolver for Refs {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        self.0.get(body).cloned()
    }
}

fn el(tag: &str, dom: i32) -> NodeBuilder {
    NodeBuilder::element(tag).dom_id(dom)
}

fn button(dom: i32, label: &str) -> NodeBuilder {
    el("button", dom).child(NodeBuilder::text(label))
}

fn key(dom: i32) -> Result<NodeKey, Box<dyn Error>> {
    let token = DocToken::new(MAIN).ok_or("a test document token is never empty")?;
    Ok(NodeKey::new(token, DomNodeId::new(dom)))
}

/// A list of three items, two of them holding a button and a link, then a heading.
fn tree() -> Result<SnapshotTree, Box<dyn Error>> {
    let apc = ApcBuilder::new(MAIN)
        .root(NodeBuilder::root().children(vec![
            el("ul", 10).children(vec![
                el("li", 11).children(vec![button(12, "Edit"), el("a", 13).role("link")]),
                el("li", 14).child(el("ul", 15).child(el("li", 16).child(button(17, "Go")))),
                el("li", 18).child(el("a", 19).role("link")),
            ]),
            el("h2", 20).child(NodeBuilder::text("Title")),
        ]))
        .build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn node(tree: &SnapshotTree, dom: i32) -> Result<NodeId, Box<dyn Error>> {
    Ok(tree
        .node_by_key(&key(dom)?)
        .ok_or_else(|| format!("no node {dom}"))?
        .id())
}

fn document(tree: &SnapshotTree) -> Result<Scope<'static>, Box<dyn Error>> {
    Scope::document(tree, DocId::MAIN).ok_or_else(|| "no main document root".into())
}

/// `selector` as the vendored parser reads it: a selector string through `page.locator`.
fn parsed(js: &Prelude, selector: &str) -> Result<ParsedSelector, Box<dyn Error>> {
    js.chunk(&format!(
        "page.locator({})",
        serde_json::to_string(selector)?
    ))
}

/// One line per stage, `tree` or `page`, then the selector its page form sends: each part as
/// `name=body`, a nested body in parentheses, a CSS part as its source, and a substituted leaf as
/// `engine=index<the leaf's source>`.
fn shape(plan: &ChunkPlan) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(plan
        .stages()?
        .iter()
        .map(|stage| match stage {
            PlannedStage::Tree(run) => format!("tree {}", render(run.page_form().selector())),
            PlannedStage::Page(form) => format!("page {}", render(form.selector())),
        })
        .collect())
}

fn render(selector: &ParsedSelector) -> String {
    let parts: Vec<String> = selector
        .parts
        .iter()
        .map(|part| match &part.body {
            SelectorBody::Text(body) if part.name.starts_with("internal:neo-") => {
                format!("{}={body}<{}>", part.name, part.source)
            }
            SelectorBody::Text(body) => format!("{}={body}", part.name),
            SelectorBody::Nested(nested) => format!("{}=({})", part.name, render(&nested.parsed)),
            SelectorBody::Css(_) => format!("css={}", part.source),
        })
        .collect();
    parts.join(" >> ")
}

#[test]
fn classifies_each_part_and_cuts_maximal_runs() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let cases: [(&str, &[&str], &str); 14] = [
        (
            "page.getByRole('button')",
            &["tree internal:neo-below=0<button>"],
            "a role leaf is a tree part",
        ),
        (
            "page.locator('role=button')",
            &["tree internal:neo-below=0<button>"],
            "the public role engine is a tree leaf too",
        ),
        (
            "page.locator('aria-ref=e5')",
            &["tree internal:neo-inside=0<e5>"],
            "a ref is a tree leaf, whose engine keeps the root itself",
        ),
        (
            "page.locator('div')",
            &["page css=div"],
            "CSS is the page's",
        ),
        (
            "page.getByRole('row').filter({ hasText: 'Pro' }).getByRole('button')",
            &[
                "tree internal:neo-below=0<row>",
                "page internal:has-text=\"Pro\"i",
                "tree internal:neo-below=0<button>",
            ],
            "the sides alternate, and each run counts its candidates from 0",
        ),
        (
            "page.getByRole('list').filter({ has: page.getByRole('link') })",
            &["tree internal:neo-below=0<list> >> internal:has=(internal:neo-below=1<link>)"],
            "a combinator whose nested selector is all tree joins the tree run",
        ),
        (
            "page.locator('li').filter({ has: page.locator('a').getByRole('img') })",
            &["page css=li >> internal:has=(css=a >> internal:neo-below=0<img>)"],
            "a combinator whose nested selector mixes the sides is the page's, its leaf substituted",
        ),
        (
            "page.getByRole('button').nth(1).locator('span')",
            &[
                "tree internal:neo-below=0<button> >> nth=1",
                "page css=span",
            ],
            "nth joins the run in progress",
        ),
        (
            "page.locator('nth=0 >> div')",
            &["page nth=0 >> css=div"],
            "a neutral before any sided part joins the first run",
        ),
        (
            "page.locator('nth=0').describe('first')",
            &["tree nth=0 >> internal:describe=\"first\""],
            "a chunk of only neutrals is one tree run",
        ),
        (
            "page.getByRole('button').or(page.locator('internal:text=\"Go\"i'))",
            &[
                "tree internal:neo-below=0<button>",
                "page internal:or=(internal:text=\"Go\"i)",
            ],
            "an or over a page selector is the page's",
        ),
        (
            "page.locator('internal:testid=[data-testid=\"rows\"s]').getByRole('cell').nth(2)",
            &[
                "page internal:testid=[data-testid=\"rows\"s]",
                "tree internal:neo-below=0<cell> >> nth=2",
            ],
            "a test id is the page's, and nth stays with the role run",
        ),
        (
            "page.locator('tr').filter({ hasNot: page.getByRole('button') })",
            &[
                "page css=tr",
                "tree internal:has-not=(internal:neo-below=0<button>)",
            ],
            "has-not over a tree selector is a tree part",
        ),
        (
            "page.locator('div').locator(page.getByRole('button'))",
            &[
                "page css=div",
                "tree internal:chain=(internal:neo-below=0<button>)",
            ],
            "a chain into a tree selector is a tree part",
        ),
    ];
    for (locator, expected, why) in cases {
        let plan = ChunkPlan::of(&js.chunk(locator)?)?;
        assert_eq!(shape(&plan)?, expected, "{locator}: {why}");
    }
    Ok(())
}

#[test]
fn substitutes_every_tree_leaf_at_every_depth() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let chunk = parsed(
        &js,
        "li >> internal:has=\"span >> internal:role=button\" \
         >> internal:has-not=\"em >> aria-ref=e1\" \
         >> internal:and=\"li >> internal:role=link\" \
         >> internal:or=\"b >> internal:role=heading\" \
         >> internal:chain=\"i >> internal:has=\\\"internal:role=listitem\\\"\"",
    )?;
    let plan = ChunkPlan::of(&chunk)?;
    let stages = plan.stages()?;
    let PlannedStage::Page(form) = stages.first() else {
        return Err("every combinator mixes the sides, so the chunk is one page run".into());
    };
    assert!(stages.rest().is_empty(), "one maximal run");
    let below = |index: &str, source: &str| json!({ "name": "internal:neo-below", "body": index, "source": source });
    let css = |source: &str| json!({ "name": "css", "body": [], "source": source });
    let sent = serde_json::to_value(form.selector())?;
    let parts = sent["parts"].as_array().ok_or("a page form has parts")?;
    assert_eq!(parts.len(), 6, "a leaf is replaced, never removed");
    assert_eq!(parts[0]["name"], "css", "a page part stays as it is");
    let nested = |index: usize| -> Result<Value, Box<dyn Error>> {
        let parsed = &parts[index]["body"]["parsed"]["parts"];
        let mut parsed = parsed.clone();
        // CSS bodies are the parser's AST; the source names them well enough here.
        for part in parsed.as_array_mut().ok_or("a nested body has parts")? {
            if part["name"] == "css" {
                part["body"] = json!([]);
            }
        }
        Ok(parsed)
    };
    assert_eq!(
        nested(1)?,
        json!([css("span"), below("0", "button")]),
        "inside has"
    );
    assert_eq!(
        nested(2)?,
        json!([
            css("em"),
            { "name": "internal:neo-inside", "body": "1", "source": "e1" }
        ]),
        "inside has-not, a ref becomes the inclusive engine"
    );
    assert_eq!(
        nested(3)?,
        json!([css("li"), below("2", "link")]),
        "inside and"
    );
    assert_eq!(
        nested(4)?,
        json!([css("b"), below("3", "heading")]),
        "inside or"
    );
    let chain = nested(5)?;
    assert_eq!(chain[0], css("i"), "inside chain");
    assert_eq!(
        chain[1]["name"], "internal:has",
        "an all-tree combinator under a page part stays a combinator, for the page's loop"
    );
    assert_eq!(
        chain[1]["body"]["parsed"]["parts"],
        json!([below("4", "listitem")]),
        "two levels down, in upstream's visiting order"
    );

    // Each candidate is its leaf alone, over the whole document.
    let tree = tree()?;
    let mut refs = Refs::default();
    refs.0.insert("e1".to_owned(), key(13)?);
    let names = tree.name_cache();
    let expected: [(&[i32], &str); 5] = [
        (&[12, 17], "candidate 0 is role=button"),
        (&[13], "candidate 1 is aria-ref=e1"),
        (&[13, 19], "candidate 2 is role=link"),
        (&[20], "candidate 3 is role=heading"),
        (&[11, 14, 16, 18], "candidate 4 is role=listitem"),
    ];
    assert_eq!(form.candidates().len(), expected.len(), "one per leaf");
    for (candidate, (doms, why)) in form.candidates().iter().zip(expected) {
        let matches = candidate.select(&tree, document(&tree)?, &names, &refs)?;
        let nodes: Result<Vec<NodeId>, _> = doms.iter().map(|&dom| node(&tree, dom)).collect();
        assert_eq!(matches.nodes(), nodes?, "{why}");
    }
    Ok(())
}

#[test]
fn a_tree_run_keeps_a_page_form_with_its_own_candidates() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let plan = ChunkPlan::of(&js.chunk(
        "page.getByRole('listitem').filter({ has: page.locator('aria-ref=e1') }).first()",
    )?)?;
    let stages = plan.stages()?;
    let PlannedStage::Tree(run) = stages.first() else {
        return Err("an all-tree chunk is one tree run".into());
    };
    assert_eq!(
        render(run.page_form().selector()),
        "internal:neo-below=0<listitem> >> internal:has=(internal:neo-inside=1<e1>) >> nth=0",
        "the fallback a set of roots with no tree node runs"
    );
    assert_eq!(run.page_form().candidates().len(), 2, "one per leaf");
    Ok(())
}

#[test]
fn a_top_level_capture_is_rewritten_as_upstream_runs_it() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let cases: [(&str, &[&str], Option<&str>, &str); 3] = [
        (
            "*css=div >> internal:role=button",
            &[
                "page css=div",
                "tree internal:has=(internal:neo-below=0<button>)",
            ],
            Some("internal:role=button"),
            "the captured part's elements, kept when the rest matches inside them",
        ),
        (
            "*internal:role=list >> li",
            &[
                "tree internal:neo-below=0<list>",
                "page internal:has=(css=li)",
            ],
            Some("li"),
            "the rest is classified as the has it becomes",
        ),
        (
            "div >> *internal:role=list",
            &["page css=div", "tree internal:neo-below=0<list>"],
            None,
            "a capture on the last part adds no has",
        ),
    ];
    for (selector, expected, has_source, why) in cases {
        let chunk = parsed(&js, selector)?;
        assert!(
            chunk.capture.is_some(),
            "{selector}: the parser marks the capture"
        );
        let plan = ChunkPlan::of(&chunk)?;
        assert_eq!(shape(&plan)?, expected, "{selector}: {why}");
        let last = plan.stages()?.iter().last().ok_or("a plan has a stage")?;
        let form = match last {
            PlannedStage::Tree(run) => run.page_form(),
            PlannedStage::Page(form) => form,
        };
        let source = form
            .selector()
            .parts
            .last()
            .filter(|part| part.name == "internal:has")
            .map(|part| part.source.as_str());
        assert_eq!(
            source, has_source,
            "{selector}: the has part's source is upstream's stringifySelector of the rest"
        );
        assert!(
            form.selector().capture.is_none(),
            "{selector}: the rewrite leaves no top-level capture"
        );
    }
    assert_eq!(
        plan_of(&js, "*internal:role=list >> internal:role=listitem")?
            .stages()?
            .rest()
            .len(),
        0,
        "an all-tree capture stays one tree run"
    );
    Ok(())
}

fn plan_of(js: &Prelude, selector: &str) -> Result<ChunkPlan, Box<dyn Error>> {
    Ok(ChunkPlan::of(&parsed(js, selector)?)?)
}

#[test]
fn nth_with_a_capture_fails_only_once_the_plan_runs() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let plan = plan_of(&js, "*internal:role=list >> nth=0")?;
    let error = plan
        .stages()
        .err()
        .ok_or("upstream throws when the chunk runs")?;
    assert_eq!(error, TreeStageError::NthWithCapture, "the run's error");
    assert_eq!(
        error.to_string(),
        "Can't query n-th element in a request with the capture.",
        "upstream's text"
    );

    let nested = plan_of(&js, "li >> internal:has=\"*css=span >> nth=0\"")?;
    let stages = nested
        .stages()
        .map_err(|error| format!("a nested capture is its own selector's business: {error}"))?;
    let PlannedStage::Page(form) = stages.first() else {
        return Err("li and a has over CSS are the page's".into());
    };
    let has = form.selector().parts.get(1).ok_or("the has part")?;
    let SelectorBody::Nested(body) = &has.body else {
        return Err("a has part is nested".into());
    };
    assert_eq!(
        body.parsed.capture,
        Some(0),
        "the nested capture reaches the page, whose querySelectorAll throws once a root reaches it"
    );
    Ok(())
}

#[test]
fn refuses_an_engine_the_page_lacks_with_the_probes_words() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let refused = |selector: &str| -> Result<String, Box<dyn Error>> {
        Ok(ChunkPlan::of(&parsed(&js, selector)?)
            .err()
            .ok_or_else(|| format!("{selector} must be refused"))?
            .to_string())
    };
    for name in [
        "internal:neo-from",
        "internal:neo-below",
        "internal:neo-inside",
    ] {
        assert_eq!(
            refused(&format!("{name}=0"))?,
            format!("Unknown engine \"{name}\" while parsing selector {name}=0"),
            "agent input never reaches a call-scoped engine"
        );
    }
    assert_eq!(
        refused("li >> internal:has=\"internal:neo-inside=0\"")?,
        "Unknown engine \"internal:neo-inside\" while parsing selector \
         li >> internal:has=\"internal:neo-inside=0\"",
        "at any depth, and the message prints the whole chunk"
    );
    assert_eq!(
        refused("bogus=1")?,
        "Unknown engine \"bogus\" while parsing selector bogus=1",
        "upstream's text, as the probe's test records it"
    );
    assert_eq!(
        refused("*bogus=1 >> div")?,
        "Unknown engine \"bogus\" while parsing selector *bogus=1 >> div",
        "the chunk prints with its capture"
    );
    assert_eq!(
        refused("div >> left-of=\"span\"")?,
        "Unknown engine \"left-of\" while parsing selector div >> left-of=\"span\"",
        "upstream's InjectedScript has no layout engine, so the probe refuses one today"
    );
    let hint = "Unknown selector engine \"ref\". Use page.getByRef('e5') or 'aria-ref=e5'.";
    assert_eq!(
        refused("ref=e5")?,
        hint,
        "Neo's hint, as the probe words it"
    );
    assert_eq!(refused("ref=\"e5\"")?, hint, "the body is unquoted");
    assert_eq!(
        refused("button >> internal:has=\"ref=e5\"")?,
        hint,
        "nested too"
    );
    assert_eq!(
        refused("ref=e5 >> bogus=1")?,
        hint,
        "the first unknown part in visiting order wins"
    );
    assert_eq!(
        refused("bogus=1 >> ref=e5")?,
        "Unknown engine \"bogus\" while parsing selector bogus=1 >> ref=e5",
        "the first unknown part in visiting order wins"
    );
    let error = ChunkPlan::of(&parsed(&js, "bogus=1")?)
        .err()
        .ok_or("bogus is refused")?;
    assert_eq!(
        error,
        ChunkPlanError::UnknownEngine {
            name: "bogus".to_owned(),
            selector: "bogus=1".to_owned(),
        },
        "the error names the engine"
    );
    Ok(())
}

/// The names the vendored `InjectedScript` constructor registers (`this._engines.set('…'`).
fn vendored_engine_names() -> Result<Vec<String>, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts");
    let source = fs::read_to_string(path)?;
    let marker = "this._engines.set('";
    Ok(source
        .split(marker)
        .skip(1)
        .filter_map(|rest| rest.split_once('\'').map(|(name, _)| name.to_owned()))
        .collect())
}

#[test]
fn accepts_every_engine_the_vendored_injected_script_registers() -> Result<(), Box<dyn Error>> {
    let names = vendored_engine_names()?;
    assert_eq!(
        names.len(),
        32,
        "chunk_plan.rs lists 32 engines; a vendored Playwright that changes the table changes it"
    );
    for name in &names {
        let body = if name == "css" {
            json!([])
        } else if name.starts_with("internal:")
            && ["has", "has-not", "and", "or", "chain"]
                .iter()
                .any(|combinator| name == &format!("internal:{combinator}"))
        {
            json!({ "parsed": { "parts": [{ "name": "css", "body": [], "source": "x" }] } })
        } else {
            json!("x")
        };
        let chunk: ParsedSelector = serde_json::from_value(
            json!({ "parts": [{ "name": name, "body": body, "source": "x" }] }),
        )?;
        assert!(
            ChunkPlan::of(&chunk).is_ok(),
            "{name} is an engine of the page's table, so planning accepts it"
        );
    }
    Ok(())
}

#[test]
fn an_all_tree_chunk_is_one_tree_run_that_selects_as_its_tree_stage() -> Result<(), Box<dyn Error>>
{
    let js = Prelude::load()?;
    let tree = tree()?;
    let mut refs = Refs::default();
    refs.0.insert("e1".to_owned(), key(13)?);
    let names = tree.name_cache();
    let chunks = [
        js.chunk(
            "page.getByRole('list').filter({ has: page.getByRole('link') }).getByRole('listitem').nth(1)",
        )?,
        js.chunk("page.getByRole('listitem').filter({ hasNot: page.getByRole('button') })")?,
        js.chunk("page.getByRole('button').or(page.getByRole('link')).last()")?,
        js.chunk("page.getByRole('link').and(page.locator('aria-ref=e1'))")?,
        js.chunk("page.getByRole('listitem').filter({ has: page.locator('aria-ref=e1') })")?,
        parsed(&js, "*internal:role=list >> internal:role=link")?,
    ];
    for chunk in chunks {
        let plan = ChunkPlan::of(&chunk)?;
        let stages = plan.stages()?;
        let (PlannedStage::Tree(run), []) = (stages.first(), stages.rest()) else {
            return Err(format!("{}: an all-tree chunk is one tree run", render(&chunk)).into());
        };
        let planned = run.stage().select(&tree, document(&tree)?, &names, &refs)?;
        let whole = TreeStage::new(&chunk)?.select(&tree, document(&tree)?, &names, &refs)?;
        assert_eq!(
            planned,
            whole,
            "{}: the plan's one tree run selects what the chunk's own tree stage selects",
            render(&chunk)
        );
        assert!(
            !planned.is_empty(),
            "{}: the case matches something",
            render(&chunk)
        );
    }
    Ok(())
}

#[test]
fn a_chunk_with_no_sided_part_runs_as_one_tree_run() -> Result<(), Box<dyn Error>> {
    let tree = tree()?;
    let names = tree.name_cache();
    let refs = Refs::default();
    let empty = ParsedSelector {
        parts: Vec::new(),
        capture: None,
    };
    let plan = ChunkPlan::of(&empty)?;
    let stages = plan.stages()?;
    let PlannedStage::Tree(run) = stages.first() else {
        return Err("an empty chunk is one tree run".into());
    };
    assert!(stages.rest().is_empty(), "and only that");
    let Scope::Root(root) = document(&tree)? else {
        return Err("a document scope is a root".into());
    };
    assert_eq!(
        run.stage()
            .select(&tree, Scope::Root(root), &names, &refs)?
            .nodes(),
        [root],
        "upstream's loop over no parts returns the query root"
    );
    assert_eq!(
        plan.chunk(),
        &empty,
        "the plan keeps the chunk it was made from"
    );
    Ok(())
}
