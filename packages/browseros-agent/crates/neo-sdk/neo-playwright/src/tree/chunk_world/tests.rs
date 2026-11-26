//! `ChunkWorld` over a scripted tab whose captures `TabTree::capture` takes, so design 6.2's
//! state (each world's install time, each document's binding in `DocTable`) is the real one: the
//! start's two proofs, the positional handover of a call's nodes, a node or a world gone, the pin
//! and the keys a run asks for, and a mixed chunk run end to end.

use std::time::Duration;

use neo_snapshot_apc::refs::RefLookup;
use neo_snapshot_apc::select::{ChunkPlan, ParsedSelector};
use neo_snapshot_apc::tree::{
    ApcBuilder, DocId, DocToken, MIN_CONTRACT_VERSION, NodeBuilder, NodeId, SnapshotTree, TreeDoc,
};
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeFailure, ProbeReply, ProbeRequest, SetMember};
use crate::resolve::{ChunkOutcome, ChunkPage, ChunkSet, First};
use crate::target::{CallHeader, Strictness};
use crate::test_support::{SCRIPTED_CHILD, ScriptedRun, ScriptedTab, TestResult, count_op};
use crate::tree::tree_script::{ChildFrameSpec, TreeScript, apc_reply, encode_apc_reply, key};
use crate::tree::{
    AttemptStart, ChunkWorld, RunAttempt, RunRoot, TabOriginState, TabTree, Unproven,
};
use crate::world::{DocLookup, Pin, Place};

const DOC: &str = "doc-main";
const CHILD_DOC: &str = "doc-child";
/// The card page: a card holding the Pay button, and a Back button outside it.
const CARD: i32 = 30;
const PAY: i32 = 31;
const BACK: i32 = 40;

fn card_page() -> Value {
    let card = NodeBuilder::element("div").dom_id(CARD).child(
        NodeBuilder::element("button")
            .dom_id(PAY)
            .child(NodeBuilder::text("Pay")),
    );
    let back = NodeBuilder::element("button")
        .dom_id(BACK)
        .child(NodeBuilder::text("Back"));
    apc_reply(
        &ApcBuilder::new(DOC)
            .root(NodeBuilder::root().dom_id(1).children(vec![card, back]))
            .build(),
    )
}

/// A main document with the nodes 10 and 20, and a same-process child frame whose document
/// holds the node 33.
fn framed_page() -> Value {
    encode_apc_reply(
        DOC,
        Some("https://a.test/"),
        &[10, 20],
        Some(&ChildFrameSpec {
            iframe_dom_id: 500,
            doc_token: CHILD_DOC,
            frame_token: SCRIPTED_CHILD,
            url: Some("https://a.test/child"),
            child_dom_ids: &[33],
        }),
        Some(MIN_CONTRACT_VERSION),
    )
}

fn node_in(tree: &SnapshotTree, doc: &str, dom: i32) -> TestResult<NodeId> {
    Ok(tree
        .node_by_key(&key(doc, dom)?)
        .ok_or("the capture holds the node")?
        .id())
}

fn node(tree: &SnapshotTree, dom: i32) -> TestResult<NodeId> {
    node_in(tree, DOC, dom)
}

fn token(doc: &str) -> TestResult<DocToken> {
    Ok(DocToken::new(doc).ok_or("a test token is not empty")?)
}

/// A `describePins` of `count` call nodes: a request whose nodes the page reads positionally.
fn describe(count: u32) -> ProbeRequest {
    ProbeRequest::DescribePins {
        members: (0..count).map(SetMember::Node).collect(),
        chunk: None,
    }
}

/// `probe::pinned_nodes`'s deep-serialized reply: each pin's node by backend id, `None` for a pin
/// the probe evicted.
fn pinned(nodes: &[Option<i32>]) -> Value {
    let value: Vec<Value> = nodes
        .iter()
        .map(|node| match node {
            Some(id) => json!({ "type": "node", "value": { "backendNodeId": id } }),
            None => json!({ "type": "null" }),
        })
        .collect();
    json!({ "result": { "type": "object",
        "deepSerializedValue": { "type": "array", "value": value } } })
}

