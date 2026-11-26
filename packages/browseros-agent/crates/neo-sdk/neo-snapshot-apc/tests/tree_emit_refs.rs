//! Offline end-to-end integration tests for `SnapshotTree::aria`, `TabRefTable`,
//! `PrivateNodes`, `NameCache`, and `neo_snapshot::render`.

use std::error::Error;
use std::thread::Builder as ThreadBuilder;

use neo_snapshot::{AriaChild, AriaNode, Baseline, Call, Earlier, NodeBox, Passed, Snapshot};
use neo_snapshot_apc::proto::{
    AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion, AnnotatedRole,
    BoundingRect, BrowserOsContentAttributes, BrowserOsPageContentData, BrowserOsRef,
    ClickabilityReason, ContentAttributeType, ContentAttributes, ContentData, ContentNode,
    DocumentIdentifier, FormControlType, FrameData, Geometry, RedactionDecision, SelectOption,
};
use neo_snapshot_apc::refs::{Ref, RefKind, RefLookup, RefTarget, TabRefTable};
use neo_snapshot_apc::tree::{
    ApcBuilder, AriaAsk, AriaRoot, AriaTree, BuildOptions, DocId, DocSeq, DocToken, DomNodeId,
    FrameBuilder, MAX_JSON_DEPTH, MIN_CONTRACT_VERSION, NameCache, NameVisibility, NodeBuilder,
    NodeId, NodeKey, PartialIframeSpec, PrivateNodes, SiteKey, SnapshotTree, TreeNode,
};
use serde_json::Number;

const CANARY: &str = "NEO-SECRET-CANARY";

fn doc_token(raw: &str) -> Result<DocToken, Box<dyn Error>> {
    DocToken::new(raw).ok_or_else(|| format!("expected non-empty DocToken for {raw:?}").into())
}

fn node_key(doc: &str, dom_id: i32) -> Result<NodeKey, Box<dyn Error>> {
    Ok(NodeKey::new(doc_token(doc)?, DomNodeId::new(dom_id)))
}

fn build_tree(
    apc: AnnotatedPageContent,
    options: BuildOptions<'_>,
) -> Result<SnapshotTree, Box<dyn Error>> {
    Ok(SnapshotTree::build(apc, options)?)
}

fn node_id_of(tree: &SnapshotTree, doc: &str, dom_id: i32) -> Result<NodeId, Box<dyn Error>> {
    let key = node_key(doc, dom_id)?;
    tree.node_by_key(&key)
        .map(TreeNode::id)
        .ok_or_else(|| format!("missing node ({doc}, {dom_id})").into())
}

fn find_by_role_name<'a>(
    nodes: &'a [AriaNode],
    role: &str,
    name: Option<&str>,
) -> Option<&'a AriaNode> {
    let mut stack: Vec<&'a AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        if node.role == role && node.name.as_deref() == name {
            return Some(node);
        }
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
    None
}

fn find_by_role_text<'a>(nodes: &'a [AriaNode], role: &str, text: &str) -> Option<&'a AriaNode> {
    let mut stack: Vec<&'a AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        if node.role == role && node.text.as_deref() == Some(text) {
            return Some(node);
        }
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
    None
}

fn all_nodes(nodes: &[AriaNode]) -> Vec<&AriaNode> {
    let mut out = Vec::new();
    let mut stack: Vec<&AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
    out
}

fn parse_ref(raw: &str) -> Result<Ref, Box<dyn Error>> {
    Ref::parse(raw).ok_or_else(|| format!("invalid canonical ref {raw:?}").into())
}

fn visible_button(dom_id: i32, y: i32, text: &str) -> NodeBuilder {
    NodeBuilder::form_control(FormControlType::ButtonButton)
        .dom_id(dom_id)
        .tag("button")
        .bounding_box(0, y, 120, 24)
        .child(NodeBuilder::text(text))
}

fn render_page_baseline(
    snapshot: &Snapshot,
    passed: &Passed,
) -> Result<(String, Baseline), Box<dyn Error>> {
    let answer = neo_snapshot::render(snapshot, Call::Page(passed));
    let pending = answer
        .pending()
        .ok_or("Call::Page must yield a pending baseline")?;
    let baseline = Baseline {
        text: pending.text.to_owned(),
        form: pending.form,
    };
    Ok((answer.into_text(), baseline))
}

fn render_delta_with_baseline(
    snapshot: &Snapshot,
    passed: &Passed,
    earlier: &Baseline,
) -> Result<(String, Baseline), Box<dyn Error>> {
    let answer = neo_snapshot::render(
        snapshot,
        Call::Delta {
            passed,
            earlier: Earlier::Text(earlier),
        },
    );
    let pending = answer
        .pending()
        .ok_or("Call::Delta against Earlier::Text must yield a pending baseline")?;
    let baseline = Baseline {
        text: pending.text.to_owned(),
        form: pending.form,
    };
    Ok((answer.into_text(), baseline))
}