/// The ops of the probe requests sent after the first `from`.
fn ops_since(tab: &ScriptedTab<TreeScript>, from: usize) -> Vec<String> {
    tab.script()
        .probe_calls()
        .into_iter()
        .skip(from)
        .map(|sent| sent.op)
        .collect()
}

fn plan(parts: &[Value]) -> TestResult<ChunkPlan> {
    let parsed: ParsedSelector = serde_json::from_value(json!({ "parts": parts }))?;
    Ok(ChunkPlan::of(&parsed)?)
}

fn part(name: &str, body: &str) -> Value {
    json!({ "name": name, "body": body, "source": body })
}

fn assert_send<T: Send>(_: &T) {}

#[tokio::test]
async fn start_takes_a_world_installed_before_the_capture_and_its_frames_document() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(framed_page()).with_child());
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let before = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let child = captured
        .tree()
        .document_by_token(&token(CHILD_DOC)?)
        .ok_or("the capture holds the child document")?
        .id();
    let child_node = node_in(captured.tree(), CHILD_DOC, 33)?;

    assert!(
        ChunkWorld::start(&attempt, before.clone(), DocId::MAIN, RunRoot::Document).is_ok(),
        "a world installed before the capture, over its frame's document, runs"
    );
    assert_eq!(
        ChunkWorld::start(&attempt, before.clone(), child, RunRoot::Document).err(),
        Some(Unproven::DocumentNotInFrame),
        "the child frame's document is not the main frame's world's (case 4)"
    );
    let handle = RunRoot::Handle {
        object: "handle-node".to_owned(),
        node: Some(child_node),
    };
    assert_eq!(
        ChunkWorld::start(&attempt, before.clone(), DocId::MAIN, handle).err(),
        Some(Unproven::DocumentNotInFrame),
        "a handle whose node is in another document cannot root this document's run"
    );

    page.frames().invalidate(&before);
    let after = page.world(&page.main_endpoint(), Place::Main).await?;
    assert_eq!(
        ChunkWorld::start(&attempt, after, DocId::MAIN, RunRoot::Document).err(),
        Some(Unproven::WorldAfterCapture),
        "a world installed after the capture request proves nothing about what it saw (case 3)"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_call_hands_its_nodes_over_in_order_after_the_root_eight_resolves_at_a_time() -> TestResult
{
    let ids: Vec<i32> = (101..=120).collect();
    let divs = ids
        .iter()
        .map(|&id| NodeBuilder::element("div").dom_id(id))
        .collect();
    let apc = ApcBuilder::new(DOC)
        .root(NodeBuilder::root().dom_id(1).children(divs))
        .build();
    let tab = ScriptedTab::new(TreeScript::new(apc_reply(&apc)));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let nodes: Vec<NodeId> = ids
        .iter()
        .map(|&id| node(captured.tree(), id))
        .collect::<TestResult<_>>()?;
    for &id in &ids {
        // The last node answers first: objects taken as their resolves answer would arrive
        // reversed.
        tab.script()
            .delay_resolve_node(id, Duration::from_millis(u64::try_from(121 - id)?));
    }
    let ctx = world.context().0;
    let object = |id: i32| json!({ "objectId": format!("node-{ctx}-{id}") });

    let (_run, mut side) =
        ChunkWorld::start(&attempt, world.clone(), DocId::MAIN, RunRoot::Document)
            .map_err(|unproven| format!("{unproven:?}"))?;
    let from = tab.script().probe_calls().len();
    tab.script()
        .push_probe_reply(json!({ "kind": "pinsDescribed", "items": [] }));
    let reply = side.call(&describe(20), &nodes).await?;
    assert!(
        matches!(reply, Some(ProbeReply::PinsDescribed { .. })),
        "the page's reply goes back to the run as it came"
    );
    assert_eq!(
        ops_since(&tab, from),
        ["describePins"],
        "one probe call, and no pin request: handed nodes are never pinned"
    );
    let sent = tab.script().probe_calls();
    let arguments = sent[from].arguments.as_array().ok_or("call arguments")?;
    assert_eq!(
        arguments[1],
        json!({ "value": null }),
        "a run from the Document sends an empty root before its nodes"
    );
    let handed: Vec<Value> = ids.iter().map(|&id| object(id)).collect();
    assert_eq!(
        arguments[2..],
        handed[..],
        "call node i is nodes[i], whatever order the resolves answered in"
    );
    assert_eq!(
        tab.script().most_resolves_in_flight(),
        8,
        "the resolves overlap, eight in flight and never more"
    );
    let resolves = tab.script().resolve_node_calls();
    assert_eq!(resolves.len(), 20, "each node resolved once");
    for params in resolves {
        assert_eq!(
            params["executionContextId"], ctx,
            "every node is resolved in the run's world"
        );
        assert_eq!(
            params["objectGroup"],
            scope.object_group(world.session()),
            "in the op's object group, which the op releases when it ends"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_run_from_an_element_handle_sends_the_handles_node_as_every_calls_root() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(card_page()));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let (card, pay, back) = (
        node(captured.tree(), CARD)?,
        node(captured.tree(), PAY)?,
        node(captured.tree(), BACK)?,
    );
    let ctx = world.context().0;
    // A handle's node in the capture, and one the capture left out (`QueryRoot::Untracked`).
    for tracked in [Some(card), None] {
        let handle = RunRoot::Handle {
            object: "handle-node".to_owned(),
            node: tracked,
        };
        let (_run, mut side) = ChunkWorld::start(&attempt, world.clone(), DocId::MAIN, handle)
            .map_err(|unproven| format!("{unproven:?}"))?;
        let from = tab.script().probe_calls().len();
        tab.script()
            .push_probe_reply(json!({ "kind": "pinsDescribed", "items": [] }));
        side.call(&describe(2), &[pay, back]).await?;
        let sent = tab.script().probe_calls();
        let arguments = sent[from].arguments.as_array().ok_or("call arguments")?;
        assert_eq!(
            arguments[1..],
            [
                json!({ "objectId": "handle-node" }),
                json!({ "objectId": format!("node-{ctx}-{PAY}") }),
                json!({ "objectId": format!("node-{ctx}-{BACK}") }),
            ],
            "the handle's node is the call's root, the run's nodes follow it in order"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_node_gone_makes_the_run_stale_and_a_world_gone_retires_its_document_and_refs()
-> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(framed_page()).with_child());
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let main_frame = page.main_endpoint().frame;
    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let (first, second) = (node(captured.tree(), 10)?, node(captured.tree(), 20)?);
    let foreign = node_in(captured.tree(), CHILD_DOC, 33)?;
    let (_run, mut side) =
        ChunkWorld::start(&attempt, world.clone(), DocId::MAIN, RunRoot::Document)
            .map_err(|unproven| format!("{unproven:?}"))?;
    let world_is_live = || {
        page.frames()
            .cached_world_for_frame(&main_frame)
            .is_some_and(|cached| cached.same_install(&world))
    };

    tab.script()
        .fail_resolve_node(20, "No node with given id found");
    let calls = tab.sent("Runtime.callFunctionOn");
    assert!(
        side.call(&describe(2), &[first, second]).await?.is_none(),
        "a member gone since the capture makes the run stale"
    );
    assert_eq!(
        tab.sent("Runtime.callFunctionOn"),
        calls,
        "and the call is never sent without it"
    );
    assert!(world_is_live(), "a node gone leaves its world live");

    let (resolves, calls) = (
        tab.sent("DOM.resolveNode"),
        tab.sent("Runtime.callFunctionOn"),
    );
    for broken in [
        side.call(&describe(2), &[first, foreign]).await.map(drop),
        side.hydrate(foreign).await.map(drop),
    ] {
        let Err(ProbeFailure::Broken(why)) = broken else {
            return Err(
                format!("a node of another document is a bug in the run: {broken:?}").into(),
            );
        };
        assert!(
            !why.contains("33") && !why.contains(CHILD_DOC),
            "the refusal names the cause, never an id: {why}"
        );
    }
    assert_eq!(
        (
            tab.sent("DOM.resolveNode"),
            tab.sent("Runtime.callFunctionOn")
        ),
        (resolves, calls),
        "a broken handover is refused before any CDP call"
    );

    let doc = token(DOC)?;
    let seq = captured
        .tree()
        .main_document()
        .and_then(TreeDoc::seq)
        .ok_or("the capture numbered the main document")?;
    let minted = tab_tree.ref_for(seq, &key(DOC, 10)?);
    assert!(matches!(tab_tree.resolve_ref(&minted), RefLookup::Key(_)));
    tab.script()
        .mark_global_stale(world.global(), "Cannot find context with specified id");
    let gone = side.call(&describe(1), &[first]).await;
    assert!(
        matches!(gone, Err(ProbeFailure::Gone(_))),
        "a stale context is the world gone, for the op's liveness rule: {gone:?}"
    );
    assert!(!world_is_live(), "the gone world is invalidated");
    assert!(
        matches!(page.frames().lookup_doc(&doc), DocLookup::Retired),
        "its document retires in DocTable"
    );
    assert_eq!(
        tab_tree.resolve_ref(&minted),
        RefLookup::Retired,
        "and the document's refs retire with it"
    );
    Ok(())
}

#[tokio::test]
async fn a_target_crash_stays_the_ops_failure_and_retires_nothing() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(card_page()));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let pay = node(captured.tree(), PAY)?;
    let (_run, mut side) = ChunkWorld::start(&attempt, world, DocId::MAIN, RunRoot::Document)
        .map_err(|unproven| format!("{unproven:?}"))?;

    tab.script().set_resolve_node_error(Some("Target crashed"));
    let called = side.call(&describe(1), &[pay]).await;
    assert!(
        matches!(called, Err(ProbeFailure::Cdp(_))),
        "a crashed target fails the op, never reads as a detached node: {called:?}"
    );
    let hydrated = side.hydrate(pay).await;
    assert!(
        matches!(hydrated, Err(ProbeFailure::Cdp(_))),
        "and so for a hydration: {hydrated:?}"
    );
    assert!(
        matches!(
            page.frames().lookup_doc(&token(DOC)?),
            DocLookup::Bound { .. }
        ),
        "a crash is no world gone: the document stays bound"
    );
    Ok(())
}

#[tokio::test]
async fn hydrate_pins_through_the_proven_world_and_binds_it() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(card_page()));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    tab.script().set_unproven_describe(true);
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    tab.script().set_unproven_describe(false);
    let doc = token(DOC)?;
    assert!(
        matches!(page.frames().lookup_doc(&doc), DocLookup::Reserved { .. }),
        "a capture that proved no world alive reserves the document's seq"
    );
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let pay = node(captured.tree(), PAY)?;
    let (_run, mut side) =
        ChunkWorld::start(&attempt, world.clone(), DocId::MAIN, RunRoot::Document)
            .map_err(|unproven| format!("{unproven:?}"))?;

    tab.script().set_resolve_probe_reply(Some(
        json!({ "kind": "element", "pin": 42, "preview": "<button>Pay</button>" }),
    ));
    let from = tab.script().probe_calls().len();
    assert_eq!(
        side.hydrate(pay).await?,
        Some(Pin(42)),
        "the probe's pin of the node's element"
    );
    let sent = tab.script().probe_calls();
    assert_eq!(
        (sent[from].op.as_str(), &sent[from].arguments[1]),
        (
            "resolve",
            &json!({ "objectId": format!("node-{}-{PAY}", world.context().0) })
        ),
        "the pin is of the node resolved in the run's world"
    );
    let DocLookup::Bound { world: bound, .. } = page.frames().lookup_doc(&doc) else {
        return Err("the pin proved the world holds the document".into());
    };
    assert!(
        bound.same_install(&world),
        "DocTable binds the document to this very install"
    );

    tab.script()
        .set_resolve_probe_reply(Some(json!({ "kind": "detached" })));
    assert_eq!(
        side.hydrate(pay).await?,
        None,
        "a node detached since the capture makes the run stale"
    );
    tab.script().set_resolve_probe_reply(Some(
        json!({ "kind": "element", "pin": 43, "preview": "<button>Pay</button>" }),
    ));
    let _ = tab_tree.retire_doc(page.frames(), &doc);
    assert_eq!(
        side.hydrate(pay).await?,
        None,
        "a pin DocTable will not bind, its document retired, is never handed out"
    );
    Ok(())
}

#[tokio::test]
async fn keys_read_a_runs_pins_in_one_batch_under_its_document() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(card_page()));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let (_run, mut side) = ChunkWorld::start(&attempt, world, DocId::MAIN, RunRoot::Document)
        .map_err(|unproven| format!("{unproven:?}"))?;

    let from = tab.script().probe_calls().len();
    tab.script()
        .set_pinned_nodes_reply(pinned(&[Some(CARD), Some(PAY)]));
    assert_eq!(
        side.keys(&[Pin(5), Pin(6)]).await?,
        Some(vec![key(DOC, CARD)?, key(DOC, PAY)?]),
        "each pin's node, keyed with the run's document"
    );
    assert_eq!(ops_since(&tab, from), ["pinned"], "one batch for every pin");

    tab.script()
        .set_pinned_nodes_reply(pinned(&[Some(CARD), None]));
    assert_eq!(
        side.keys(&[Pin(5), Pin(6)]).await?,
        None,
        "a pin the probe evicted makes the run stale"
    );
    tab.script()
        .set_pinned_nodes_reply(pinned(&[Some(CARD), Some(PAY)]));
    let _ = tab_tree.retire_doc(page.frames(), &token(DOC)?);
    assert_eq!(
        side.keys(&[Pin(5), Pin(6)]).await?,
        None,
        "keys DocTable will not bind to the run's document are never handed out"
    );
    Ok(())
}