fn build_delta_page(target: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
    let mut root = NodeBuilder::root().dom_id(1).child(
        NodeBuilder::element("h1")
            .dom_id(2)
            .bounding_box(0, 0, 300, 32)
            .child(NodeBuilder::text("Workspace Settings Panel")),
    );
    for idx in 1..=18_i32 {
        root = root.child(visible_button(
            10 + idx,
            idx * 30,
            &format!("Action Item {idx:02}"),
        ));
    }
    root = root.child(target);
    let apc = ApcBuilder::new("doc-delta").root(root).build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn refs_rules_one_through_three_and_delta_stability() -> Result<(), Box<dyn Error>> {
    let expected_target = RefLookup::Key(RefTarget {
        key: node_key("doc-delta", 50)?,
        click_proxy: None,
    });
    let passed = Passed::default();
    let mut refs = TabRefTable::new();

    let aria_v1 =
        build_delta_page(visible_button(50, 600, "Save"))?.aria(AriaAsk::ai(&mut refs))?;
    let save_ref = find_by_role_name(aria_v1.nodes(), "button", Some("Save"))
        .and_then(|node| node.r#ref.clone())
        .ok_or("missing initial Save button ref")?;
    let (_, baseline_v1) = render_page_baseline(&aria_v1.into_snapshot(), &passed)?;

    // Rules 1 & 2: identical (role, name) across captures reuses every e-ref and yields no-change delta.
    let aria_repeat =
        build_delta_page(visible_button(50, 600, "Save"))?.aria(AriaAsk::ai(&mut refs))?;
    assert_eq!(
        find_by_role_name(aria_repeat.nodes(), "button", Some("Save"))
            .and_then(|node| node.r#ref.as_deref()),
        Some(save_ref.as_str())
    );
    let (delta_same, baseline_same) =
        render_delta_with_baseline(&aria_repeat.into_snapshot(), &passed, &baseline_v1)?;
    assert_eq!(
        delta_same,
        "# no change since your last snapshot of this tab"
    );

    // Rule 3a: name change on the same NodeKey mints a fresh e-ref; old alias stays resolvable.
    let aria_v2 =
        build_delta_page(visible_button(50, 600, "Saved"))?.aria(AriaAsk::ai(&mut refs))?;
    let saved_ref = find_by_role_name(aria_v2.nodes(), "button", Some("Saved"))
        .and_then(|node| node.r#ref.clone())
        .ok_or("missing renamed Saved button ref")?;
    assert_ne!(saved_ref, save_ref);
    assert_eq!(refs.resolve_str(&save_ref), expected_target);
    assert_eq!(refs.resolve_str(&saved_ref), expected_target);
    let (delta_renamed, baseline_v2) =
        render_delta_with_baseline(&aria_v2.into_snapshot(), &passed, &baseline_same)?;
    assert!(
        delta_renamed
            .starts_with("# changes since your last snapshot of this tab: 1 added, 1 removed")
    );
    assert!(delta_renamed.contains(&format!("-- button \"Save\" [ref={save_ref}]")));
    assert!(delta_renamed.contains(&format!("+- button \"Saved\" [ref={saved_ref}]")));

    // Rule 3b: role change (button -> link) mints a third e-ref; all three resolve to NodeKey.
    let link_node = NodeBuilder::element("a")
        .dom_id(50)
        .href("/saved")
        .bounding_box(0, 600, 120, 24)
        .child(NodeBuilder::text("Saved"));
    let aria_v3 = build_delta_page(link_node)?.aria(AriaAsk::ai(&mut refs))?;
    let link_ref = find_by_role_name(aria_v3.nodes(), "link", Some("Saved"))
        .and_then(|node| node.r#ref.clone())
        .ok_or("missing retyped link ref")?;
    assert_ne!(link_ref, save_ref);
    assert_ne!(link_ref, saved_ref);
    for r in [&save_ref, &saved_ref, &link_ref] {
        assert_eq!(refs.resolve_str(r), expected_target);
    }
    let (delta_retyped, _) =
        render_delta_with_baseline(&aria_v3.into_snapshot(), &passed, &baseline_v2)?;
    assert!(
        delta_retyped
            .starts_with("# changes since your last snapshot of this tab: 1 added, 1 removed")
    );
    assert!(delta_retyped.contains(&format!("-- button \"Saved\" [ref={saved_ref}]")));
    assert!(delta_retyped.contains(&format!("+- link \"Saved\" [ref={link_ref}]")));
    Ok(())
}

#[test]
fn refs_a_button_that_briefly_takes_no_pointer_events_keeps_its_ref() -> Result<(), Box<dyn Error>>
{
    let mut refs = TabRefTable::new();
    let first = build_delta_page(visible_button(50, 600, "Pay"))?.aria(AriaAsk::ai(&mut refs))?;
    let pay_ref = find_by_role_name(first.nodes(), "button", Some("Pay"))
        .and_then(|node| node.r#ref.clone())
        .ok_or("Pay prints with a ref")?;

    let blocked = build_delta_page(visible_button(50, 600, "Pay").pointer_events_none(true))?
        .aria(AriaAsk::ai(&mut refs))?;
    assert!(
        find_by_role_name(blocked.nodes(), "button", Some("Pay"))
            .is_some_and(|node| node.r#ref.is_none()),
        "a button that takes no pointer events prints without a ref"
    );

    let restored =
        build_delta_page(visible_button(50, 600, "Pay"))?.aria(AriaAsk::ai(&mut refs))?;
    assert_eq!(
        find_by_role_name(restored.nodes(), "button", Some("Pay"))
            .and_then(|node| node.r#ref.as_deref()),
        Some(pay_ref.as_str()),
        "the node never left the capture, so it gets its e-ref back"
    );
    Ok(())
}

#[test]
fn refs_rule_four_minted_n_ref_unifies_without_forcing_inclusion() -> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-r4")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(visible_button(10, 0, "Publish"))
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .dom_id(20)
                        .tag("button")
                        .visibility_hidden(true)
                        .bounding_box(0, 30, 120, 24)
                        .child(NodeBuilder::text("Hidden Action")),
                ),
        )
        .build();
    let tree = build_tree(apc, BuildOptions::new())?;

    let visible_key = node_key("doc-r4", 10)?;
    let hidden_key = node_key("doc-r4", 20)?;
    let mut refs = TabRefTable::new();

    let n1 = refs.ref_for(&visible_key, None);
    let n2 = refs.ref_for(&hidden_key, None);
    assert_eq!(n1.to_string(), "n1");
    assert_eq!(n1.kind(), RefKind::Minted);
    assert_eq!(n2.to_string(), "n2");
    assert_eq!(n2.kind(), RefKind::Minted);

    let aria = tree.aria(AriaAsk::ai(&mut refs))?;
    assert!(
        find_by_role_name(aria.nodes(), "button", Some("Hidden Action")).is_none(),
        "minting an n-ref must never force a hidden node into the snapshot"
    );
    assert!(
        all_nodes(aria.nodes())
            .into_iter()
            .all(|node| node.r#ref.as_deref() != Some("n2"))
    );

    let publish =
        find_by_role_name(aria.nodes(), "button", Some("Publish")).ok_or("missing Publish btn")?;
    let e1_str = publish
        .r#ref
        .as_deref()
        .ok_or("visible button must receive e1")?;
    assert_eq!(e1_str, "e1");
    let e1 = parse_ref(e1_str)?;

    assert_eq!(refs.ref_for(&visible_key, None), e1);
    assert_eq!(refs.ref_for(&hidden_key, None), n2);
    let visible_target = RefLookup::Key(RefTarget {
        key: visible_key,
        click_proxy: None,
    });
    assert_eq!(refs.resolve(&n1), visible_target);
    assert_eq!(refs.resolve(&e1), visible_target);
    assert_eq!(
        refs.resolve(&n2),
        RefLookup::Key(RefTarget {
            key: hidden_key,
            click_proxy: None,
        })
    );
    Ok(())
}

fn build_multi_doc_lifecycle_tree() -> Result<SnapshotTree, Box<dyn Error>> {
    let child_a = FrameBuilder::new("doc-child-a")
        .url("https://a.example/frame")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(visible_button(10, 0, "Child A")),
        );
    let child_b = FrameBuilder::new("doc-child-b")
        .url("https://b.example/frame")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(visible_button(10, 0, "Child B")),
        );
    let apc = ApcBuilder::new("doc-main")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(visible_button(10, 0, "Main Btn"))
                .child(
                    NodeBuilder::iframe(child_a)
                        .dom_id(20)
                        .bounding_box(0, 30, 200, 100),
                )
                .child(
                    NodeBuilder::iframe(child_b)
                        .dom_id(30)
                        .bounding_box(0, 140, 200, 100),
                ),
        )
        .build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn refs_lifecycle_retire_and_forget_enforce_monotonic_non_collision() -> Result<(), Box<dyn Error>>
{
    let tree = build_multi_doc_lifecycle_tree()?;
    let doc_a = doc_token("doc-child-a")?;
    let mut refs = TabRefTable::new();

    let first = tree.aria(AriaAsk::ai(&mut refs))?;
    let main_ref_1 = find_by_role_name(first.nodes(), "button", Some("Main Btn"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Main Btn ref")?;
    let child_a_ref_1 = find_by_role_name(first.nodes(), "button", Some("Child A"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Child A ref")?;
    let child_b_ref_1 = find_by_role_name(first.nodes(), "button", Some("Child B"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Child B ref")?;
    assert_eq!(main_ref_1, "e1");
    assert_eq!(child_a_ref_1, "f1e1");
    assert_eq!(child_b_ref_1, "f2e1");
    assert!(refs.resolve_str("e99").is_unknown());
    assert!(refs.resolve_str("f1e99").is_unknown());
    assert!(refs.resolve_str("f9e1").is_unknown());

    // forget_doc drops records for doc-child-a while keeping its seq binding and monotonic counter.
    refs.forget_doc(&doc_a);
    assert!(refs.resolve_str(&child_a_ref_1).is_retired());
    assert!(refs.resolve_str(&main_ref_1).as_key().is_some());
    assert!(refs.resolve_str(&child_b_ref_1).as_key().is_some());

    let second = tree.aria(AriaAsk::ai(&mut refs))?;
    let child_a_ref_2 = find_by_role_name(second.nodes(), "button", Some("Child A"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Child A ref after forget_doc")?;
    assert_eq!(child_a_ref_2, "f1e2");
    assert!(refs.resolve_str(&child_a_ref_1).is_retired());

    // retire_doc retires doc-child-a's sequence so re-capturing assigns a brand-new seq.
    refs.retire_doc(&doc_a);
    assert!(refs.resolve_str(&child_a_ref_2).is_retired());
    let third = tree.aria(AriaAsk::ai(&mut refs))?;
    let child_a_ref_3 = find_by_role_name(third.nodes(), "button", Some("Child A"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Child A ref after retire_doc")?;
    assert_eq!(child_a_ref_3, "f3e1");

    // retire_seq retires doc-child-b's seq 2 directly.
    refs.retire_seq(DocSeq::new(2));
    assert!(refs.resolve_str(&child_b_ref_1).is_retired());

    // forget drops every record while retaining monotonic per-seq counters.
    refs.forget();
    assert!(refs.resolve_str(&main_ref_1).is_retired());
    assert!(refs.resolve_str(&child_a_ref_3).is_retired());

    let fourth = tree.aria(AriaAsk::ai(&mut refs))?;
    let main_ref_2 = find_by_role_name(fourth.nodes(), "button", Some("Main Btn"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Main Btn ref after forget")?;
    assert_ne!(main_ref_2, main_ref_1);
    assert!(parse_ref(&main_ref_2)?.element() > parse_ref(&main_ref_1)?.element());
    assert!(refs.resolve_str(&main_ref_1).is_retired());
    Ok(())
}

fn build_buttons_doc(buttons: &[(i32, &str)]) -> Result<SnapshotTree, Box<dyn Error>> {
    let mut root = NodeBuilder::root().dom_id(1);
    for (idx, &(dom_id, label)) in buttons.iter().enumerate() {
        let y = i32::try_from(idx)? * 30;
        root = root.child(visible_button(dom_id, y, label));
    }
    let apc = ApcBuilder::new("doc-bounded").root(root).build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn refs_bounded_eviction_drops_identity_and_evicts_oldest_unseen_first()
-> Result<(), Box<dyn Error>> {
    let mut refs = TabRefTable::with_max_records(3);

    // Capture 1: A (e1, oldest tick) and B (e2, newer tick).
    let cap1 = build_buttons_doc(&[(10, "Alpha"), (20, "Beta")])?.aria(AriaAsk::ai(&mut refs))?;
    let ref_a = find_by_role_name(cap1.nodes(), "button", Some("Alpha"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Alpha ref")?;
    let ref_b1 = find_by_role_name(cap1.nodes(), "button", Some("Beta"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Beta ref")?;
    assert_eq!(refs.record_count(), 2);

    // Capture 2: only C (e3). Within cap (3), so A & B stay resolvable but lose last_identity.
    let cap2 = build_buttons_doc(&[(30, "Gamma")])?.aria(AriaAsk::ai(&mut refs))?;
    let ref_c = find_by_role_name(cap2.nodes(), "button", Some("Gamma"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Gamma ref")?;
    assert_eq!(refs.record_count(), 3);
    assert!(refs.resolve_str(&ref_a).as_key().is_some());
    assert!(refs.resolve_str(&ref_b1).as_key().is_some());

    // Capture 3: C (reused) + D (new). Record count exceeds 3 -> evicts oldest unseen (A), keeps B.
    let cap3 = build_buttons_doc(&[(30, "Gamma"), (40, "Delta")])?.aria(AriaAsk::ai(&mut refs))?;
    assert_eq!(
        find_by_role_name(cap3.nodes(), "button", Some("Gamma")).and_then(|n| n.r#ref.as_deref()),
        Some(ref_c.as_str())
    );
    assert_eq!(refs.record_count(), 3);
    assert!(refs.resolve_str(&ref_a).is_retired());
    assert!(refs.resolve_str(&ref_b1).as_key().is_some());

    // Capture 4: B returns with identical (role, name); dropped last_identity mints fresh e-ref
    // while keeping original ref_b1 resolvable as an alias.
    let cap4 = build_buttons_doc(&[(20, "Beta"), (30, "Gamma"), (40, "Delta")])?
        .aria(AriaAsk::ai(&mut refs))?;
    let ref_b2 = find_by_role_name(cap4.nodes(), "button", Some("Beta"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing returning Beta ref")?;
    assert_ne!(ref_b2, ref_b1);
    let expected_b = RefLookup::Key(RefTarget {
        key: node_key("doc-bounded", 20)?,
        click_proxy: None,
    });
    assert_eq!(refs.resolve_str(&ref_b1), expected_b);
    assert_eq!(refs.resolve_str(&ref_b2), expected_b);
    Ok(())
}

fn select_opt(value: &str, text: &str, selected: bool) -> SelectOption {
    SelectOption {
        value: Some(value.to_owned()),
        text: Some(text.to_owned()),
        is_selected: Some(selected),
        is_disabled: Some(false),
    }
}

fn build_ua_shadow_inputs() -> Vec<NodeBuilder> {
    let text_input = NodeBuilder::form_control(FormControlType::InputText)
        .dom_id(10)
        .tag("input")
        .aria_label("Public text")
        .field_value("value: public text")
        .bounding_box(0, 0, 200, 24)
        .child(
            NodeBuilder::container()
                .ua_shadow(true)
                .child(NodeBuilder::text("UA_ECHO_TEXT").ua_shadow(true)),
        );

    let date_input = NodeBuilder::form_control(FormControlType::InputDate)
        .dom_id(20)
        .tag("input")
        .aria_label("Birth date")
        .field_value("2020-02-03")
        .bounding_box(0, 30, 200, 24)
        .child(
            NodeBuilder::container()
                .ua_shadow(true)
                .child(
                    NodeBuilder::container()
                        .ua_shadow(true)
                        .role("spinbutton")
                        .aria_label("Month")
                        .child(NodeBuilder::text("02").ua_shadow(true)),
                )
                .child(NodeBuilder::text("/").ua_shadow(true)),
        );

    let file_input = NodeBuilder::form_control(FormControlType::InputFile)
        .dom_id(30)
        .tag("input")
        .aria_label("Upload doc")
        .bounding_box(0, 60, 200, 24)
        .child(
            NodeBuilder::form_control(FormControlType::InputButton)
                .ua_shadow(true)
                .tag("input")
                .child(NodeBuilder::text("Choose File").ua_shadow(true)),
        )
        .child(NodeBuilder::text("No file chosen").ua_shadow(true));

    let textarea = NodeBuilder::form_control(FormControlType::TextArea)
        .dom_id(50)
        .tag("textarea")
        .aria_label("Notes")
        .field_value("value: public textarea")
        .bounding_box(0, 220, 240, 60)
        .child(NodeBuilder::text("UA_TEXTAREA_ECHO").ua_shadow(true));

    vec![text_input, date_input, file_input, textarea]
}

fn build_ua_shadow_selects() -> Vec<NodeBuilder> {
    let collapsed = NodeBuilder::form_control(FormControlType::SelectOne)
        .dom_id(40)
        .tag("select")
        .aria_label("Fruit")
        .bounding_box(0, 90, 160, 24)
        .select_option(select_opt("a", "Apple", false))
        .select_option(select_opt("b", "Banana", true))
        .select_option(SelectOption {
            value: Some("c".to_owned()),
            text: Some("Cherry".to_owned()),
            is_selected: Some(false),
            is_disabled: Some(true),
        })
        .child(NodeBuilder::text("UA_BANANA_ECHO").ua_shadow(true));

    let listbox = NodeBuilder::form_control(FormControlType::SelectMultiple)
        .dom_id(41)
        .tag("select")
        .list_box(true)
        .aria_label("Multi Fruit")
        .bounding_box(0, 120, 160, 60)
        .select_option(select_opt("lb", "List Opt", true))
        .child(
            NodeBuilder::element("option")
                .bounding_box(0, 120, 160, 20)
                .child(NodeBuilder::text("List Opt")),
        );

    let real_opt = NodeBuilder::form_control(FormControlType::SelectOne)
        .dom_id(42)
        .tag("select")
        .aria_label("Real Select")
        .bounding_box(0, 190, 160, 24)
        .select_option(select_opt("r", "Real Opt", false))
        .child(
            NodeBuilder::element("option")
                .bounding_box(0, 190, 160, 20)
                .child(NodeBuilder::text("Real Opt")),
        );

    vec![collapsed, listbox, real_opt]
}

fn build_details_node(base_id: i32, y: i32, prefix: &str, hidden_body: bool) -> NodeBuilder {
    let mut slot = NodeBuilder::container()
        .ua_shadow(true)
        .child(
            NodeBuilder::element("p")
                .dom_id(base_id + 2)
                .bounding_box(0, y + 30, 200, 20)
                .child(NodeBuilder::text(&format!("{prefix} paragraph"))),
        )
        .child(visible_button(
            base_id + 3,
            y + 55,
            &format!("{prefix} btn"),
        ));
    if hidden_body {
        slot = slot.annotated_role(AnnotatedRole::ContentHidden);
    }
    NodeBuilder::element("details")
        .dom_id(base_id)
        .bounding_box(0, y, 300, 80)
        .child(
            NodeBuilder::container()
                .ua_shadow(true)
                .child(NodeBuilder::text(&format!("UA_{prefix}_MARKER")).ua_shadow(true))
                .child(
                    NodeBuilder::element("summary")
                        .dom_id(base_id + 1)
                        .bounding_box(0, y, 200, 24)
                        .child(NodeBuilder::text(&format!("{prefix} summary"))),
                )
                .child(slot),
        )
}

fn build_ua_shadow_media_and_details() -> Vec<NodeBuilder> {
    let video = NodeBuilder::element("video")
        .dom_id(60)
        .bounding_box(0, 290, 320, 180)
        .child(
            NodeBuilder::container()
                .ua_shadow(true)
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .ua_shadow(true)
                        .tag("button")
                        .child(NodeBuilder::text("Shadow Play").ua_shadow(true)),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .ua_shadow(true)
                        .tag("button")
                        .child(NodeBuilder::text("Shadow Mute").ua_shadow(true)),
                ),
        );
    vec![
        video,
        build_details_node(70, 480, "Closed", true),
        build_details_node(80, 560, "Open", false),
    ]
}

fn assert_ua_shadow_inputs_and_selects(
    tree: &SnapshotTree,
    nodes: &[AriaNode],
    pw: &str,
) -> Result<(), Box<dyn Error>> {
    let text_in = find_by_role_name(nodes, "textbox", Some("Public text")).ok_or("missing text")?;
    assert_eq!(text_in.text.as_deref(), Some("value: public text"));
    assert!(text_in.children.is_empty());
    assert_eq!(pw.matches("\"value: public text\"").count(), 1);

    let date_in = find_by_role_name(nodes, "textbox", Some("Birth date")).ok_or("missing date")?;
    assert_eq!(date_in.text.as_deref(), Some("2020-02-03"));
    assert!(date_in.children.is_empty());
    assert!(
        all_nodes(nodes)
            .into_iter()
            .all(|node| node.role != "spinbutton")
    );

    let file_in = find_by_role_name(nodes, "button", Some("Upload doc")).ok_or("missing file")?;
    assert!(file_in.children.is_empty());
    assert_eq!(file_in.text, None);

    let notes = find_by_role_name(nodes, "textbox", Some("Notes")).ok_or("missing textarea")?;
    assert_eq!(notes.text.as_deref(), Some("value: public textarea"));
    assert!(notes.children.is_empty());

    let fruit = find_by_role_name(nodes, "combobox", Some("Fruit")).ok_or("missing Fruit")?;
    assert_eq!(fruit.children.len(), 3);
    let banana = find_by_role_name(nodes, "option", Some("Banana")).ok_or("missing Banana")?;
    assert_eq!(banana.selected, Some(true));
    assert_eq!(banana.r#ref, None);
    let cherry = find_by_role_name(nodes, "option", Some("Cherry")).ok_or("missing Cherry")?;
    assert_eq!(cherry.disabled, Some(true));
    assert_eq!(cherry.r#ref, None);

    let multi = find_by_role_name(nodes, "listbox", Some("Multi Fruit")).ok_or("missing Multi")?;
    assert_eq!(multi.children.len(), 1);
    assert!(
        tree.synthetic_options(node_id_of(tree, "doc-ua", 41)?)
            .is_empty()
    );
    let real = find_by_role_name(nodes, "combobox", Some("Real Select")).ok_or("missing Real")?;
    assert_eq!(real.children.len(), 1);
    assert!(
        tree.synthetic_options(node_id_of(tree, "doc-ua", 42)?)
            .is_empty()
    );
    Ok(())
}

#[test]
fn ua_shadow_all_seven_shapes_and_select_no_duplicate_synthesis() -> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-ua")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .children(build_ua_shadow_inputs())
                .children(build_ua_shadow_selects())
                .children(build_ua_shadow_media_and_details()),
        )
        .build();
    let tree = build_tree(apc, BuildOptions::new())?;

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let pw = neo_snapshot::render(&ai.to_snapshot(), Call::Playwright).into_text();
    assert_ua_shadow_inputs_and_selects(&tree, ai.nodes(), &pw)?;

    assert!(find_by_role_text(ai.nodes(), "generic", "Closed summary").is_some());
    assert!(find_by_role_text(ai.nodes(), "paragraph", "Closed paragraph").is_none());
    assert!(find_by_role_name(ai.nodes(), "button", Some("Closed btn")).is_none());

    let open_pos = pw.find("Open summary").ok_or("missing Open summary")?;
    let para_pos = pw.find("Open paragraph").ok_or("missing Open paragraph")?;
    let btn_pos = pw.find("Open btn").ok_or("missing Open btn")?;
    assert!(open_pos < para_pos && para_pos < btn_pos);

    for leaked in [
        "UA_ECHO_TEXT",
        "Choose File",
        "No file chosen",
        "UA_BANANA_ECHO",
        "UA_TEXTAREA_ECHO",
        "Shadow Play",
        "Shadow Mute",
        "UA_Closed_MARKER",
        "UA_Open_MARKER",
        "Closed paragraph",
        "Closed btn",
    ] {
        assert!(!pw.contains(leaked), "ua_shadow leaked {leaked:?}");
    }
    Ok(())
}

fn sample_scroll_controls() -> (NodeBuilder, NodeBuilder, NodeBuilder, NodeBuilder) {
    (
        NodeBuilder::element("h2")
            .dom_id(10)
            .bounding_box(0, 0, 200, 28)
            .child(NodeBuilder::text("Catalog")),
        visible_button(11, 40, "Buy"),
        NodeBuilder::form_control(FormControlType::InputText)
            .dom_id(12)
            .tag("input")
            .aria_label("Qty")
            .field_value("2")
            .bounding_box(0, 80, 100, 24),
        NodeBuilder::element("a")
            .dom_id(13)
            .href("/item")
            .bounding_box(0, 120, 80, 20)
            .child(NodeBuilder::text("Details")),
    )
}

#[test]
fn plain_container_scroll_independence() -> Result<(), Box<dyn Error>> {
    let (h1, b1, t1, l1) = sample_scroll_controls();
    let flat_tree = build_tree(
        ApcBuilder::new("doc-flat")
            .root(NodeBuilder::root().child(h1).child(b1).child(t1).child(l1))
            .build(),
        BuildOptions::new(),
    )?;

    let (h2, b2, t2, l2) = sample_scroll_controls();
    let wrapped_tree = build_tree(
        ApcBuilder::new("doc-flat")
            .root(
                NodeBuilder::root().child(
                    NodeBuilder::container()
                        .bounding_box(0, 0, 400, 300)
                        .child(
                            NodeBuilder::element("span")
                                .bounding_box(0, 0, 200, 28)
                                .child(h2),
                        )
                        .child(
                            NodeBuilder::container()
                                .bounding_box(0, 40, 200, 120)
                                .child(b2)
                                .child(
                                    NodeBuilder::element("span")
                                        .bounding_box(0, 80, 120, 24)
                                        .child(t2),
                                )
                                .child(
                                    NodeBuilder::container()
                                        .bounding_box(0, 120, 100, 20)
                                        .child(l2),
                                ),
                        ),
                ),
            )
            .build(),
        BuildOptions::new(),
    )?;

    let mut flat_refs = TabRefTable::new();
    let mut wrapped_refs = TabRefTable::new();
    let flat_ai = flat_tree.aria(AriaAsk::ai(&mut flat_refs))?;
    let wrapped_ai = wrapped_tree.aria(AriaAsk::ai(&mut wrapped_refs))?;
    assert_eq!(flat_ai.nodes(), wrapped_ai.nodes());

    let flat_snap = flat_ai.into_snapshot();
    let wrapped_snap = wrapped_ai.into_snapshot();
    assert_eq!(
        neo_snapshot::render(&flat_snap, Call::Playwright).into_text(),
        neo_snapshot::render(&wrapped_snap, Call::Playwright).into_text()
    );
    assert_eq!(
        neo_snapshot::render(&flat_snap, Call::Full { max_tokens: None }).into_text(),
        neo_snapshot::render(&wrapped_snap, Call::Full { max_tokens: None }).into_text()
    );
    assert_eq!(
        neo_snapshot::render(&flat_snap, Call::Page(&Passed::default())).into_text(),
        neo_snapshot::render(&wrapped_snap, Call::Page(&Passed::default())).into_text()
    );
    Ok(())
}

#[test]
fn clickable_div_cursor_pointer_and_nested_suppression() -> Result<(), Box<dyn Error>> {
    let click_tree = build_tree(
        ApcBuilder::new("doc-click")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::container()
                            .dom_id(10)
                            .bounding_box(0, 0, 220, 60)
                            .clickability_reason(ClickabilityReason::CursorPointer)
                            .child(NodeBuilder::text("Pointer card"))
                            .child(
                                NodeBuilder::container()
                                    .dom_id(11)
                                    .bounding_box(10, 20, 180, 30)
                                    .clickability_reason(ClickabilityReason::ClickHandler)
                                    .child(NodeBuilder::text("Nested handler"))
                                    .child(
                                        visible_button(12, 22, "Nested btn")
                                            .clickability_reason(ClickabilityReason::CursorPointer),
                                    ),
                            ),
                    )
                    .child(
                        NodeBuilder::container()
                            .dom_id(20)
                            .bounding_box(0, 80, 220, 30)
                            .clickability_reason(ClickabilityReason::ClickHandler)
                            .child(NodeBuilder::text("Handler card")),
                    ),
            )
            .build(),
        BuildOptions::new(),
    )?;

    let mut refs = TabRefTable::new();
    let ai = click_tree.aria(AriaAsk::ai(&mut refs))?;
    let outer_card = ai.nodes().first().ok_or("missing pointer card")?;
    assert_eq!(outer_card.role, "generic");
    assert!(outer_card.r#ref.is_some());
    assert_eq!(outer_card.cursor.as_deref(), Some("pointer"));

    let nested_btn =
        find_by_role_name(ai.nodes(), "button", Some("Nested btn")).ok_or("missing Nested btn")?;
    assert_eq!(nested_btn.cursor, None);

    let handler_card =
        find_by_role_text(ai.nodes(), "generic", "Handler card").ok_or("missing Handler card")?;
    assert!(handler_card.r#ref.is_some());
    assert_eq!(handler_card.cursor.as_deref(), Some("pointer"));

    let pointer_count = all_nodes(ai.nodes())
        .into_iter()
        .filter(|node| node.cursor.as_deref() == Some("pointer"))
        .count();
    assert_eq!(pointer_count, 2);

    let pw = neo_snapshot::render(&ai.into_snapshot(), Call::Playwright).into_text();
    assert!(pw.contains("generic [ref=e1] [cursor=pointer]"));
    Ok(())
}

/// Contract field 38 (P22): stock APC fills an element's empty boxes from its content, so a
/// zero-sized `cursor: pointer` element arrives with a drawn-looking outer box. Playwright tests
/// the element's own `getBoundingClientRect()` and gives it no ref; the flag makes Rust agree.
/// The same shape without the flag, which is all a version 7 browser sends, keeps its ref.
#[test]
fn own_box_empty_takes_a_borrowed_box_out_of_the_pointer_targets() -> Result<(), Box<dyn Error>> {
    let pointer_card = |dom_id: i32, y: i32, text: &str| {
        NodeBuilder::container()
            .dom_id(dom_id)
            .bounding_box(0, y, 59, 73)
            .clickability_reason(ClickabilityReason::CursorPointer)
            .child(NodeBuilder::text(text))
    };
    let tree = build_tree(
        ApcBuilder::new("doc-own-box")
            .root(
                NodeBuilder::root()
                    .child(pointer_card(10, 0, "Zero-sized target").own_box_empty(true))
                    .child(pointer_card(20, 100, "Borrowed-box target")),
            )
            .build(),
        BuildOptions::new(),
    )?;
    let zero_sized = tree
        .node(node_id_of(&tree, "doc-own-box", 10)?)
        .ok_or("missing zero-sized node")?;
    assert!(
        !zero_sized.hidden().is_box_visible(),
        "a borrowed box around an empty own box is not drawn"
    );
    assert!(
        tree.node(node_id_of(&tree, "doc-own-box", 20)?)
            .ok_or("missing borrowed-box node")?
            .hidden()
            .is_box_visible(),
        "without the flag the borrowed outer box still reads as drawn"
    );

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let pointer_nodes: Vec<&AriaNode> = all_nodes(ai.nodes())
        .into_iter()
        .filter(|node| node.cursor.as_deref() == Some("pointer"))
        .collect();
    assert_eq!(
        pointer_nodes.len(),
        1,
        "only the unflagged card is a pointer target"
    );
    let pw = neo_snapshot::render(&ai.into_snapshot(), Call::Playwright).into_text();
    let line_of = |text: &str| {
        pw.lines()
            .find(|line| line.contains(text))
            .map(str::to_owned)
            .ok_or_else(|| format!("no line prints {text:?} in:\n{pw}"))
    };
    let zero_line = line_of("Zero-sized target")?;
    assert!(
        !zero_line.contains("[ref=") && !zero_line.contains("[cursor=pointer]"),
        "the zero-sized card gets no ref and no pointer mark: {zero_line:?}"
    );
    let borrowed_line = line_of("Borrowed-box target")?;
    assert!(
        borrowed_line.contains("[ref=") && borrowed_line.contains("[cursor=pointer]"),
        "the unflagged card keeps its ref and pointer mark: {borrowed_line:?}"
    );
    Ok(())
}

fn zero_size_input(control_type: FormControlType, dom_id: i32, y: i32) -> NodeBuilder {
    NodeBuilder::form_control(control_type)
        .dom_id(dom_id)
        .tag("input")
        .bounding_box(0, y, 0, 0)
}

fn build_click_proxy_all_controls() -> Result<SnapshotTree, Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("label")
                .dom_id(10)
                .label_for(11)
                .bounding_box(0, 0, 180, 24)
                .child(NodeBuilder::text("Agree to "))
                .child(
                    NodeBuilder::element("a")
                        .dom_id(12)
                        .href("/terms")
                        .bounding_box(80, 0, 60, 24)
                        .child(NodeBuilder::text("Terms")),
                ),
        )
        .child(zero_size_input(FormControlType::InputCheckbox, 11, 0))
        .child(
            NodeBuilder::element("label")
                .dom_id(20)
                .label_for(21)
                .bounding_box(0, 30, 160, 24)
                .child(zero_size_input(FormControlType::InputCheckbox, 21, 30))
                .child(NodeBuilder::text("Subscribe")),
        )
        .child(
            NodeBuilder::element("label")
                .dom_id(30)
                .label_for(31)
                .bounding_box(0, 60, 160, 24)
                .child(NodeBuilder::text("Plan Basic")),
        )
        .child(zero_size_input(FormControlType::InputRadio, 31, 60))
        .child(
            NodeBuilder::element("label")
                .dom_id(40)
                .label_for(41)
                .bounding_box(0, 90, 160, 24)
                .child(zero_size_input(FormControlType::InputRadio, 41, 90))
                .child(NodeBuilder::text("Plan Pro")),
        )
        .child(
            NodeBuilder::element("label")
                .dom_id(50)
                .label_for(51)
                .bounding_box(0, 120, 160, 24)
                .child(NodeBuilder::text("Upload ID")),
        )
        .child(zero_size_input(FormControlType::InputFile, 51, 120))
        .child(
            NodeBuilder::element("label")
                .dom_id(60)
                .label_for(61)
                .bounding_box(0, 150, 160, 24)
                .child(zero_size_input(FormControlType::InputFile, 61, 150))
                .child(NodeBuilder::text("Upload Avatar")),
        );
    let apc = ApcBuilder::new("doc-proxy").root(root).build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn click_proxy_zero_size_checkbox_radio_file_ai_vs_default() -> Result<(), Box<dyn Error>> {
    let tree = build_click_proxy_all_controls()?;
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;

    let terms = find_by_role_name(ai.nodes(), "link", Some("Terms"))
        .ok_or("unrelated link inside folded label must stay visible")?;
    assert_eq!(terms.url.as_deref(), Some("/terms"));
    assert!(terms.r#ref.is_some());

    let expected_pairs = [
        ("checkbox", "Agree to Terms", 11, 10),
        ("checkbox", "Subscribe", 21, 20),
        ("radio", "Plan Basic", 31, 30),
        ("radio", "Plan Pro", 41, 40),
        ("button", "Upload ID", 51, 50),
        ("button", "Upload Avatar", 61, 60),
    ];
    for (role, name, control_dom, label_dom) in expected_pairs {
        let node = find_by_role_name(ai.nodes(), role, Some(name))
            .ok_or_else(|| format!("missing {role} {name:?}"))?;
        let r = node
            .r#ref
            .as_deref()
            .ok_or_else(|| format!("missing ref on {role} {name:?}"))?;
        assert_eq!(
            refs.resolve_str(r),
            RefLookup::Key(RefTarget {
                key: node_key("doc-proxy", control_dom)?,
                click_proxy: Some(node_key("doc-proxy", label_dom)?),
            })
        );
    }

    for folded in [
        "Agree to",
        "Subscribe",
        "Plan Basic",
        "Plan Pro",
        "Upload ID",
        "Upload Avatar",
    ] {
        let has_stray = all_nodes(ai.nodes())
            .into_iter()
            .any(|node| node.text.as_deref().is_some_and(|t| t.contains(folded)));
        assert!(!has_stray, "folded label text leaked: {folded:?}");
    }

    let default = tree.aria(AriaAsk::default_mode())?;
    assert!(
        all_nodes(default.nodes())
            .into_iter()
            .all(|node| node.r#ref.is_none())
    );
    assert!(
        all_nodes(default.nodes())
            .into_iter()
            .any(|node| node.role == "text" && node.text.as_deref() == Some("Agree to"))
    );
    assert!(all_nodes(default.nodes()).into_iter().any(|node| {
        node.role == "text"
            && node
                .text
                .as_deref()
                .is_some_and(|text| text.contains("Plan Basic"))
    }));
    Ok(())
}

fn build_nosize_negative_tree() -> Result<SnapshotTree, Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("label")
                .dom_id(10)
                .label_for(11)
                .bounding_box(0, 0, 120, 24)
                .child(NodeBuilder::text("Hidden Text")),
        )
        .child(zero_size_input(FormControlType::InputText, 11, 0).field_value("draft"))
        .child(
            NodeBuilder::element("label")
                .dom_id(20)
                .label_for(21)
                .bounding_box(0, 30, 120, 24)
                .child(NodeBuilder::text("Hidden Qty")),
        )
        .child(zero_size_input(FormControlType::InputNumber, 21, 30).field_value("42"))
        .child(zero_size_input(FormControlType::InputRadio, 31, 60).aria_label("Lonely Radio"))
        .child(
            NodeBuilder::element("label")
                .dom_id(40)
                .label_for(41)
                .bounding_box(0, 90, 140, 24)
                .child(NodeBuilder::text("Contents Check")),
        )
        .child(
            zero_size_input(FormControlType::InputCheckbox, 41, 90)
                .display_contents(true)
                .aria_label("Contents Check"),
        )
        .child(
            NodeBuilder::element("label")
                .dom_id(50)
                .label_for(51)
                .pointer_events_none(true)
                .bounding_box(0, 120, 140, 24)
                .child(NodeBuilder::text("No Pointer Label")),
        )
        .child(zero_size_input(FormControlType::InputCheckbox, 51, 120))
        .child(
            NodeBuilder::element("label")
                .dom_id(60)
                .label_for(61)
                .aria_hidden(true)
                .bounding_box(0, 150, 140, 24)
                .child(NodeBuilder::text("Aria Hidden Label")),
        )
        .child(
            zero_size_input(FormControlType::InputCheckbox, 61, 150)
                .aria_label("Hidden Label Target"),
        )
        .child(
            NodeBuilder::element("label")
                .dom_id(70)
                .label_for(71)
                .bounding_box(0, 180, 140, 24)
                .child(NodeBuilder::text("Material Toggle")),
        )
        .child(
            NodeBuilder::form_control(FormControlType::InputCheckbox)
                .dom_id(71)
                .tag("input")
                .pointer_events_none(true)
                .bounding_box(0, 180, 20, 20),
        );
    let apc = ApcBuilder::new("doc-nosize").root(root).build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn click_proxy_nosize_negative_cases() -> Result<(), Box<dyn Error>> {
    let tree = build_nosize_negative_tree()?;
    for dom_id in [11, 21, 31, 41, 51, 61, 71] {
        assert!(
            tree.click_proxy(node_id_of(&tree, "doc-nosize", dom_id)?)
                .is_none(),
            "dom_id {dom_id} must not receive a ClickProxy"
        );
    }

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;

    for (role, name) in [
        ("textbox", "Hidden Text"),
        ("spinbutton", "Hidden Qty"),
        ("radio", "Lonely Radio"),
        ("checkbox", "No Pointer Label"),
        ("checkbox", "Hidden Label Target"),
        ("checkbox", "Material Toggle"),
    ] {
        let node = find_by_role_name(ai.nodes(), role, Some(name))
            .ok_or_else(|| format!("missing unref'd {role} {name:?}"))?;
        assert_eq!(node.r#ref, None, "{role} {name:?} must not receive a ref");
    }
    assert!(find_by_role_name(ai.nodes(), "checkbox", Some("Contents Check")).is_none());

    let page_text =
        neo_snapshot::render(&ai.into_snapshot(), Call::Page(&Passed::default())).into_text();
    for expected in [
        "textbox \"Hidden Text\"",
        "spinbutton \"Hidden Qty\"",
        "checkbox \"Material Toggle\"",
    ] {
        let line = page_text
            .lines()
            .find(|line| line.contains(expected))
            .ok_or_else(|| format!("missing field_without_ref line for {expected}"))?;
        assert!(!line.contains("[ref="), "unexpected ref on line: {line}");
    }
    Ok(())
}

fn set_raw_redaction(
    apc: &mut AnnotatedPageContent,
    dom_id: i32,
    code: i32,
) -> Result<(), Box<dyn Error>> {
    let mut stack: Vec<&mut ContentNode> = apc.root_node.iter_mut().collect();
    while let Some(node) = stack.pop() {
        if let Some(attrs) = node.content_attributes.as_mut()
            && attrs.common_ancestor_dom_node_id == Some(dom_id)
        {
            attrs.redaction_decision = Some(code);
            if let Some(ContentData::FormControl(control)) = attrs.content_data.as_mut() {
                control.redaction_decision = Some(code);
            }
            return Ok(());
        }
        stack.extend(node.children_nodes.iter_mut());
    }
    Err(format!("missing raw ContentNode for dom_id {dom_id}").into())
}

struct RedactCase {
    dom_id: i32,
    tag: &'static str,
    control: FormControlType,
    label: &'static str,
    decision: RedactionDecision,
    bullets: u32,
}

fn redacted_control(case: &RedactCase) -> NodeBuilder {
    let secret = format!("{CANARY}-{}", case.dom_id);
    let mut builder = NodeBuilder::form_control(case.control)
        .dom_id(case.dom_id)
        .tag(case.tag)
        .aria_label(case.label)
        .redaction(case.decision)
        .field_value(&secret)
        .bounding_box(0, case.dom_id * 2, 200, 24)
        .child(NodeBuilder::text(&secret).dom_text(&secret));
    if case.bullets > 0 {
        builder = builder.redacted_value_length(case.bullets);
    }
    builder
}

fn assert_no_canary(surface: &str, text: &str) {
    assert!(
        !text.contains(CANARY),
        "canary leaked into {surface}: {text}"
    );
}

fn assert_zero_canary_surface(
    tree: &SnapshotTree,
    ai: &AriaTree,
    default: &AriaTree,
    refs: &TabRefTable,
    private: &PrivateNodes,
    cache: &NameCache<'_>,
) -> Result<(), Box<dyn Error>> {
    assert_no_canary("ai json", &serde_json::to_string(ai.nodes())?);
    assert_no_canary("default json", &serde_json::to_string(default.nodes())?);

    for (label, snap) in [("ai", ai.to_snapshot()), ("default", default.to_snapshot())] {
        assert_no_canary(
            &format!("{label} Playwright"),
            &neo_snapshot::render(&snap, Call::Playwright).into_text(),
        );
        assert_no_canary(
            &format!("{label} Full"),
            &neo_snapshot::render(&snap, Call::Full { max_tokens: None }).into_text(),
        );
        assert_no_canary(
            &format!("{label} Page"),
            &neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text(),
        );
    }

    for node in tree.nodes() {
        let id = node.id();
        assert_no_canary(
            "NameCache visible name",
            cache.name(id, NameVisibility::VisibleOnly).text(),
        );
        assert_no_canary(
            "NameCache hidden name",
            cache.name(id, NameVisibility::IncludeHidden).text(),
        );
        assert_no_canary(
            "NameCache visible description",
            cache.description(id, NameVisibility::VisibleOnly).text(),
        );
        assert_no_canary(
            "NameCache hidden description",
            cache.description(id, NameVisibility::IncludeHidden).text(),
        );
        assert_no_canary("NameCache error_message", cache.error_message(id));
        assert_no_canary("TreeNode Debug", &format!("{node:?}"));
    }

    for doc in tree.documents() {
        assert_no_canary("TreeDoc Debug", &format!("{doc:?}"));
    }
    assert_no_canary("SnapshotTree Debug", &format!("{tree:?}"));
    assert_no_canary("AriaTree ai Debug", &format!("{ai:?}"));
    assert_no_canary("AriaTree default Debug", &format!("{default:?}"));
    assert_no_canary("TabRefTable Debug", &format!("{refs:?}"));
    assert_no_canary("PrivateNodes Debug", &format!("{private:?}"));
    assert_no_canary("NameCache Debug", &format!("{cache:?}"));
    Ok(())
}

type RedactSpec = (i32, &'static str, RedactionDecision, u32);

const CHROMIUM_INPUT_SPECS: &[RedactSpec] = &[
    (
        10,
        "in-curr-pw",
        RedactionDecision::RedactedHasBeenPassword,
        8,
    ),
    (
        11,
        "in-new-pw",
        RedactionDecision::RedactedHasBeenPassword,
        9,
    ),
    (12, "in-otp", RedactionDecision::RedactedIsOtp, 6),
    (
        13,
        "in-cc-num",
        RedactionDecision::RedactedIsSensitivePaymentField,
        16,
    ),
    (
        14,
        "in-cc-csc",
        RedactionDecision::RedactedIsSensitivePaymentField,
        3,
    ),
    (
        30,
        "custom-css",
        RedactionDecision::RedactedCustomPasswordCss,
        6,
    ),
    (
        31,
        "custom-js",
        RedactionDecision::RedactedCustomPasswordJs,
        7,
    ),
    (
        32,
        "type-flip-ch",
        RedactionDecision::RedactedHasBeenPassword,
        12,
    ),
    (
        41,
        "empty-pay",
        RedactionDecision::UnredactedEmptyPaymentField,
        0,
    ),
    (
        42,
        "empty-css",
        RedactionDecision::UnredactedEmptyCustomPassword,
        0,
    ),
    (50, "future-99", RedactionDecision::NoRedactionNecessary, 11),
];

const CHROMIUM_TEXTAREA_SPECS: &[RedactSpec] = &[
    (
        20,
        "ta-curr-pw",
        RedactionDecision::RedactedHasBeenPassword,
        7,
    ),
    (
        21,
        "ta-new-pw",
        RedactionDecision::RedactedHasBeenPassword,
        10,
    ),
    (22, "ta-otp", RedactionDecision::RedactedIsOtp, 5),
    (
        23,
        "ta-cc-num",
        RedactionDecision::RedactedIsSensitivePaymentField,
        15,
    ),
    (
        24,
        "ta-cc-csc",
        RedactionDecision::RedactedIsSensitivePaymentField,
        4,
    ),
    (
        43,
        "empty-otp",
        RedactionDecision::UnredactedEmptyOtpField,
        0,
    ),
];

fn chromium_redact_cases() -> Vec<RedactCase> {
    let mut cases: Vec<RedactCase> = CHROMIUM_INPUT_SPECS
        .iter()
        .map(|&(dom_id, label, decision, bullets)| RedactCase {
            dom_id,
            tag: "input",
            control: FormControlType::InputText,
            label,
            decision,
            bullets,
        })
        .collect();
    cases.push(RedactCase {
        dom_id: 40,
        tag: "input",
        control: FormControlType::InputPassword,
        label: "empty-pw",
        decision: RedactionDecision::UnredactedEmptyPassword,
        bullets: 0,
    });
    cases.extend(
        CHROMIUM_TEXTAREA_SPECS
            .iter()
            .map(|&(dom_id, label, decision, bullets)| RedactCase {
                dom_id,
                tag: "textarea",
                control: FormControlType::TextArea,
                label,
                decision,
                bullets,
            }),
    );
    cases
}

fn build_chromium_privacy_apc() -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let mut controls: Vec<NodeBuilder> = chromium_redact_cases()
        .iter()
        .map(redacted_control)
        .collect();
    controls.push(
        NodeBuilder::form_control(FormControlType::InputText)
            .dom_id(60)
            .tag("input")
            .aria_label("public-note")
            .field_value("public-ok")
            .bounding_box(0, 120, 200, 24),
    );
    let mut apc = ApcBuilder::new("doc-priv-ch")
        .root(NodeBuilder::root().dom_id(1).children(controls))
        .build();
    set_raw_redaction(&mut apc, 50, 99)?;
    Ok(apc)
}

#[test]
fn privacy_chromium_redaction_tokens_type_flip_and_zero_canaries() -> Result<(), Box<dyn Error>> {
    let apc = build_chromium_privacy_apc()?;
    let private = PrivateNodes::new();
    let tree = build_tree(apc, BuildOptions::new().with_private_nodes(&private))?;

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let default = tree.aria(AriaAsk::default_mode())?;

    let expected_bullets = [
        ("in-curr-pw", 8),
        ("in-new-pw", 9),
        ("in-otp", 6),
        ("in-cc-num", 16),
        ("in-cc-csc", 3),
        ("ta-curr-pw", 7),
        ("ta-new-pw", 10),
        ("ta-otp", 5),
        ("ta-cc-num", 15),
        ("ta-cc-csc", 4),
        ("custom-css", 6),
        ("custom-js", 7),
        ("type-flip-ch", 12),
        ("future-99", 11),
    ];
    for (label, count) in expected_bullets {
        let node = find_by_role_name(ai.nodes(), "textbox", Some(label))
            .ok_or_else(|| format!("missing redacted textbox {label:?}"))?;
        assert_eq!(node.text, Some("•".repeat(count)));
    }
    for empty_label in ["empty-pw", "empty-pay", "empty-css", "empty-otp"] {
        let node = find_by_role_name(ai.nodes(), "textbox", Some(empty_label))
            .ok_or_else(|| format!("missing empty-private textbox {empty_label:?}"))?;
        assert_eq!(node.text, None);
    }
    let public = find_by_role_name(ai.nodes(), "textbox", Some("public-note"))
        .ok_or("missing public-note")?;
    assert_eq!(public.text.as_deref(), Some("public-ok"));

    let cache = NameCache::new(&tree);
    assert_zero_canary_surface(&tree, &ai, &default, &refs, &private, &cache)?;
    Ok(())
}

fn text_input_node(dom_id: i32, y: i32, label: &str, value: &str) -> NodeBuilder {
    NodeBuilder::form_control(FormControlType::InputText)
        .dom_id(dom_id)
        .tag("input")
        .aria_label(label)
        .field_value(value)
        .bounding_box(0, y, 180, 24)
}

fn build_host_private_iframes() -> Vec<NodeBuilder> {
    let same_origin = FrameBuilder::new("doc-same-origin")
        .origin("https://app.example", false)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(text_input_node(
                    77,
                    0,
                    "Moved Secret",
                    &format!("{CANARY}-moved"),
                ))
                .child(text_input_node(
                    88,
                    30,
                    "NonOpaque Public",
                    "public-non-opaque",
                )),
        );
    let cross_origin = FrameBuilder::new("doc-cross-origin")
        .origin("https://public.example", false)
        .root(NodeBuilder::root().dom_id(1).child(text_input_node(
            77,
            0,
            "CrossOrigin Public",
            "public-cross-origin",
        )));
    let opaque_origin = FrameBuilder::new("doc-opaque-origin")
        .origin("https://app.example", true)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(text_input_node(
                    77,
                    0,
                    "Opaque Public",
                    "public-opaque-origin",
                ))
                .child(text_input_node(
                    88,
                    30,
                    "Opaque Secret",
                    &format!("{CANARY}-opaque"),
                )),
        );
    let other_nonce = FrameBuilder::new("doc-other-nonce")
        .origin("opaque-nonce-other", true)
        .root(NodeBuilder::root().dom_id(1).child(text_input_node(
            88,
            0,
            "OtherNonce Public",
            "public-other-nonce",
        )));
    // No security origin, and a URL on another site than the marks: the document's site is
    // unknown, and a URL fallback would wrongly read it as public.example.
    let no_origin = FrameBuilder::new("doc-no-origin")
        .url("https://public.example/embed")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(text_input_node(
                    77,
                    0,
                    "NoOrigin Secret",
                    &format!("{CANARY}-no-origin"),
                ))
                .child(text_input_node(
                    89,
                    30,
                    "NoOrigin Public",
                    "public-no-origin",
                ))
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .dom_id(90)
                        .tag("button")
                        .bounding_box(0, 60, 140, 24)
                        .labelledby(vec![BrowserOsRef {
                            dom_node_id: Some(96),
                            unrendered_text: Some(format!("{CANARY}-fallback")),
                        }]),
                ),
        );
    vec![
        NodeBuilder::iframe(same_origin).bounding_box(0, 220, 300, 80),
        NodeBuilder::iframe(cross_origin).bounding_box(0, 310, 300, 80),
        NodeBuilder::iframe(opaque_origin).bounding_box(0, 400, 300, 80),
        NodeBuilder::iframe(other_nonce).bounding_box(0, 490, 300, 80),
        NodeBuilder::iframe(no_origin).bounding_box(0, 580, 300, 100),
    ]
}

fn site(origin: &str) -> Result<SiteKey, Box<dyn Error>> {
    SiteKey::from_origin(origin).ok_or_else(|| format!("{origin:?} must read as an origin").into())
}

/// The site of a document whose security origin is the opaque `nonce`: the only way to name an
/// opaque site is through a document APC reported, as the tab's mirror does when it seeds one.
fn opaque_site(nonce: &str) -> Result<SiteKey, Box<dyn Error>> {
    let tree = build_tree(
        ApcBuilder::new("doc-nonce-probe")
            .main_origin(nonce, true)
            .root(NodeBuilder::root().dom_id(1))
            .build(),
        BuildOptions::new(),
    )?;
    let doc = tree.main_document().ok_or("main doc")?;
    SiteKey::of_doc(doc).ok_or_else(|| format!("nonce {nonce:?} must be a site").into())
}

fn build_host_private_multiframe_apc() -> AnnotatedPageContent {
    let ce_secret = format!("{CANARY}-😀");
    let lbl_secret = format!("{CANARY}-lbl");
    let main_root = NodeBuilder::root()
        .dom_id(1)
        .child(text_input_node(10, 0, "Host TypeFlip Emoji", "p😀s"))
        .child(
            text_input_node(11, 30, "Host Input", &format!("{CANARY}-in"))
                .child(NodeBuilder::text(&format!("{CANARY}-in-child"))),
        )
        .child(
            NodeBuilder::form_control(FormControlType::TextArea)
                .dom_id(12)
                .tag("textarea")
                .aria_label("Host Textarea")
                .field_value(&format!("{CANARY}-ta"))
                .bounding_box(0, 60, 180, 40)
                .child(NodeBuilder::text(&format!("{CANARY}-ta-child"))),
        )
        .child(
            NodeBuilder::form_control(FormControlType::ContentEditable)
                .dom_id(20)
                .role("textbox")
                .aria_label("Host Editable")
                .bounding_box(0, 110, 180, 30)
                .child(NodeBuilder::text(&ce_secret).dom_text(&ce_secret)),
        )
        .child(
            NodeBuilder::element("span")
                .dom_id(30)
                .bounding_box(0, 150, 140, 20)
                .child(NodeBuilder::text(&lbl_secret).dom_text(&lbl_secret)),
        )
        .child(
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .dom_id(31)
                .tag("button")
                .bounding_box(0, 180, 140, 24)
                .labelledby(vec![BrowserOsRef {
                    dom_node_id: Some(30),
                    unrendered_text: None,
                }]),
        )
        .children(build_host_private_iframes());

    ApcBuilder::new("doc-main")
        .main_origin("https://app.example", false)
        .root(main_root)
        .build()
}

fn assert_textbox_text(
    nodes: &[AriaNode],
    name: &str,
    expected: &str,
) -> Result<(), Box<dyn Error>> {
    let node = find_by_role_name(nodes, "textbox", Some(name))
        .ok_or_else(|| format!("missing textbox {name:?}"))?;
    assert_eq!(node.text.as_deref(), Some(expected));
    Ok(())
}

#[test]
fn privacy_host_private_origin_isolation_contenteditable_and_labelledby()
-> Result<(), Box<dyn Error>> {
    let app = site("https://app.example")?;
    let mut private = PrivateNodes::new();
    // doc-main is on https://app.example, so its boxes are marked on that site; so is 77, the box
    // the page later moves into the same-origin iframe, and 96, a node of the origin-less frame.
    for dom_id in [10, 11, 12, 20, 30, 77, 96] {
        private.mark(Some(app.clone()), DomNodeId::new(dom_id));
    }
    private.mark(
        Some(opaque_site("https://app.example")?),
        DomNodeId::new(88),
    );

    let tree = build_tree(
        build_host_private_multiframe_apc(),
        BuildOptions::new().with_private_nodes(&private),
    )?;
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let default = tree.aria(AriaAsk::default_mode())?;

    // Emoji surrogate pair "p😀s" has 4 UTF-16 code units -> 4 bullets.
    assert_textbox_text(ai.nodes(), "Host TypeFlip Emoji", "••••")?;
    assert_textbox_text(
        ai.nodes(),
        "Host Editable",
        &"•".repeat(format!("{CANARY}-😀").encode_utf16().count()),
    )?;
    assert_textbox_text(
        ai.nodes(),
        "Moved Secret",
        &"•".repeat(format!("{CANARY}-moved").encode_utf16().count()),
    )?;
    // 77 is marked on app.example: masked in the same-origin iframe (another document token),
    // clear on public.example, and clear in the opaque document whose nonce spells app.example.
    assert_textbox_text(ai.nodes(), "CrossOrigin Public", "public-cross-origin")?;
    assert_textbox_text(ai.nodes(), "Opaque Public", "public-opaque-origin")?;
    // 88 is marked on the opaque nonce only: clear in the tuple app.example document and under
    // another nonce.
    assert_textbox_text(ai.nodes(), "NonOpaque Public", "public-non-opaque")?;
    assert_textbox_text(ai.nodes(), "OtherNonce Public", "public-other-nonce")?;
    assert_textbox_text(
        ai.nodes(),
        "Opaque Secret",
        &"•".repeat(format!("{CANARY}-opaque").encode_utf16().count()),
    )?;
    // The origin-less iframe's site is unknown: a marked id masks, an unmarked one stays clear.
    assert_textbox_text(
        ai.nodes(),
        "NoOrigin Secret",
        &"•".repeat(format!("{CANARY}-no-origin").encode_utf16().count()),
    )?;
    assert_textbox_text(ai.nodes(), "NoOrigin Public", "public-no-origin")?;

    let expected_btn_bullets = "•".repeat(format!("{CANARY}-lbl").encode_utf16().count());
    let labelled_btn = find_by_role_name(ai.nodes(), "button", Some(&expected_btn_bullets))
        .ok_or("button aria-labelledby host-private box must carry bullets")?;
    assert!(labelled_btn.r#ref.is_some());

    let cache = NameCache::new(&tree);
    let btn_id = node_id_of(&tree, "doc-main", 31)?;
    assert_eq!(
        cache.name(btn_id, NameVisibility::VisibleOnly).text(),
        expected_btn_bullets
    );
    let fallback_btn = node_id_of(&tree, "doc-no-origin", 90)?;
    assert_eq!(
        cache.name(fallback_btn, NameVisibility::VisibleOnly).text(),
        "•".repeat(format!("{CANARY}-fallback").encode_utf16().count()),
        "a labelledby fallback into a marked node of an origin-less document prints bullets"
    );
    assert_zero_canary_surface(&tree, &ai, &default, &refs, &private, &cache)?;
    Ok(())
}

#[test]
fn privacy_an_unknown_site_mark_masks_its_id_in_every_document() -> Result<(), Box<dyn Error>> {
    let mut private = PrivateNodes::new();
    private.mark(None, DomNodeId::new(77));

    let tree = build_tree(
        build_host_private_multiframe_apc(),
        BuildOptions::new().with_private_nodes(&private),
    )?;
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;

    let bullets = |value: &str| "•".repeat(value.encode_utf16().count());
    for (name, value) in [
        ("Moved Secret", format!("{CANARY}-moved")),
        ("CrossOrigin Public", "public-cross-origin".to_owned()),
        ("Opaque Public", "public-opaque-origin".to_owned()),
        ("NoOrigin Secret", format!("{CANARY}-no-origin")),
    ] {
        assert_textbox_text(ai.nodes(), name, &bullets(&value))?;
    }
    assert_textbox_text(ai.nodes(), "NonOpaque Public", "public-non-opaque")?;
    assert_textbox_text(ai.nodes(), "NoOrigin Public", "public-no-origin")?;
    Ok(())
}

fn build_boxes_and_roots_tree() -> Result<SnapshotTree, Box<dyn Error>> {
    let cross_child = FrameBuilder::new("doc-cross")
        .url("https://cross.example/embed")
        .origin("https://cross.example", false)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .bounding_box(200, 100, 600, 400)
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .dom_id(10)
                        .tag("button")
                        .bounding_box(240, 70, 120, 40)
                        .child(NodeBuilder::text("Scrolled Child")),
                ),
        );

    let apc = ApcBuilder::new("doc-boxes")
        .viewport(0, 0, 1000, 500)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .bounding_box(0, 0, 2000, 1000)
                .child(
                    NodeBuilder::element("section")
                        .dom_id(10)
                        .aria_label("Filters")
                        .bounding_box(0, 0, 400, 100)
                        .child(
                            NodeBuilder::form_control(FormControlType::ButtonButton)
                                .dom_id(11)
                                .tag("button")
                                .bounding_box(50, -30, 160, 40)
                                .child(NodeBuilder::text("Scrolled Main")),
                        ),
                )
                .child(visible_button(12, 60, "Outside Sibling"))
                .child(
                    NodeBuilder::iframe(cross_child)
                        .dom_id(20)
                        .bounding_box(200, 100, 600, 400),
                ),
        )
        .build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn aria_root_document_and_subtree_and_boxes_dpr_iframe_scroll() -> Result<(), Box<dyn Error>> {
    let tree = build_boxes_and_roots_tree()?;
    let mut refs = TabRefTable::new();
    let full = tree.aria(AriaAsk::ai(&mut refs).with_boxes(true))?;

    let main_btn = find_by_role_name(full.nodes(), "button", Some("Scrolled Main"))
        .ok_or("missing Scrolled Main")?;
    assert_eq!(
        main_btn.r#box,
        Some(NodeBox {
            x: Number::from(25),
            y: Number::from(-15),
            width: Number::from(80),
            height: Number::from(20),
        })
    );
    let child_btn = find_by_role_name(full.nodes(), "button", Some("Scrolled Child"))
        .ok_or("missing Scrolled Child")?;
    assert_eq!(
        child_btn.r#box,
        Some(NodeBox {
            x: Number::from(20),
            y: Number::from(-15),
            width: Number::from(60),
            height: Number::from(20),
        })
    );
    let pw_boxes = neo_snapshot::render(&full.to_snapshot(), Call::Playwright).into_text();
    assert!(pw_boxes.contains("[box=25,-15,80,20]"));
    assert!(pw_boxes.contains("[box=20,-15,60,20]"));

    let sibling_ref_before = find_by_role_name(full.nodes(), "button", Some("Outside Sibling"))
        .and_then(|n| n.r#ref.clone())
        .ok_or("missing Outside Sibling ref")?;

    // AriaRoot::Document emits only the requested iframe document.
    let child_doc_id = tree
        .document_by_token(&doc_token("doc-cross")?)
        .ok_or("missing doc-cross")?
        .id();
    assert_eq!(child_doc_id, DocId::new(1));
    let doc_only = tree.aria(AriaAsk::ai(&mut refs).with_root(AriaRoot::Document(child_doc_id)))?;
    assert!(find_by_role_name(doc_only.nodes(), "button", Some("Scrolled Child")).is_some());
    assert!(find_by_role_name(doc_only.nodes(), "button", Some("Scrolled Main")).is_none());

    // AriaRoot::Subtree emits only the requested section without dropping sibling ref identities.
    let section_id = node_id_of(&tree, "doc-boxes", 10)?;
    let sub_only = tree.aria(AriaAsk::ai(&mut refs).with_root(AriaRoot::Subtree(section_id)))?;
    assert!(find_by_role_name(sub_only.nodes(), "region", Some("Filters")).is_some());
    assert!(find_by_role_name(sub_only.nodes(), "button", Some("Scrolled Main")).is_some());
    assert!(find_by_role_name(sub_only.nodes(), "button", Some("Outside Sibling")).is_none());

    let full_again = tree.aria(AriaAsk::ai(&mut refs))?;
    let sibling_ref_after =
        find_by_role_name(full_again.nodes(), "button", Some("Outside Sibling"))
            .and_then(|n| n.r#ref.as_deref())
            .ok_or("missing Outside Sibling ref after subtree capture")?;
    assert_eq!(sibling_ref_after, sibling_ref_before);
    Ok(())
}

fn nested_groups(depth: usize, leaf: NodeBuilder) -> NodeBuilder {
    let mut current = leaf;
    for _ in 0..depth {
        current = NodeBuilder::container()
            .role("group")
            .bounding_box(0, 0, 100, 100)
            .child(current);
    }
    current
}

#[test]
fn depth_cap_refusal_prunes_grandchild_partial_and_renders_comment() -> Result<(), Box<dyn Error>> {
    let mid_child = FrameBuilder::new("doc-mid")
        .url("https://mid.example/deep")
        .root(
            NodeBuilder::root().child(nested_groups(
                76,
                NodeBuilder::container()
                    .role("group")
                    .bounding_box(0, 0, 100, 100)
                    .child(visible_button(50, 0, "Mid Leaf"))
                    .child(
                        NodeBuilder::partial_iframe(PartialIframeSpec::Timeout)
                            .href("https://grandchild.example/slow")
                            .bounding_box(0, 30, 80, 40),
                    ),
            )),
        );

    let apc = ApcBuilder::new("doc-outer")
        .root(NodeBuilder::root().child(nested_groups(
            76,
            NodeBuilder::iframe(mid_child).bounding_box(0, 0, 100, 100),
        )))
        .build();
    let tree = build_tree(apc, BuildOptions::new())?;

    let mut refs = TabRefTable::new();
    let out = tree.aria(AriaAsk::ai(&mut refs))?;
    assert!(neo_snapshot::json_depth(out.nodes()) <= MAX_JSON_DEPTH);
    assert!(find_by_role_name(out.nodes(), "button", Some("Mid Leaf")).is_none());

    let iframe = find_by_role_name(out.nodes(), "iframe", None)
        .ok_or("outer iframe must remain as a leaf")?;
    assert!(iframe.children.is_empty());

    assert_eq!(
        out.partial().len(),
        1,
        "refusing mid iframe must prune its grandchild partial note"
    );
    let note = out.partial().first().ok_or("missing refusal partial")?;
    assert_eq!(note.frame, "https://mid.example/deep");
    assert!(!note.r#ref.is_empty());
    assert!(note.reason.contains("more than a snapshot carries"));

    let snap = out.into_snapshot();
    for text in [
        neo_snapshot::render(&snap, Call::Playwright).into_text(),
        neo_snapshot::render(&snap, Call::Full { max_tokens: None }).into_text(),
        neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text(),
    ] {
        assert!(text.contains("# partial: https://mid.example/deep not read ("));
        assert!(!text.contains("grandchild.example"));
        assert!(!text.contains("Mid Leaf"));
    }
    Ok(())
}

fn build_modal_inert_tree() -> Result<SnapshotTree, Box<dyn Error>> {
    let dialog = NodeBuilder::element("dialog")
        .dom_id(30)
        .inert(false)
        .labelledby(vec![BrowserOsRef {
            dom_node_id: Some(40),
            unrendered_text: None,
        }])
        .bounding_box(100, 100, 400, 240)
        .child(
            NodeBuilder::element("h2")
                .dom_id(40)
                .inert(false)
                .bounding_box(110, 110, 260, 28)
                .child(NodeBuilder::text("Confirm delete")),
        )
        .child(
            NodeBuilder::element("p")
                .inert(false)
                .bounding_box(110, 145, 260, 20)
                .child(NodeBuilder::text("Delete this project?")),
        )
        .child(visible_button(41, 180, "Modal confirm").inert(false))
        .child(visible_button(42, 210, "Modal cancel").inert(false))
        .child(
            NodeBuilder::container()
                .inert(true)
                .bounding_box(110, 250, 260, 60)
                .child(
                    NodeBuilder::element("a")
                        .href("#inert")
                        .inert(true)
                        .bounding_box(115, 255, 120, 20)
                        .child(NodeBuilder::text("Inert link inside modal")),
                )
                .child(visible_button(43, 280, "Inert btn inside modal").inert(true)),
        );

    let body = NodeBuilder::element("body")
        .inert(true)
        .bounding_box(0, 0, 800, 600)
        .child(
            NodeBuilder::element("h1")
                .inert(true)
                .bounding_box(0, 0, 300, 32)
                .child(NodeBuilder::text("Page behind modal")),
        )
        .child(visible_button(10, 40, "Behind modal").inert(true))
        .child(
            NodeBuilder::element("p")
                .inert(true)
                .bounding_box(0, 70, 300, 20)
                .child(NodeBuilder::text("Background paragraph")),
        )
        .child(
            NodeBuilder::container()
                .inert(true)
                .bounding_box(0, 100, 300, 40)
                .child(visible_button(11, 105, "Outside inert div btn").inert(true)),
        )
        .child(dialog);

    let apc = ApcBuilder::new("doc-modal")
        .main_focused_node_id(41)
        .root(NodeBuilder::root().child(body))
        .build();
    build_tree(apc, BuildOptions::new())
}

#[test]
fn inert_modal_dialog_escape_rendered_through_neo_page() -> Result<(), Box<dyn Error>> {
    let tree = build_modal_inert_tree()?;
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let confirm = find_by_role_name(ai.nodes(), "button", Some("Modal confirm"))
        .ok_or("missing Modal confirm")?;
    assert_eq!(confirm.active, Some(true));
    assert_eq!(confirm.inert, None);
    assert!(
        find_by_role_name(ai.nodes(), "dialog", None).is_some(),
        "the dialog's name only repeats its printed heading, so ai mode drops it"
    );

    let snap = ai.into_snapshot();
    let page = neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text();
    assert!(page.starts_with("# not listed:"));
    assert!(page.contains("- dialog [ref="), "{page}");
    assert!(page.contains("heading \"Confirm delete\""), "{page}");
    assert!(page.contains("Modal confirm"));
    assert!(page.contains("Modal cancel"));
    assert!(page.contains("Delete this project?"));
    for omitted in [
        "Behind modal",
        "Background paragraph",
        "Outside inert div btn",
        "Inert link inside modal",
        "Inert btn inside modal",
    ] {
        assert!(!page.contains(omitted), "inert content leaked: {omitted:?}");
    }

    let with_inert = neo_snapshot::render(
        &snap,
        Call::Page(&Passed {
            inert: Some(true),
            ..Passed::default()
        }),
    )
    .into_text();
    assert!(!with_inert.contains("# not listed:"));
    assert!(with_inert.contains("Behind modal"));
    assert!(with_inert.contains("Inert btn inside modal"));

    let full = neo_snapshot::render(&snap, Call::Full { max_tokens: None }).into_text();
    assert!(full.contains("# inert:"));
    Ok(())
}

fn build_deep_group_chain(depth: usize) -> AnnotatedPageContent {
    let mut current: Option<ContentNode> = None;
    for index in (1..=depth).rev() {
        let is_root = index == 1;
        let attribute_type = if is_root {
            ContentAttributeType::Root
        } else {
            ContentAttributeType::Container
        };
        let mut children_nodes = Vec::new();
        if let Some(child) = current.take() {
            children_nodes.push(child);
        }
        let rect = BoundingRect {
            x: Some(0),
            y: Some(0),
            width: Some(100),
            height: Some(100),
            is_screenshot_relative: Some(false),
        };
        let browseros = (!is_root).then(|| {
            Box::new(BrowserOsContentAttributes {
                role: Some("group".to_owned()),
                ..BrowserOsContentAttributes::default()
            })
        });
        current = Some(ContentNode {
            children_nodes,
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: i32::try_from(index).ok(),
                attribute_type: Some(i32::from(attribute_type)),
                geometry: Some(Geometry {
                    outer_bounding_box: Some(rect),
                    visible_bounding_box: Some(rect),
                    ..Geometry::default()
                }),
                browseros,
                ..ContentAttributes::default()
            }),
        });
    }

    AnnotatedPageContent {
        version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
        mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        root_node: current,
        main_frame_data: Some(FrameData {
            document_identifier: Some(DocumentIdentifier {
                serialized_token: Some("doc-deep-25k".to_owned()),
            }),
            ..FrameData::default()
        }),
        browseros: Some(BrowserOsPageContentData {
            contract_version: Some(MIN_CONTRACT_VERSION),
            subframe_timeout_ms: Some(2_000),
        }),
        ..AnnotatedPageContent::default()
    }
}

#[test]
fn deep_25k_chain_renders_on_2mib_stack_without_overflow() -> Result<(), Box<dyn Error>> {
    let handle = ThreadBuilder::new()
        .name("tree-emit-refs-25k".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| -> Result<(), String> {
            let group_tree =
                SnapshotTree::build(build_deep_group_chain(25_000), BuildOptions::new())
                    .map_err(|err| err.to_string())?;
            let mut refs = TabRefTable::new();
            let refused = group_tree.aria(AriaAsk::ai(&mut refs));
            if !refused.is_err_and(|too_deep| too_deep.levels() > 24_000) {
                return Err(
                    "a 25k-level page must be refused, as today's snapshot refuses it".to_owned(),
                );
            }
            let ai = group_tree
                .aria(AriaAsk::ai(&mut refs).with_depth(Some(100)))
                .map_err(|err| err.to_string())?;
            if ai.nodes().len() != 1 {
                return Err(format!("expected 1 root group, got {}", ai.nodes().len()));
            }
            let depth = 1 + neo_snapshot::json_depth(ai.nodes());
            if depth > MAX_JSON_DEPTH {
                return Err(format!("json depth {depth} exceeded {MAX_JSON_DEPTH}"));
            }
            let snap = ai.into_snapshot();
            let pw = neo_snapshot::render(&snap, Call::Playwright).into_text();
            let full = neo_snapshot::render(&snap, Call::Full { max_tokens: None }).into_text();
            let page = neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text();
            if !pw.contains("group") || !full.contains("group") || page.len() > pw.len() {
                return Err("unexpected rendered output for 25k group chain".to_owned());
            }

            let plain_tree =
                SnapshotTree::build(ApcBuilder::deep_chain(25_000), BuildOptions::new())
                    .map_err(|err| err.to_string())?;
            let plain_snap = plain_tree
                .aria(AriaAsk::ai(&mut refs))
                .map_err(|err| err.to_string())?
                .into_snapshot();
            let plain_pw = neo_snapshot::render(&plain_snap, Call::Playwright).into_text();
            if plain_pw.len() > pw.len() {
                return Err("unexpected rendered output for 25k plain chain".to_owned());
            }
            Ok(())
        })?;

    let outcome = match handle.join() {
        Ok(result) => result,
        Err(payload) => return Err(format!("25k stack thread panicked: {payload:?}").into()),
    };
    outcome.map_err(Into::into)
}

fn build_unrendered_relation_apc(
    lbl_secret: &str,
    desc_secret: &str,
    err_secret: &str,
) -> AnnotatedPageContent {
    let pw_secret = format!("{CANARY}-PW");
    ApcBuilder::new("doc-rel-mask")
        .main_origin("https://vault.example", false)
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .dom_id(10)
                        .tag("button")
                        .aria_invalid("true")
                        .labelledby(vec![BrowserOsRef {
                            dom_node_id: Some(100),
                            unrendered_text: Some(lbl_secret.to_owned()),
                        }])
                        .describedby(vec![BrowserOsRef {
                            dom_node_id: Some(101),
                            unrendered_text: Some(desc_secret.to_owned()),
                        }])
                        .errormessage(vec![BrowserOsRef {
                            dom_node_id: Some(102),
                            unrendered_text: Some(err_secret.to_owned()),
                        }])
                        .bounding_box(0, 0, 120, 24),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputPassword)
                        .dom_id(20)
                        .tag("input")
                        .aria_label("Vault key")
                        .redaction(RedactionDecision::RedactedHasBeenPassword)
                        .redacted_value_length(6)
                        .field_value(&pw_secret)
                        .bounding_box(0, 30, 160, 24),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .dom_id(21)
                        .tag("button")
                        .labelledby(vec![BrowserOsRef {
                            dom_node_id: Some(20),
                            unrendered_text: Some(format!("{CANARY}-CHREF")),
                        }])
                        .bounding_box(0, 60, 120, 24),
                ),
        )
        .build()
}

#[test]
fn privacy_unrendered_relation_fallbacks_are_masked_without_canary_leaks()
-> Result<(), Box<dyn Error>> {
    let lbl_secret = format!("{CANARY}-LBL");
    let desc_secret = format!("{CANARY}-DESC");
    let err_secret = format!("{CANARY}-ERR");
    let apc = build_unrendered_relation_apc(&lbl_secret, &desc_secret, &err_secret);

    // doc-rel-mask is on https://vault.example: the relation targets are marked on that site,
    // whichever document the mark was first made in.
    let vault = site("https://vault.example")?;
    let mut private = PrivateNodes::new();
    for dom_id in [100, 101, 102] {
        private.mark(Some(vault.clone()), DomNodeId::new(dom_id));
    }

    let tree = build_tree(apc, BuildOptions::new().with_private_nodes(&private))?;
    let cache = NameCache::new(&tree);
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let default = tree.aria(AriaAsk::default_mode())?;
    assert_zero_canary_surface(&tree, &ai, &default, &refs, &private, &cache)?;

    let btn_id = node_id_of(&tree, "doc-rel-mask", 10)?;
    let expected_lbl = "•".repeat(lbl_secret.encode_utf16().count());
    let expected_desc = "•".repeat(desc_secret.encode_utf16().count());
    let expected_err = "•".repeat(err_secret.encode_utf16().count());
    assert_eq!(
        cache.name(btn_id, NameVisibility::VisibleOnly).text(),
        expected_lbl
    );
    assert_eq!(
        cache
            .description(btn_id, NameVisibility::VisibleOnly)
            .text(),
        expected_desc
    );
    assert_eq!(cache.error_message(btn_id), expected_err);
    assert!(find_by_role_name(ai.nodes(), "button", Some(&expected_lbl)).is_some());

    let ch_btn_id = node_id_of(&tree, "doc-rel-mask", 21)?;
    assert_eq!(
        cache.name(ch_btn_id, NameVisibility::VisibleOnly).text(),
        "••••••"
    );
    Ok(())
}

fn labelled_button(dom_id: i32, y: i32, target_dom: i32) -> NodeBuilder {
    NodeBuilder::form_control(FormControlType::ButtonButton)
        .dom_id(dom_id)
        .tag("button")
        .labelledby(vec![BrowserOsRef {
            dom_node_id: Some(target_dom),
            unrendered_text: None,
        }])
        .bounding_box(0, y, 120, 24)
}

fn build_masked_select_apc(
    host_opt: &str,
    host_val_only: &str,
    nested_opt: &str,
) -> AnnotatedPageContent {
    let ch_select = NodeBuilder::form_control(FormControlType::SelectOne)
        .dom_id(10)
        .tag("select")
        .aria_label("Card Expiry")
        .redaction(RedactionDecision::RedactedIsSensitivePaymentField)
        .redacted_value_length(4)
        .select_option(select_opt(
            &format!("{CANARY}-CHV1"),
            &format!("{CANARY}-CH1"),
            true,
        ))
        .select_option(select_opt(
            &format!("{CANARY}-CHV2"),
            &format!("{CANARY}-CH2"),
            false,
        ))
        .bounding_box(0, 0, 140, 24);
    let host_select = NodeBuilder::form_control(FormControlType::SelectOne)
        .dom_id(20)
        .tag("select")
        .select_option(select_opt(&format!("{CANARY}-HV1"), host_opt, true))
        .select_option(SelectOption {
            text: None,
            value: Some(host_val_only.to_owned()),
            is_selected: Some(false),
            is_disabled: Some(false),
        })
        .bounding_box(0, 60, 140, 24);
    let nested_div = NodeBuilder::container()
        .dom_id(30)
        .bounding_box(0, 120, 200, 40)
        .child(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .dom_id(31)
                .tag("select")
                .aria_label("Nested Secret Select")
                .select_option(select_opt(&format!("{CANARY}-NV"), nested_opt, true))
                .bounding_box(0, 120, 140, 24),
        );

    ApcBuilder::new("doc-select-mask")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(ch_select)
                .child(labelled_button(11, 30, 10))
                .child(host_select)
                .child(labelled_button(21, 90, 20))
                .child(nested_div),
        )
        .build()
}

#[test]
fn privacy_masked_select_sanitizes_synthetic_and_embedded_option_names()
-> Result<(), Box<dyn Error>> {
    let host_opt = format!("{CANARY}-HOST");
    let host_val_only = format!("{CANARY}-VALONLY");
    let nested_opt = format!("{CANARY}-NESTED");
    let apc = build_masked_select_apc(&host_opt, &host_val_only, &nested_opt);

    // doc-select-mask has no origin, so these are unknown-site marks.
    let mut private = PrivateNodes::new();
    private.mark(None, DomNodeId::new(20));
    private.mark(None, DomNodeId::new(30));
    let tree = build_tree(apc, BuildOptions::new().with_private_nodes(&private))?;
    let cache = NameCache::new(&tree);
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;
    let default = tree.aria(AriaAsk::default_mode())?;
    assert_zero_canary_surface(&tree, &ai, &default, &refs, &private, &cache)?;

    let ch_select = find_by_role_name(ai.nodes(), "combobox", Some("Card Expiry"))
        .ok_or("missing Chromium-redacted select")?;
    assert_eq!(ch_select.text.as_deref(), Some("••••"));
    assert!(ch_select.children.is_empty());

    let host_opt_bullets = "•".repeat(host_opt.encode_utf16().count());
    let host_val_bullets = "•".repeat(host_val_only.encode_utf16().count());
    let nested_bullets = "•".repeat(nested_opt.encode_utf16().count());
    assert!(find_by_role_name(ai.nodes(), "option", Some(&host_opt_bullets)).is_some());
    assert!(find_by_role_name(ai.nodes(), "option", Some(&host_val_bullets)).is_some());
    assert!(find_by_role_name(ai.nodes(), "option", Some(&nested_bullets)).is_some());
    let btn21 = node_id_of(&tree, "doc-select-mask", 21)?;
    assert_eq!(
        cache.name(btn21, NameVisibility::VisibleOnly).text(),
        host_opt_bullets
    );
    Ok(())
}

fn build_inert_boundaries_apc() -> AnnotatedPageContent {
    let dialog = NodeBuilder::element("dialog")
        .dom_id(20)
        .inert(false)
        .aria_label("Active Modal")
        .bounding_box(50, 50, 320, 220)
        .child(visible_button(21, 70, "Modal Action").inert(false))
        .child(
            NodeBuilder::container()
                .dom_id(22)
                .role("generic")
                .inert(false)
                .bounding_box(60, 110, 200, 30)
                .child(
                    NodeBuilder::container()
                        .dom_id(23)
                        .inert(true)
                        .bounding_box(60, 110, 200, 30)
                        .child(NodeBuilder::text("Inert inside generic").inert(true)),
                ),
        )
        .child(
            NodeBuilder::container()
                .dom_id(24)
                .inert(false)
                .aria_label("Repeated inert copy")
                .bounding_box(60, 150, 200, 30)
                .child(
                    NodeBuilder::container()
                        .dom_id(25)
                        .inert(true)
                        .bounding_box(60, 150, 200, 30)
                        .child(NodeBuilder::text("Repeated inert copy").inert(true)),
                ),
        );
    let body = NodeBuilder::element("body")
        .dom_id(2)
        .inert(true)
        .bounding_box(0, 0, 800, 600)
        .child(
            NodeBuilder::container()
                .dom_id(10)
                .inert(true)
                .bounding_box(0, 0, 300, 30)
                .child(NodeBuilder::text("Background plain div text").inert(true)),
        )
        .child(dialog);
    ApcBuilder::new("doc-inert-bounds")
        .root(NodeBuilder::root().dom_id(1).child(body))
        .build()
}

#[test]
fn inert_boundaries_survive_plain_container_pruning_and_generic_text_inlining()
-> Result<(), Box<dyn Error>> {
    let tree = build_tree(build_inert_boundaries_apc(), BuildOptions::new())?;
    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs))?;

    let action = find_by_role_name(ai.nodes(), "button", Some("Modal Action"))
        .ok_or("missing live Modal Action button")?;
    assert_eq!(action.inert, None);

    let bg = find_by_role_text(ai.nodes(), "generic", "Background plain div text")
        .ok_or("missing inert background plain div")?;
    assert_eq!(bg.inert, Some(true));

    let nested_inert = find_by_role_text(ai.nodes(), "generic", "Inert inside generic")
        .ok_or("missing inert child inside live generic")?;
    assert_eq!(nested_inert.inert, Some(true));

    let repeated = find_by_role_text(ai.nodes(), "generic", "Repeated inert copy")
        .ok_or("inert child repeating live parent name must keep its inert node")?;
    assert_eq!(repeated.inert, Some(true));

    let snap = ai.into_snapshot();
    let page = neo_snapshot::render(&snap, Call::Page(&Passed::default())).into_text();
    assert!(page.starts_with("# not listed:"));
    assert!(page.contains("Modal Action"));
    assert!(!page.contains("Background plain div text"));
    assert!(!page.contains("Inert inside generic"));

    let with_inert = neo_snapshot::render(
        &snap,
        Call::Page(&Passed {
            inert: Some(true),
            ..Passed::default()
        }),
    )
    .into_text();
    assert!(with_inert.contains("Background plain div text"));
    assert!(with_inert.contains("Inert inside generic"));
    Ok(())
}