#[tokio::test]
async fn a_mixed_chunk_runs_its_page_part_then_its_tree_part_and_pins_the_match() -> TestResult {
    let tab = ScriptedTab::new(TreeScript::new(card_page()));
    let run = ScriptedRun::over(&tab).await?;
    let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let tab_tree = TabTree::new(TabOriginState::OpenedUnderInstance);

    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    let captured = tab_tree.capture(&scope, &page, AttemptStart::now()).await?;
    let names = captured.tree().name_cache();
    let attempt = RunAttempt {
        scope: &scope,
        page: &page,
        tab: &tab_tree,
        capture: &captured,
        names: &names,
    };
    let (chunk_run, mut side) = ChunkWorld::start(&attempt, world, DocId::MAIN, RunRoot::Document)
        .map_err(|unproven| format!("{unproven:?}"))?;
    let plan = plan(&[
        part("internal:attr", "[data-card]"),
        part("internal:role", "button"),
    ])?;
    // The page matches the card; the card's pin keys back to its tree node; the tree finds the
    // card's button; the button is pinned.
    tab.script()
        .push_probe_reply(json!({ "kind": "staged", "matches": [{ "pin": 7 }] }));
    tab.script().set_pinned_nodes_reply(pinned(&[Some(CARD)]));
    tab.script().set_resolve_probe_reply(Some(
        json!({ "kind": "element", "pin": 42, "preview": "<button>Pay</button>" }),
    ));
    let from = tab.script().probe_calls().len();

    let running = chunk_run.run(&plan, Strictness::Strict, &mut side);
    assert_send(&running);
    let ChunkOutcome::Matched(matched) = running.await? else {
        return Err("the card holds one button".into());
    };
    let ChunkSet::Keys(keys) = &matched.set else {
        return Err("the tree part ends the chunk".into());
    };
    assert_eq!(
        keys.nodes().to_vec(),
        vec![node(captured.tree(), PAY)?],
        "the card's button, not the one outside it"
    );
    assert_eq!(
        chunk_run.first(&matched.set, &mut side).await?,
        First::Pin(Pin(42)),
        "the match, pinned for the action"
    );
    assert_eq!(
        ops_since(&tab, from),
        ["stage", "pinned", "resolve"],
        "the page part, its match's key, then the one hydration"
    );
    Ok(())
}
