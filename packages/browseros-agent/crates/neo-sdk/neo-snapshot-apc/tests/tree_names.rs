//! Offline hand-built and Playwright corpus integration tests for
//! [`neo_snapshot_apc::tree`] ACCNAME accessible names, descriptions, error messages,
//! [`neo_snapshot_apc::tree::NameCache`] concurrency, QuickJS whitespace parity, and
//! 100k-depth stack safety.

use std::error::Error;
use std::thread;

use neo_snapshot_apc::proto::{
    AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion, AnnotatedRole,
    BrowserOsContentAttributes, BrowserOsPageContentData, BrowserOsRef, ContentAttributeType,
    ContentAttributes, ContentData, ContentNode, DocumentIdentifier, FormControlType, FrameData,
    ImageInfo, RedactionDecision, SelectOption, TextInfo,
};
use neo_snapshot_apc::tree::{
    self, ApcBuilder, BuildOptions, Description, DocToken, DomNodeId, FrameBuilder,
    MIN_CONTRACT_VERSION, Name, NameCache, NameVisibility, NodeBuilder, NodeId, NodeKey, Role,
    SnapshotTree, TreeNode,
};
use rquickjs::{Context, Runtime};
use serde::Deserialize;

fn build_tree(root: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-main").root(root).build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn node_by_dom<'a>(
    tree: &'a SnapshotTree,
    doc_token: &str,
    dom_id: i32,
) -> Result<&'a TreeNode, Box<dyn Error>> {
    let token = DocToken::new(doc_token).ok_or("doc token must be non-empty")?;
    let key = NodeKey::new(token, DomNodeId::new(dom_id));
    tree.node_by_key(&key)
        .ok_or_else(|| format!("missing node ({doc_token}, {dom_id})").into())
}

fn node_id(tree: &SnapshotTree, doc_token: &str, dom_id: i32) -> Result<NodeId, Box<dyn Error>> {
    Ok(node_by_dom(tree, doc_token, dom_id)?.id())
}

fn visible_name(tree: &SnapshotTree, dom_id: i32) -> Result<String, Box<dyn Error>> {
    Ok(tree
        .accessible_name(
            node_id(tree, "doc-main", dom_id)?,
            NameVisibility::VisibleOnly,
        )
        .into_text())
}

fn hidden_name(tree: &SnapshotTree, dom_id: i32) -> Result<String, Box<dyn Error>> {
    Ok(tree
        .accessible_name(
            node_id(tree, "doc-main", dom_id)?,
            NameVisibility::IncludeHidden,
        )
        .into_text())
}

fn assert_visible_names(
    tree: &SnapshotTree,
    cases: &[(i32, &str, &str)],
) -> Result<(), Box<dyn Error>> {
    for &(dom, expected, reason) in cases {
        assert_eq!(visible_name(tree, dom)?, expected, "{reason}");
    }
    Ok(())
}

fn ref_id(dom_id: i32) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: Some(dom_id),
        unrendered_text: None,
    }
}

fn ref_unrendered(text: &str) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: None,
        unrendered_text: Some(text.to_owned()),
    }
}

fn ref_id_with_unrendered(dom_id: i32, text: &str) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: Some(dom_id),
        unrendered_text: Some(text.to_owned()),
    }
}

fn select_opt(text: Option<&str>, value: Option<&str>, selected: bool) -> SelectOption {
    SelectOption {
        value: value.map(str::to_owned),
        text: text.map(str::to_owned),
        is_selected: Some(selected),
        is_disabled: None,
    }
}

fn attach_image_caption(apc: &mut AnnotatedPageContent, target_dom_id: i32, caption: &str) {
    let mut stack = Vec::new();
    if let Some(root) = apc.root_node.as_mut() {
        stack.push(root);
    }
    while let Some(node) = stack.pop() {
        if let Some(attrs) = node.content_attributes.as_mut()
            && attrs.common_ancestor_dom_node_id == Some(target_dom_id)
        {
            attrs.attribute_type = Some(i32::from(ContentAttributeType::Image));
            attrs.content_data = Some(ContentData::Image(ImageInfo {
                image_caption: Some(caption.to_owned()),
                security_origin: None,
                url: None,
            }));
        }
        for child in &mut node.children_nodes {
            stack.push(child);
        }
    }
}

// ============================================================================
// Step 1 — Name-prohibited roles
// ============================================================================

#[test]
fn step1_all_15_prohibited_roles_drop_all_name_sources() -> Result<(), Box<dyn Error>> {
    let prohibited: &[(&str, Role)] = &[
        ("caption", Role::Caption),
        ("code", Role::Code),
        ("definition", Role::Definition),
        ("deletion", Role::Deletion),
        ("emphasis", Role::Emphasis),
        ("generic", Role::Generic),
        ("insertion", Role::Insertion),
        ("mark", Role::Mark),
        ("paragraph", Role::Paragraph),
        ("presentation", Role::Presentation),
        ("strong", Role::Strong),
        ("subscript", Role::Subscript),
        ("superscript", Role::Superscript),
        ("term", Role::Term),
        ("time", Role::Time),
    ];

    let mut root = NodeBuilder::root().dom_id(1).child(
        NodeBuilder::element("span")
            .dom_id(2)
            .child(NodeBuilder::text("From LabelledBy")),
    );

    for (idx, &(role_attr, _)) in prohibited.iter().enumerate() {
        let dom = i32::try_from(idx + 10)?;
        // `<p>` keeps `Role::Presentation` even with global ARIA label attributes because its
        // implicit `paragraph` role is label-prohibited in conflict resolution.
        let tag = if role_attr == "presentation" {
            "p"
        } else {
            "div"
        };
        root = root.child(
            NodeBuilder::element(tag)
                .dom_id(dom)
                .role(role_attr)
                .labelledby(vec![ref_id(2)])
                .aria_label("From AriaLabel")
                .title("From Title")
                .child(NodeBuilder::text("From Content")),
        );
    }

    root = root.child(
        NodeBuilder::element("div")
            .dom_id(100)
            .role("presentation")
            .title("Div Presentation Title")
            .child(NodeBuilder::text("Div Presentation Text")),
    );

    let tree = build_tree(root)?;
    let cache = tree.name_cache();

    for (idx, &(role_attr, expected_role)) in prohibited.iter().enumerate() {
        let dom = i32::try_from(idx + 10)?;
        let node = node_by_dom(&tree, "doc-main", dom)?;
        assert_eq!(
            node.role(),
            Some(expected_role),
            "fixture for prohibited role {role_attr:?} must compute {expected_role:?}"
        );
        for visibility in [NameVisibility::VisibleOnly, NameVisibility::IncludeHidden] {
            let name = cache.name(node.id(), visibility);
            assert!(
                name.is_empty(),
                "prohibited role {role_attr:?} must drop labelledby, aria-label, content, and title under {visibility:?}"
            );
            assert!(
                !name.derived_from_content(),
                "prohibited role {role_attr:?} must never report derived_from_content"
            );
        }
    }

    assert_eq!(
        visible_name(&tree, 100)?,
        "",
        "unconflicted <div role=presentation> must drop content and title"
    );
    Ok(())
}

#[test]
fn step1_unroled_elements_and_explicit_none_are_not_prohibited() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(2)
                    .child(NodeBuilder::text("External Label")),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(10)
                    .aria_label("Unroled Div"),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(11)
                    .aria_label("Unroled Span"),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(12)
                    .role("generic")
                    .aria_label("Dropped Generic"),
            )
            .child(
                NodeBuilder::element("p")
                    .dom_id(13)
                    .role("none")
                    .aria_label("Kept By None"),
            )
            .child(
                NodeBuilder::element("p")
                    .dom_id(14)
                    .role("none")
                    .labelledby(vec![ref_id(2)]),
            ),
    )?;

    assert_eq!(
        node_by_dom(&tree, "doc-main", 10)?.role(),
        None,
        "plain <div> must have no ARIA role"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 11)?.role(),
        None,
        "plain <span> must have no ARIA role"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        Some(Role::Generic),
        "explicit role=generic must compute Role::Generic"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.role(),
        Some(Role::None),
        "<p role=none aria-label=...> must compute Role::None"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 14)?.role(),
        Some(Role::None),
        "<p role=none aria-labelledby=...> must compute Role::None"
    );

    assert_visible_names(
        &tree,
        &[
            (10, "Unroled Div", "unroled <div> keeps aria-label"),
            (11, "Unroled Span", "unroled <span> keeps aria-label"),
            (12, "", "<div role=generic> drops aria-label at Step 1"),
            (
                13,
                "Kept By None",
                "explicit role=none is not prohibited at Step 1 and keeps aria-label",
            ),
            (
                14,
                "External Label",
                "explicit role=none resolves aria-labelledby at Step 2",
            ),
        ],
    )
}

// ============================================================================
// Step 2 — aria-labelledby (order, self-ref, hidden chains, cycles, frames, privacy)
// ============================================================================

#[test]
fn step2_labelledby_precedence_order_dedup_and_derived_from_content() -> Result<(), Box<dyn Error>>
{
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(10)
                    .child(NodeBuilder::text("First")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(11)
                    .child(NodeBuilder::text("Second")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(12)
                    .child(NodeBuilder::text("Third")),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(13)
                    .label_for(20)
                    .child(NodeBuilder::text("Native Label")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .labelledby(vec![ref_id(10), ref_id(11), ref_id(10), ref_id(12)])
                    .aria_label("Ignored AriaLabel")
                    .title("Ignored Title")
                    .child(NodeBuilder::text("Ignored Content")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(30)
                    .labelledby(vec![ref_id(30), ref_id(12)])
                    .child(NodeBuilder::text("SelfPart")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(40)
                    .labelledby(vec![ref_id(41)])
                    .child(
                        NodeBuilder::element("span")
                            .dom_id(41)
                            .child(NodeBuilder::text("Inside Child")),
                    ),
            ),
    )?;

    let ext = tree.accessible_name(node_id(&tree, "doc-main", 20)?, NameVisibility::VisibleOnly);
    assert_eq!(
        ext.text(),
        "First Second Third",
        "labelledby must join refs in order, deduplicate repeated dom_node_id, and beat aria-label, native label, content, and title"
    );
    assert!(
        !ext.derived_from_content(),
        "external labelledby referents must keep derived_from_content false"
    );

    let self_ref =
        tree.accessible_name(node_id(&tree, "doc-main", 30)?, NameVisibility::VisibleOnly);
    assert_eq!(
        self_ref.text(),
        "SelfPart Third",
        "self-referencing labelledby (`lb-self lb-self-tail`) must evaluate self and join tail"
    );
    assert!(
        self_ref.derived_from_content(),
        "self-referencing labelledby must set derived_from_content true"
    );

    let desc_ref =
        tree.accessible_name(node_id(&tree, "doc-main", 40)?, NameVisibility::VisibleOnly);
    assert_eq!(
        desc_ref.text(),
        "Inside Child",
        "descendant-referencing labelledby must compute descendant text"
    );
    assert!(
        desc_ref.derived_from_content(),
        "labelledby pointing inside the target subtree must set derived_from_content true"
    );
    Ok(())
}

#[test]
fn step2_labelledby_hidden_referents_and_visibility_modes() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("div")
                    .dom_id(10)
                    .aria_hidden(true)
                    .child(NodeBuilder::text("Hidden "))
                    .child(
                        NodeBuilder::element("span")
                            .aria_hidden(true)
                            .child(NodeBuilder::text("Descendant")),
                    ),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(11)
                    .visibility_hidden(true)
                    .child(NodeBuilder::text("Css Invisible")),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(12)
                    .child(NodeBuilder::text("Visible "))
                    .child(
                        NodeBuilder::element("span")
                            .aria_hidden(true)
                            .child(NodeBuilder::text("SecretChild")),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .labelledby(vec![ref_id(10)]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .labelledby(vec![ref_id(11)]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(22)
                    .labelledby(vec![ref_id(12)]),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                20,
                "Hidden Descendant",
                "direct aria-hidden labelledby referent and its hidden descendants still contribute",
            ),
            (
                21,
                "Css Invisible",
                "direct visibility_hidden labelledby referent still contributes",
            ),
            (
                22,
                "Visible",
                "visible labelledby referent omits aria-hidden children under VisibleOnly",
            ),
        ],
    )?;
    assert_eq!(
        hidden_name(&tree, 22)?,
        "Visible SecretChild",
        "visible labelledby referent includes aria-hidden children under IncludeHidden"
    );
    Ok(())
}

#[test]
fn step2_labelledby_one_hop_aria_label_and_unrendered_fallbacks() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(10)
                    .child(NodeBuilder::text("Second Hop Target")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(11)
                    .labelledby(vec![ref_id(10)])
                    .child(NodeBuilder::text("Own Content")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(12)
                    .aria_label("Referent AriaLabel")
                    .child(NodeBuilder::text("Ignored Child")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .labelledby(vec![ref_id(11), ref_id(12)]),
            )
            .child(NodeBuilder::element("button").dom_id(21).labelledby(vec![
                ref_unrendered("Display None Text"),
                ref_id_with_unrendered(999, "Detached Fallback"),
            ]))
            .child(
                NodeBuilder::element("button")
                    .dom_id(22)
                    .labelledby(vec![ref_id(888)])
                    .child(NodeBuilder::text("Fallback Content")),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                20,
                "Own Content Referent AriaLabel",
                "labelledby must not follow a second labelledby hop and must honor referent aria-label",
            ),
            (
                21,
                "Display None Text Detached Fallback",
                "unrendered refs with unrendered_text must contribute text in order",
            ),
            (
                22,
                "Fallback Content",
                "unresolved dom_node_id without unrendered_text must fall through to content",
            ),
        ],
    )
}

#[test]
fn step2_labelledby_cycles_and_cross_frame_isolation() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-child").root(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(50)
                    .child(NodeBuilder::text("IFRAME_LEAK")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(51)
                    .child(NodeBuilder::text("IFRAME_SHADOW")),
            ),
    );

    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::iframe(iframe).dom_id(5))
            .child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .labelledby(vec![ref_id(11)])
                    .child(NodeBuilder::text("NodeA")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .labelledby(vec![ref_id(10)])
                    .child(NodeBuilder::text("NodeB")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(12)
                    .labelledby(vec![ref_id(12)]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .labelledby(vec![ref_id(50)])
                    .child(NodeBuilder::text("Main Fallback")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(51)
                    .child(NodeBuilder::text("Main Own Label")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .labelledby(vec![ref_id(51)]),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "NodeB",
                "A -> B -> A labelledby cycle must terminate at B's text without looping",
            ),
            (
                11,
                "NodeA",
                "B -> A -> B labelledby cycle must terminate at A's text without looping",
            ),
            (
                12,
                "",
                "empty A -> A self-cycle must terminate cleanly with empty name",
            ),
            (
                20,
                "Main Fallback",
                "parent labelledby must never leak a matching dom_id from a child iframe",
            ),
            (
                21,
                "Main Own Label",
                "parent labelledby stays scoped to doc-main when both docs share dom_id",
            ),
        ],
    )
}

#[test]
fn step2_labelledby_redacted_boxes_emit_bullets_and_never_leak() -> Result<(), Box<dyn Error>> {
    let redacted_variants: &[(RedactionDecision, u32)] = &[
        (RedactionDecision::RedactedHasBeenPassword, 5),
        (RedactionDecision::RedactedIsOtp, 6),
        (RedactionDecision::RedactedIsSensitivePaymentField, 4),
        (RedactionDecision::RedactedCustomPasswordCss, 3),
        (RedactionDecision::RedactedCustomPasswordJs, 2),
    ];

    let mut root = NodeBuilder::root().dom_id(1);
    for (idx, &(decision, bullet_len)) in redacted_variants.iter().enumerate() {
        let box_dom = i32::try_from(10 + idx)?;
        let target_dom = i32::try_from(30 + idx)?;
        root = root
            .child(
                NodeBuilder::form_control(FormControlType::InputPassword)
                    .tag("input")
                    .dom_id(box_dom)
                    .field_value("POISON_SECRET_VALUE")
                    .redaction(decision)
                    .redacted_value_length(bullet_len)
                    .child(NodeBuilder::text("POISON_CHILD_TEXT")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(target_dom)
                    .labelledby(vec![ref_id(box_dom)]),
            );
    }

    // Redacted non-textbox element referenced via labelledby must not walk children in Step 2f.
    root = root
        .child(
            NodeBuilder::element("div")
                .dom_id(50)
                .redaction(RedactionDecision::RedactedCustomPasswordCss)
                .redacted_value_length(4)
                .child(NodeBuilder::text("POISON_DIV_CHILD")),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(51)
                .labelledby(vec![ref_id(50)]),
        );

    let tree = build_tree(root)?;

    for (idx, &(decision, bullet_len)) in redacted_variants.iter().enumerate() {
        let target_dom = i32::try_from(30 + idx)?;
        let expected = "•".repeat(usize::try_from(bullet_len)?);
        assert_eq!(
            visible_name(&tree, target_dom)?,
            expected,
            "redacted box ({decision:?}) in labelledby must emit {bullet_len} bullets and never leak field_value or children"
        );
    }
    assert_eq!(
        visible_name(&tree, 51)?,
        "",
        "redacted non-textbox element must never leak child text in Step 2f"
    );
    Ok(())
}

#[test]
fn step2_labelledby_zero_len_redacted_and_unredacted_empty_boxes() -> Result<(), Box<dyn Error>> {
    let empty_variants: &[RedactionDecision] = &[
        RedactionDecision::UnredactedEmptyPassword,
        RedactionDecision::UnredactedEmptyPaymentField,
        RedactionDecision::UnredactedEmptyCustomPassword,
        RedactionDecision::UnredactedEmptyOtpField,
    ];

    let mut root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::form_control(FormControlType::InputPassword)
                .tag("input")
                .dom_id(10)
                .field_value("POISON_NO_LEN")
                .redaction(RedactionDecision::RedactedHasBeenPassword),
        )
        .child(
            NodeBuilder::form_control(FormControlType::InputPassword)
                .tag("input")
                .dom_id(11)
                .field_value("POISON_ZERO_LEN")
                .redaction(RedactionDecision::RedactedIsOtp)
                .redacted_value_length(0),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(20)
                .labelledby(vec![ref_id(10)]),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(21)
                .labelledby(vec![ref_id(11)]),
        );

    for (idx, &decision) in empty_variants.iter().enumerate() {
        let box_dom = i32::try_from(40 + idx)?;
        let target_dom = i32::try_from(50 + idx)?;
        root = root
            .child(
                NodeBuilder::form_control(FormControlType::InputPassword)
                    .tag("input")
                    .dom_id(box_dom)
                    .field_value("POISON_UNREDACTED_EMPTY")
                    .redaction(decision),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(target_dom)
                    .labelledby(vec![ref_id(box_dom)]),
            );
    }

    let tree = build_tree(root)?;

    assert_visible_names(
        &tree,
        &[
            (
                20,
                "",
                "redacted box with redacted_value_length=None emits empty",
            ),
            (
                21,
                "",
                "redacted box with redacted_value_length=Some(0) emits empty",
            ),
        ],
    )?;

    for (idx, &decision) in empty_variants.iter().enumerate() {
        let target_dom = i32::try_from(50 + idx)?;
        assert_eq!(
            visible_name(&tree, target_dom)?,
            "",
            "{decision:?} in labelledby must emit empty string without reading field_value"
        );
    }
    Ok(())
}

#[test]
fn step2_labelledby_unrendered_private_boxes_alone_and_with_sibling() -> Result<(), Box<dyn Error>>
{
    let empty_none_ref = BrowserOsRef {
        dom_node_id: None,
        unrendered_text: None,
    };
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(2)
                    .child(NodeBuilder::text("Prefix")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(30)
                    .labelledby(vec![ref_unrendered("")]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(31)
                    .labelledby(vec![empty_none_ref.clone()]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(32)
                    .labelledby(vec![ref_id(2), ref_unrendered("")]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(33)
                    .labelledby(vec![ref_id(2), empty_none_ref]),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (30, "", "unrendered private box with Some(\"\") emits empty"),
            (31, "", "unrendered private box with None emits empty"),
            (
                32,
                "Prefix",
                "sibling label + unrendered private Some(\"\") emits only sibling",
            ),
            (
                33,
                "Prefix",
                "sibling label + unrendered private None emits only sibling",
            ),
        ],
    )
}

fn build_private_failure_fields_tree() -> Result<SnapshotTree, Box<dyn Error>> {
    build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(10)
                    .child(NodeBuilder::text("Public label")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::TextArea)
                    .tag("textarea")
                    .role("searchbox")
                    .dom_id(11)
                    .aria_label("Private textarea")
                    .field_value("TypedCanary-2")
                    .redaction(RedactionDecision::RedactedHasBeenPassword)
                    .redacted_value_length(13)
                    .child(NodeBuilder::text("TextCanary-3")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(12)
                    .labelledby(vec![ref_id(10), ref_id(11)])
                    .child(NodeBuilder::text("Name target")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(13)
                    .describedby(vec![ref_id(12)])
                    .child(NodeBuilder::text("Description target")),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(14)
                    .child(NodeBuilder::text("Wrong: "))
                    .child(
                        NodeBuilder::element("button")
                            .dom_id(15)
                            .labelledby(vec![ref_id(11)])
                            .child(NodeBuilder::text("Error source")),
                    ),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(16)
                    .aria_label("Invalid private reference")
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(14)]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(17)
                    .describedby(vec![ref_id(11)])
                    .child(NodeBuilder::text("Direct description control")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputPassword)
                    .tag("input")
                    .dom_id(18)
                    .placeholder("PIN")
                    .field_value("SECRET")
                    .redaction(RedactionDecision::RedactedHasBeenPassword)
                    .redacted_value_length(6),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(19)
                    .describedby(vec![ref_id(18)])
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(18)])
                    .child(NodeBuilder::text("Direct placeholder control")),
            ),
    )
}

#[test]
fn private_failure_fields_direct_vs_indirect_descriptions_and_errors() -> Result<(), Box<dyn Error>>
{
    let bullets = "•".repeat(13);
    let tree = build_private_failure_fields_tree()?;
    let desc = |dom: i32| -> Result<String, Box<dyn Error>> {
        Ok(tree
            .accessible_description(
                node_id(&tree, "doc-main", dom)?,
                NameVisibility::VisibleOnly,
            )
            .into_text())
    };
    let err = |dom: i32| -> Result<String, Box<dyn Error>> {
        Ok(tree.accessible_error_message(node_id(&tree, "doc-main", dom)?))
    };

    assert_eq!(visible_name(&tree, 12)?, format!("Public label {bullets}"));
    assert_eq!(desc(13)?, format!("Public label {bullets}"));
    assert_eq!(err(16)?, format!("Wrong: {bullets}"));
    assert_eq!(
        desc(17)?,
        "Private textarea",
        "#description-direct follows Playwright roleUtils.ts:745 (aria-label, never secret)"
    );
    assert_eq!(desc(19)?, "PIN");
    assert_eq!(err(19)?, "PIN");
    Ok(())
}

// ============================================================================
// Step 3 — Embedded controls
// ============================================================================

#[test]
fn step3_embedded_textboxes_in_label_and_descendant() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("label")
                    .dom_id(10)
                    .label_for(11)
                    .child(NodeBuilder::text("Notify "))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .dom_id(12)
                            .field_value("5")
                            .placeholder("Count"),
                    )
                    .child(NodeBuilder::text(" days")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(11),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(30)
                    .child(NodeBuilder::text("Run "))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .dom_id(31)
                            .field_value("build"),
                    )
                    .child(NodeBuilder::text(" pin:"))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputPassword)
                            .tag("input")
                            .dom_id(32)
                            .field_value("9999")
                            .redaction(RedactionDecision::RedactedHasBeenPassword)
                            .redacted_value_length(4),
                    ),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                11,
                "Notify 5 days",
                "textbox embedded in <label> emits field_value",
            ),
            (
                30,
                "Run build pin:••••",
                "controls embedded as target descendants emit value or redacted bullets",
            ),
            (
                12,
                "Count",
                "top-level text input uses placeholder, not field_value",
            ),
        ],
    )
}

#[test]
fn step3_embedded_textarea_searchbox_and_role_textbox_vs_top_level() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::form_control(FormControlType::TextArea)
                    .tag("textarea")
                    .dom_id(20)
                    .field_value("memo")
                    .placeholder("Write memo"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputSearch)
                    .tag("input")
                    .dom_id(21)
                    .field_value("rust")
                    .aria_label("Site search"),
            )
            .child(
                NodeBuilder::element("div")
                    .role("textbox")
                    .dom_id(22)
                    .field_value("FALLBACK_IGNORED")
                    .aria_label("Editable box")
                    .child(NodeBuilder::text("dom-text")),
            )
            .child(
                NodeBuilder::element("div")
                    .role("textbox")
                    .dom_id(23)
                    .field_value("field-fallback")
                    .aria_label("Fallback box"),
            )
            .child(NodeBuilder::element("button").dom_id(24).labelledby(vec![
                ref_id(20),
                ref_id(21),
                ref_id(22),
                ref_id(23),
            ])),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                24,
                "memo rust dom-text field-fallback",
                "textarea, searchbox, and role=textbox (DOM text preferred over field_value) embedded in labelledby emit values",
            ),
            (
                20,
                "Write memo",
                "top-level textarea uses placeholder, not field_value",
            ),
            (
                21,
                "Site search",
                "top-level searchbox uses aria-label, not field_value",
            ),
            (
                22,
                "Editable box",
                "top-level role=textbox uses aria-label, not embedded text",
            ),
        ],
    )
}

#[test]
fn step3_embedded_collapsed_and_listbox_select_controls() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .child(NodeBuilder::text("Tags: "))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectMultiple)
                            .tag("select")
                            .select_option(select_opt(Some("Red"), Some("r"), true))
                            .select_option(select_opt(Some("Green"), Some("g"), false))
                            .select_option(select_opt(Some("Blue"), Some("b"), true)),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .child(NodeBuilder::text("Default: "))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectOne)
                            .tag("select")
                            .select_option(select_opt(Some("FirstFallback"), Some("1"), false))
                            .select_option(select_opt(Some("Second"), Some("2"), false)),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(12)
                    .child(NodeBuilder::text("Selected: "))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectMultiple)
                            .tag("select")
                            .list_box(true)
                            .child(
                                NodeBuilder::element("option")
                                    .selected(false)
                                    .child(NodeBuilder::text("Alpha")),
                            )
                            .child(
                                NodeBuilder::element("option")
                                    .selected(true)
                                    .child(NodeBuilder::text("Beta")),
                            )
                            .child(
                                NodeBuilder::element("option")
                                    .selected(true)
                                    .child(NodeBuilder::text("Gamma")),
                            ),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(13)
                    .child(NodeBuilder::text("First: "))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectOne)
                            .tag("select")
                            .list_box(true)
                            .child(
                                NodeBuilder::element("option")
                                    .selected(false)
                                    .child(NodeBuilder::text("FirstOpt")),
                            )
                            .child(
                                NodeBuilder::element("option")
                                    .selected(false)
                                    .child(NodeBuilder::text("SecondOpt")),
                            ),
                    ),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Tags: Red Blue",
                "collapsed <select> joins selected select_options",
            ),
            (
                11,
                "Default: FirstFallback",
                "collapsed <select> with no selected option falls back to first option text",
            ),
            (
                12,
                "Selected: Beta Gamma",
                "list-box <select> joins selected <option> children with spaces",
            ),
            (
                13,
                "First: FirstOpt",
                "list-box <select> with no selected <option> falls back to first <option>",
            ),
        ],
    )
}

#[test]
fn step3_collapsed_select_never_invents_name_from_submission_value() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("button")
                    .dom_id(14)
                    .child(NodeBuilder::text("EmptyUnselected:"))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectOne)
                            .tag("select")
                            .select_option(select_opt(None, Some("id-123"), false))
                            .select_option(select_opt(Some("Second"), Some("2"), false)),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(15)
                    .child(NodeBuilder::text("EmptySelected:"))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectMultiple)
                            .tag("select")
                            .select_option(select_opt(None, Some("secret-id"), true)),
                    ),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                14,
                "EmptyUnselected:",
                "collapsed <select> with no text on first option does not invent name from submission value",
            ),
            (
                15,
                "EmptySelected:",
                "collapsed <select> with no text on selected option does not invent name from submission value",
            ),
        ],
    )
}

#[test]
fn step3_embedded_aria_combobox_listbox_range_and_menu() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .child(NodeBuilder::text("Combo: "))
                    .child(
                        NodeBuilder::element("div").role("combobox").child(
                            NodeBuilder::element("div")
                                .role("listbox")
                                .child(
                                    NodeBuilder::element("div")
                                        .role("option")
                                        .selected(false)
                                        .child(NodeBuilder::text("Skip")),
                                )
                                .child(
                                    NodeBuilder::element("div")
                                        .role("option")
                                        .selected(true)
                                        .child(NodeBuilder::text("Chosen")),
                                ),
                        ),
                    )
                    .child(NodeBuilder::text(" / "))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .role("combobox")
                            .field_value("TypedCombo"),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .child(
                        NodeBuilder::form_control(FormControlType::InputRange)
                            .tag("input")
                            .field_value("10"),
                    )
                    .child(NodeBuilder::text(","))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputNumber)
                            .tag("input")
                            .field_value("20"),
                    )
                    .child(NodeBuilder::text(","))
                    .child(NodeBuilder::element("progress").field_value("30"))
                    .child(NodeBuilder::text(","))
                    .child(
                        NodeBuilder::element("div")
                            .role("scrollbar")
                            .field_value("40"),
                    )
                    .child(NodeBuilder::text(","))
                    .child(NodeBuilder::element("meter").field_value("50"))
                    .child(
                        NodeBuilder::element("div")
                            .role("menu")
                            .child(NodeBuilder::text("IgnoredMenuItem")),
                    ),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Combo: Chosen / TypedCombo",
                "ARIA combobox reads selected listbox option and <input role=combobox> falls back to field_value",
            ),
            (
                11,
                "10,20,30,40,50",
                "slider, spinbutton, progressbar, scrollbar, and meter emit field_value while menu emits empty",
            ),
        ],
    )
}

// ============================================================================
// Step 4 — aria-label
// ============================================================================

#[test]
fn step4_aria_label_precedence_and_blank_fallthrough() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("label")
                    .dom_id(10)
                    .label_for(20)
                    .child(NodeBuilder::text("Native Label")),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(11)
                    .label_for(21)
                    .child(NodeBuilder::text("Fallback Label")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(20)
                    .aria_label("Explicit AriaLabel")
                    .title("Ignored Title")
                    .placeholder("Ignored Placeholder"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(21)
                    .aria_label("   \t\r\n  "),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(22)
                    .aria_label("\u{00A0}\u{FEFF}")
                    .child(NodeBuilder::text("Button Content")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(23)
                    .aria_label("\u{0085}")
                    .child(NodeBuilder::text("Ignored Content")),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                20,
                "Explicit AriaLabel",
                "non-blank aria-label beats native <label>, title, and placeholder",
            ),
            (
                21,
                "Fallback Label",
                "ASCII whitespace-only aria-label falls through to native <label>",
            ),
            (
                22,
                "Button Content",
                "NBSP + BOM aria-label trims empty in JS whitespace and falls through to content",
            ),
            (
                23,
                "\u{0085}",
                "U+0085 NEL is not JS whitespace so aria-label keeps it instead of falling through",
            ),
        ],
    )
}

// ============================================================================
// Step 5 — Native host-language naming (every tag)
// ============================================================================

#[test]
fn step5_labels_for_single_multiple_wrapping_and_hidden_root() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("label")
                    .dom_id(10)
                    .label_for(11)
                    .child(NodeBuilder::text("Email")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(11),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(20)
                    .label_for(23)
                    .child(NodeBuilder::text("First")),
            )
            .child(NodeBuilder::element("label").dom_id(21).label_for(23))
            .child(
                NodeBuilder::element("label")
                    .dom_id(22)
                    .label_for(23)
                    .child(NodeBuilder::text("Last")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(23),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(30)
                    .label_for(31)
                    .child(NodeBuilder::text("Accept "))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .dom_id(31)
                            .field_value("SHOULD_NOT_APPEAR"),
                    )
                    .child(NodeBuilder::text(" terms")),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(40)
                    .label_for(41)
                    .aria_hidden(true)
                    .child(NodeBuilder::text("Hidden Label Text")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(41),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (11, "Email", "single <label for> names its control"),
            (
                23,
                "First Last",
                "multiple <label for> join in document order with empty labels filtered out",
            ),
            (
                31,
                "Accept terms",
                "wrapping <label> skips the target control itself",
            ),
            (
                41,
                "Hidden Label Text",
                "aria-hidden <label> root still contributes text to its labelled control",
            ),
        ],
    )
}

#[test]
fn step5_input_button_submit_reset_and_file() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::form_control(FormControlType::InputButton)
                    .tag("input")
                    .dom_id(10)
                    .field_value("Press Me")
                    .title("Btn Title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputButton)
                    .tag("input")
                    .dom_id(11)
                    .field_value("   ")
                    .title("Btn Title Fallback"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputSubmit)
                    .tag("input")
                    .dom_id(12)
                    .field_value("Send"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputSubmit)
                    .tag("input")
                    .dom_id(13)
                    .field_value("  ")
                    .title("Ignored Submit Title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputReset)
                    .tag("input")
                    .dom_id(14)
                    .field_value(""),
            )
            .child(
                NodeBuilder::element("label")
                    .label_for(20)
                    .child(NodeBuilder::text("Avatar")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(20)
                    .title("Ignored File Title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(21)
                    .title("Ignored File Title"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(22)
                    .labelledby(vec![ref_id(20)]),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Press Me",
                "input[type=button] uses non-blank field_value",
            ),
            (
                11,
                "Btn Title Fallback",
                "blank input[type=button] falls back to title",
            ),
            (12, "Send", "input[type=submit] uses non-blank field_value"),
            (
                13,
                "Submit",
                "blank input[type=submit] falls back to Submit",
            ),
            (14, "Reset", "blank input[type=reset] falls back to Reset"),
            (
                20,
                "Avatar",
                "input[type=file] uses associated <label> at top level",
            ),
            (
                21,
                "Choose File",
                "unlabelled input[type=file] uses Choose File and ignores title",
            ),
            (
                22,
                "Choose File",
                "input[type=file] inside aria-labelledby skips its <label> and yields Choose File",
            ),
        ],
    )
}

#[test]
fn step5_input_image_native_precedence() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("label")
                    .label_for(10)
                    .child(NodeBuilder::text("Img Label")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(10)
                    .alt("Img Alt")
                    .title("Img Title"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .labelledby(vec![ref_id(10)]),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(12)
                    .alt("Go")
                    .title("Ignored"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(13)
                    .alt("   ")
                    .title("Image Title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(14),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Img Label",
                "input[type=image] prefers <label> at top level",
            ),
            (
                11,
                "Img Alt",
                "input[type=image] inside labelledby skips its <label> and falls back to alt",
            ),
            (
                12,
                "Go",
                "input[type=image] prefers non-blank alt over title",
            ),
            (
                13,
                "Image Title",
                "input[type=image] with blank alt falls back to title",
            ),
            (
                14,
                "Submit",
                "input[type=image] without label/alt/title falls back to Submit",
            ),
        ],
    )
}

#[test]
fn step5_button_and_output_native_precedence() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("label")
                    .label_for(20)
                    .child(NodeBuilder::text("Labelled Button")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .child(NodeBuilder::text("Inner Text")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .child(NodeBuilder::text("Own Content")),
            )
            .child(
                NodeBuilder::element("label")
                    .label_for(30)
                    .child(NodeBuilder::text("Total")),
            )
            .child(
                NodeBuilder::element("output")
                    .dom_id(30)
                    .title("Ignored Output Title")
                    .child(NodeBuilder::text("42")),
            )
            .child(
                NodeBuilder::element("output")
                    .dom_id(31)
                    .title("Result Output")
                    .child(NodeBuilder::text("99")),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                20,
                "Labelled Button",
                "<button> prefers associated <label> over content",
            ),
            (
                21,
                "Own Content",
                "<button> without <label> uses Step 2f content",
            ),
            (
                30,
                "Total",
                "<output> prefers associated <label> over title",
            ),
            (
                31,
                "Result Output",
                "<output> without <label> falls back to title",
            ),
        ],
    )?;
    assert!(
        tree.accessible_name(node_id(&tree, "doc-main", 21)?, NameVisibility::VisibleOnly)
            .derived_from_content(),
        "unlabelled <button> named from inner text must mark derived_from_content"
    );
    Ok(())
}

#[test]
fn step5_text_like_inputs_and_textarea_title_placeholder_precedence() -> Result<(), Box<dyn Error>>
{
    let text_types: &[(FormControlType, &str)] = &[
        (FormControlType::InputText, "text"),
        (FormControlType::InputPassword, "password"),
        (FormControlType::InputEmail, "email"),
        (FormControlType::InputSearch, "search"),
        (FormControlType::InputTelephone, "tel"),
        (FormControlType::InputUrl, "url"),
        (FormControlType::InputNumber, "number"),
    ];

    let mut root = NodeBuilder::root().dom_id(1);
    for (idx, &(control_type, _)) in text_types.iter().enumerate() {
        let base = i32::try_from(10 + idx * 3)?;
        root = root
            .child(
                NodeBuilder::form_control(control_type)
                    .tag("input")
                    .dom_id(base)
                    .title("Title Wins")
                    .placeholder("Placeholder Loses"),
            )
            .child(
                NodeBuilder::form_control(control_type)
                    .tag("input")
                    .dom_id(base + 1)
                    .placeholder("Placeholder Fallback"),
            );
    }

    root = root
        .child(
            NodeBuilder::form_control(FormControlType::TextArea)
                .tag("textarea")
                .dom_id(50)
                .title("Area Title")
                .placeholder("Area Placeholder"),
        )
        .child(
            NodeBuilder::form_control(FormControlType::TextArea)
                .tag("textarea")
                .dom_id(51)
                .placeholder("Area Placeholder"),
        );

    let tree = build_tree(root)?;

    for (idx, &(_, kind)) in text_types.iter().enumerate() {
        let base = i32::try_from(10 + idx * 3)?;
        assert_eq!(
            visible_name(&tree, base)?,
            "Title Wins",
            "input[type={kind}] must prefer title over placeholder"
        );
        assert_eq!(
            visible_name(&tree, base + 1)?,
            "Placeholder Fallback",
            "input[type={kind}] without labels or title must fall back to placeholder"
        );
    }
    assert_visible_names(
        &tree,
        &[
            (
                50,
                "Area Title",
                "<textarea> prefers title over placeholder",
            ),
            (
                51,
                "Area Placeholder",
                "<textarea> falls back to placeholder",
            ),
        ],
    )
}

#[test]
fn step5_non_placeholder_form_controls_never_use_placeholder() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::form_control(FormControlType::SelectOne)
                    .tag("select")
                    .dom_id(10)
                    .placeholder("Never Select"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::SelectOne)
                    .tag("select")
                    .dom_id(11)
                    .title("Select Title")
                    .placeholder("Never Select"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(12)
                    .placeholder("Never Checkbox"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputRadio)
                    .tag("input")
                    .dom_id(13)
                    .placeholder("Never Radio"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputDate)
                    .tag("input")
                    .dom_id(14)
                    .placeholder("Never Date"),
            )
            .child(
                NodeBuilder::element("meter")
                    .dom_id(15)
                    .placeholder("Never Meter"),
            )
            .child(
                NodeBuilder::element("progress")
                    .dom_id(16)
                    .placeholder("Never Progress"),
            )
            .child(
                NodeBuilder::element("progress")
                    .dom_id(17)
                    .title("Upload Progress")
                    .placeholder("Never Progress"),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (10, "", "<select> must never fall back to placeholder"),
            (11, "Select Title", "<select> falls back to title"),
            (
                12,
                "",
                "input[type=checkbox] must never fall back to placeholder",
            ),
            (
                13,
                "",
                "input[type=radio] must never fall back to placeholder",
            ),
            (
                14,
                "",
                "input[type=date] must never fall back to placeholder",
            ),
            (15, "", "<meter> must never fall back to placeholder"),
            (16, "", "<progress> must never fall back to placeholder"),
            (17, "Upload Progress", "<progress> falls back to title"),
        ],
    )
}

#[test]
fn step5_img_area_fieldset_and_figure_native_naming() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("img")
                    .dom_id(10)
                    .alt("Logo")
                    .title("Img Title"),
            )
            .child(
                NodeBuilder::element("img")
                    .dom_id(11)
                    .alt("  ")
                    .title("Img Fallback"),
            )
            .child(
                NodeBuilder::element("area")
                    .dom_id(12)
                    .href("/map")
                    .alt("North Wing")
                    .title("Area Title"),
            )
            .child(
                NodeBuilder::element("area")
                    .dom_id(13)
                    .href("/map")
                    .title("Area Fallback"),
            )
            .child(
                NodeBuilder::element("fieldset")
                    .dom_id(20)
                    .title("Ignored")
                    .child(NodeBuilder::element("legend").child(NodeBuilder::text("Shipping")))
                    .child(NodeBuilder::element("legend").child(NodeBuilder::text("Second"))),
            )
            .child(
                NodeBuilder::element("fieldset")
                    .dom_id(21)
                    .title("Blocked Title")
                    .child(NodeBuilder::element("legend")),
            )
            .child(
                NodeBuilder::element("fieldset")
                    .dom_id(22)
                    .title("Fieldset Title")
                    .child(NodeBuilder::element("div").child(
                        NodeBuilder::element("legend").child(NodeBuilder::text("Too Deep")),
                    )),
            )
            .child(
                NodeBuilder::element("figure")
                    .dom_id(30)
                    .title("Ignored")
                    .child(NodeBuilder::element("figcaption").child(NodeBuilder::text("Chart A"))),
            )
            .child(
                NodeBuilder::element("figure")
                    .dom_id(31)
                    .title("Figure Fallback"),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (10, "Logo", "<img> prefers non-blank alt over title"),
            (
                11,
                "Img Fallback",
                "<img> with blank alt falls back to title",
            ),
            (12, "North Wing", "<area> prefers non-blank alt over title"),
            (
                13,
                "Area Fallback",
                "<area> without alt falls back to title",
            ),
            (20, "Shipping", "<fieldset> uses its first direct <legend>"),
            (
                21,
                "",
                "<fieldset> with an empty direct <legend> does NOT fall back to title",
            ),
            (
                22,
                "Fieldset Title",
                "<fieldset> ignores grandchild <legend> and falls back to title",
            ),
            (30, "Chart A", "<figure> uses its first direct <figcaption>"),
            (
                31,
                "Figure Fallback",
                "<figure> without <figcaption> falls back to title",
            ),
        ],
    )
}

#[test]
fn step5_table_svg_and_presentational_step2e_skip() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("table")
                    .dom_id(10)
                    .title("Ignored Table Title")
                    .child(NodeBuilder::element("caption").child(NodeBuilder::text("Q4 Results"))),
            )
            .child(
                NodeBuilder::element("table")
                    .dom_id(11)
                    .title("Table Title")
                    .child(
                        NodeBuilder::element("tr")
                            .child(NodeBuilder::element("td").child(NodeBuilder::text("CellVal"))),
                    ),
            )
            .child(
                NodeBuilder::element("button").dom_id(12).child(
                    NodeBuilder::element("table").child(
                        NodeBuilder::element("tr")
                            .child(NodeBuilder::element("td").child(NodeBuilder::text("Left")))
                            .child(
                                NodeBuilder::element("td")
                                    .break_before(true)
                                    .child(NodeBuilder::text("Right")),
                            ),
                    ),
                ),
            )
            .child(
                NodeBuilder::element("svg").dom_id(20).child(
                    NodeBuilder::element("title")
                        .aria_hidden(true)
                        .child(NodeBuilder::text("Bell Icon")),
                ),
            )
            .child(
                NodeBuilder::element("img")
                    .dom_id(30)
                    .role("none")
                    .alt("Skipped Alt")
                    .title("Skipped Title"),
            )
            .child(
                NodeBuilder::element("fieldset")
                    .dom_id(31)
                    .role("none")
                    .child(NodeBuilder::element("legend").child(NodeBuilder::text("Skipped"))),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (10, "Q4 Results", "<table> uses its first direct <caption>"),
            (
                11,
                "Table Title",
                "top-level <table> without <caption> falls through to Step 2i title",
            ),
            (
                12,
                "Left Right",
                "<table> descendant of <button> collects cell text in Step 2f",
            ),
            (
                20,
                "Bell Icon",
                "<svg> uses its first direct <title> child even when hidden",
            ),
            (
                30,
                "",
                "<img role=none> skips Step 2e alt and Step 2i title",
            ),
            (31, "", "<fieldset role=none> skips Step 2e direct <legend>"),
        ],
    )
}

#[test]
fn step5_svg_descendant_title_and_foreign_object_boundary() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::element("svg")
                .dom_id(20)
                .child(
                    NodeBuilder::element("g")
                        .dom_id(21)
                        .role("img")
                        .child(
                            NodeBuilder::element("title")
                                .aria_hidden(true)
                                .child(NodeBuilder::text("Revenue")),
                        )
                        .child(NodeBuilder::element("rect")),
                )
                .child(
                    NodeBuilder::element("a").dom_id(22).href("#chart").child(
                        NodeBuilder::element("title")
                            .aria_hidden(true)
                            .child(NodeBuilder::text("Chart link")),
                    ),
                )
                .child(
                    NodeBuilder::element("foreignobject")
                        .child(NodeBuilder::element("div").dom_id(23).role("img").child(
                            NodeBuilder::element("title").child(NodeBuilder::text("Not SVG title")),
                        ))
                        .child(
                            NodeBuilder::element("svg").child(
                                NodeBuilder::element("g").dom_id(24).role("img").child(
                                    NodeBuilder::element("title")
                                        .aria_hidden(true)
                                        .child(NodeBuilder::text("Inner SVG")),
                                ),
                            ),
                        ),
                ),
        ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                21,
                "Revenue",
                "SVG descendant <g role=img> uses its direct SVG <title> child",
            ),
            (
                22,
                "Chart link",
                "SVG descendant <a href> uses its direct SVG <title> child",
            ),
            (
                23,
                "",
                "HTML element inside <foreignObject> does not treat HTML <title> as an SVG title",
            ),
            (
                24,
                "Inner SVG",
                "nested <svg> inside <foreignObject> re-enters the SVG namespace for <g><title>",
            ),
        ],
    )
}

// ============================================================================
// Step 6 — Name from content
// ============================================================================

#[test]
fn step6_name_from_content_role_allowlists() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("div")
                    .role("tab")
                    .dom_id(10)
                    .child(NodeBuilder::text("Overview")),
            )
            .child(
                NodeBuilder::element("div")
                    .role("list")
                    .dom_id(11)
                    .child(NodeBuilder::text("TopListText")),
            )
            .child(
                NodeBuilder::element("button").dom_id(12).child(
                    NodeBuilder::element("div")
                        .role("list")
                        .child(NodeBuilder::text("NestedListText")),
                ),
            )
            .child(
                NodeBuilder::element("div")
                    .role("group")
                    .dom_id(13)
                    .child(NodeBuilder::text("GroupText")),
            )
            .child(
                NodeBuilder::element("div")
                    .role("dialog")
                    .dom_id(14)
                    .child(NodeBuilder::text("DialogText")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(15)
                    .child(NodeBuilder::text("Keep"))
                    .child(
                        NodeBuilder::element("div")
                            .role("group")
                            .child(NodeBuilder::text("SkipGroup")),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .role("dialog")
                            .child(NodeBuilder::text("SkipDialog")),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .role("generic")
                            .child(NodeBuilder::text("SkipGeneric")),
                    ),
            ),
    )?;

    let tab = tree.accessible_name(node_id(&tree, "doc-main", 10)?, NameVisibility::VisibleOnly);
    assert_eq!(
        tab.text(),
        "Overview",
        "always-allowed role=tab uses content"
    );
    assert!(
        tab.derived_from_content(),
        "top-level name-from-content sets derived_from_content true"
    );

    assert_visible_names(
        &tree,
        &[
            (
                11,
                "",
                "descendant-only role=list skips content at top level",
            ),
            (
                12,
                "NestedListText",
                "descendant-only role=list contributes inside a button",
            ),
            (
                13,
                "",
                "non-content role=group ignores content at top level",
            ),
            (
                14,
                "",
                "non-content role=dialog ignores content at top level",
            ),
            (
                15,
                "Keep",
                "non-content roles (group, dialog, explicit generic) do not contribute inside a button",
            ),
        ],
    )
}

#[test]
fn step6_summary_name_from_content_and_presentational_override() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::element("details")
                .child(
                    NodeBuilder::element("summary")
                        .dom_id(20)
                        .child(NodeBuilder::text("More details")),
                )
                .child(
                    NodeBuilder::element("summary")
                        .role("none")
                        .dom_id(21)
                        .child(NodeBuilder::text("Presentational summary")),
                ),
        ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (20, "More details", "<summary> computes name from content"),
            (21, "", "presentational <summary role=none> skips content"),
        ],
    )
}

#[test]
fn step6_breaks_inline_joining_pseudos_and_nested_img() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .child(NodeBuilder::element("span").child(NodeBuilder::text("foo")))
                    .child(NodeBuilder::element("span").child(NodeBuilder::text("bar"))),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .child(NodeBuilder::element("div").child(NodeBuilder::text("foo")))
                    .child(
                        NodeBuilder::element("div")
                            .break_before(true)
                            .child(NodeBuilder::text("bar")),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(12)
                    .child(NodeBuilder::text("foo"))
                    .child(NodeBuilder::text("bar").break_before(true)),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(13)
                    .child(NodeBuilder::text("line1"))
                    .child(NodeBuilder::element("br").break_before(true))
                    .child(NodeBuilder::text("line2")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(14)
                    .child(NodeBuilder::text("[").without_dom_id())
                    .child(NodeBuilder::text("Core"))
                    .child(NodeBuilder::text("]").without_dom_id())
                    .child(
                        NodeBuilder::text("badge")
                            .without_dom_id()
                            .break_before(true),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(15)
                    .child(NodeBuilder::element("span").child(NodeBuilder::text("left")))
                    .child(
                        NodeBuilder::element("div")
                            .aria_hidden(true)
                            .break_before(true)
                            .child(NodeBuilder::text("hidden")),
                    )
                    .child(NodeBuilder::element("span").child(NodeBuilder::text("right"))),
            )
            .child(
                NodeBuilder::element("a")
                    .href("/cart")
                    .dom_id(16)
                    .child(NodeBuilder::element("img").alt("Cart"))
                    .child(NodeBuilder::text(" (3)")),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (10, "foobar", "inline <span> children join with no space"),
            (11, "foo bar", "break_before on block element inserts space"),
            (
                12,
                "foo bar",
                "break_before on dropped-wrapper text node inserts space",
            ),
            (13, "line1 line2", "<br> with break_before separates text"),
            (
                14,
                "[Core] badge",
                "inline pseudos join directly while block ::after separates with space",
            ),
            (
                15,
                "left right",
                "break_before on aria-hidden sibling still separates visible neighbors",
            ),
            (16, "Cart (3)", "nested <img> contributes its alt text"),
        ],
    )
}

#[test]
fn step6_hidden_descendants_and_ignored_tags_respect_visibility() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::element("button")
                .dom_id(10)
                .child(NodeBuilder::text("Keep"))
                .child(
                    NodeBuilder::element("span")
                        .aria_hidden(true)
                        .child(NodeBuilder::text(" AriaHid")),
                )
                .child(
                    NodeBuilder::element("span")
                        .visibility_hidden(true)
                        .child(NodeBuilder::text(" VisHid")),
                )
                .child(
                    NodeBuilder::element("div")
                        .annotated_role(AnnotatedRole::ContentHidden)
                        .child(NodeBuilder::text(" ContentHid")),
                )
                .child(NodeBuilder::element("style").child(NodeBuilder::text(" .x{color:red}")))
                .child(NodeBuilder::element("script").child(NodeBuilder::text(" alert(1)")))
                .child(NodeBuilder::element("noscript").child(NodeBuilder::text(" EnableJS")))
                .child(NodeBuilder::element("template").child(NodeBuilder::text(" TemplateText"))),
        ),
    )?;

    assert_eq!(
        visible_name(&tree, 10)?,
        "Keep",
        "VisibleOnly excludes aria-hidden, visibility_hidden, ContentHidden, style, script, noscript, and template descendants"
    );
    assert_eq!(
        hidden_name(&tree, 10)?,
        "Keep AriaHid VisHid ContentHid .x{color:red} alert(1) EnableJS TemplateText",
        "IncludeHidden includes hidden and ignored-tag descendants"
    );
    Ok(())
}

#[test]
fn step6_ua_shadow_slotted_and_closed_details_content_hidden() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            // Button with transparent ua_shadow wrapper: ua_shadow text skipped, slotted child read.
            .child(
                NodeBuilder::element("button").dom_id(10).child(
                    NodeBuilder::container()
                        .ua_shadow(true)
                        .child(NodeBuilder::text("UA_NOISE").ua_shadow(true))
                        .child(NodeBuilder::element("span").child(NodeBuilder::text("Slotted"))),
                ),
            )
            // Closed <details> referenced by labelledby: ua_shadow text skipped, <summary> read,
            // ContentHidden ua_shadow slot hides slotted <p> under VisibleOnly.
            .child(
                NodeBuilder::element("details")
                    .dom_id(20)
                    .child(
                        NodeBuilder::container()
                            .ua_shadow(true)
                            .child(NodeBuilder::text("MARKER_NOISE").ua_shadow(true))
                            .child(
                                NodeBuilder::element("summary").child(NodeBuilder::text("Summary")),
                            ),
                    )
                    .child(
                        NodeBuilder::container()
                            .ua_shadow(true)
                            .annotated_role(AnnotatedRole::ContentHidden)
                            .child(
                                NodeBuilder::element("p")
                                    .break_before(true)
                                    .child(NodeBuilder::text("ClosedBody")),
                            ),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .labelledby(vec![ref_id(20)]),
            ),
    )?;

    assert_eq!(
        visible_name(&tree, 10)?,
        "Slotted",
        "ua_shadow text must be skipped while slotted non-ua_shadow descendants are read"
    );
    assert_eq!(
        visible_name(&tree, 21)?,
        "Summary",
        "closed <details> must skip ua_shadow text, read <summary>, and hide ContentHidden slotted body under VisibleOnly"
    );
    assert_eq!(
        hidden_name(&tree, 21)?,
        "Summary ClosedBody",
        "IncludeHidden must include closed <details> ContentHidden body while still skipping ua_shadow text"
    );
    Ok(())
}

#[test]
fn step6_dom_text_transform_and_contract_s9_ignores_stock_fields() -> Result<(), Box<dyn Error>> {
    let mut apc = ApcBuilder::new("doc-main")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::element("button")
                        .dom_id(10)
                        .child(NodeBuilder::text("BUY NOW").dom_text("Buy Now")),
                )
                .child(
                    NodeBuilder::element("button")
                        .dom_id(20)
                        .label("STOCK_LABEL_IGNORED"),
                )
                .child(
                    NodeBuilder::element("img")
                        .dom_id(21)
                        .label("STOCK_IMG_LABEL"),
                )
                .child(NodeBuilder::element("img").dom_id(22).alt("Contract Alt")),
        )
        .build();

    attach_image_caption(&mut apc, 21, "STOCK_IMAGE_CAPTION_IGNORED");
    attach_image_caption(&mut apc, 22, "STOCK_IMAGE_CAPTION_IGNORED");

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Buy Now",
                "dom_text overrides CSS-transformed text_content",
            ),
            (
                20,
                "",
                "Contract §9: stock ContentAttributes::label is ignored",
            ),
            (
                21,
                "",
                "Contract §9: stock ImageInfo::image_caption is ignored when browseros.alt/title are absent",
            ),
            (
                22,
                "Contract Alt",
                "Contract §9: <img> reads browseros.alt instead of stock ImageInfo::image_caption",
            ),
        ],
    )
}

// ============================================================================
// Step 7 — title fallback
// ============================================================================

#[test]
fn step7_title_fallback_for_empty_hidden_and_whitespace_content() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::element("button").dom_id(10).title("Save"))
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(11)
                    .title("Custom Save"),
            )
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(12)
                    .title("Hidden Fallback")
                    .child(
                        NodeBuilder::element("div")
                            .aria_hidden(true)
                            .child(NodeBuilder::text("x")),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(13)
                    .title("Whitespace Fallback")
                    .child(NodeBuilder::text("  \t\n  ")),
            ),
    )?;

    let btn = tree.accessible_name(node_id(&tree, "doc-main", 10)?, NameVisibility::VisibleOnly);
    assert_eq!(
        btn.text(),
        "Save",
        "empty <button title=...> falls back to title"
    );
    assert!(
        !btn.derived_from_content(),
        "title fallback must leave derived_from_content false"
    );

    assert_visible_names(
        &tree,
        &[
            (
                11,
                "Custom Save",
                "empty <div role=button title=...> falls back to title",
            ),
            (
                12,
                "Hidden Fallback",
                "button with only aria-hidden content falls back to title under VisibleOnly",
            ),
            (
                13,
                "Whitespace Fallback",
                "whitespace-only SelfNode content falls back to title",
            ),
        ],
    )?;
    assert_eq!(
        hidden_name(&tree, 12)?,
        "x",
        "under IncludeHidden the aria-hidden descendant provides content before title fallback"
    );
    Ok(())
}

#[test]
fn step7_presentational_skips_title_except_iframe_and_frame() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("div")
                    .role("none")
                    .dom_id(10)
                    .title("Skip None Div"),
            )
            .child(
                NodeBuilder::element("div")
                    .role("presentation")
                    .dom_id(11)
                    .title("Skip Presentation Div"),
            )
            .child(
                NodeBuilder::element("iframe")
                    .role("none")
                    .dom_id(12)
                    .title("Embedded Frame"),
            )
            .child(
                NodeBuilder::element("frame")
                    .role("none")
                    .dom_id(13)
                    .title("Legacy Frame"),
            )
            .child(
                NodeBuilder::element("iframe")
                    .role("presentation")
                    .dom_id(14)
                    .title("Step1 Prohibited"),
            ),
    )?;

    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        Some(Role::None),
        "<iframe role=none title=...> keeps Role::None"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.role(),
        Some(Role::None),
        "<frame role=none title=...> keeps Role::None"
    );

    assert_visible_names(
        &tree,
        &[
            (10, "", "presentational role=none div skips title"),
            (11, "", "presentational role=presentation div skips title"),
            (
                12,
                "Embedded Frame",
                "presentational <iframe role=none> still uses title at Step 2i",
            ),
            (
                13,
                "Legacy Frame",
                "presentational <frame role=none> still uses title at Step 2i",
            ),
            (
                14,
                "",
                "<iframe role=presentation> is prohibited at Step 1 before Step 2i",
            ),
        ],
    )
}

// ============================================================================
// Descriptions (title vs aria-description vs aria-describedby)
// ============================================================================

#[test]
fn descriptions_describedby_refs_hidden_unrendered_and_provenance() -> Result<(), Box<dyn Error>> {
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("span")
                    .dom_id(10)
                    .child(NodeBuilder::text("Part\u{00A0}One")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(11)
                    .aria_hidden(true)
                    .child(NodeBuilder::text("Hidden Two")),
            )
            .child(
                NodeBuilder::element("div")
                    .dom_id(12)
                    .child(NodeBuilder::text("Shown"))
                    .child(
                        NodeBuilder::element("span")
                            .aria_hidden(true)
                            .child(NodeBuilder::text(" Extra")),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(20)
                    .describedby(vec![ref_id(10), ref_id(11), ref_unrendered("Unrendered")])
                    .aria_description("Ignored AriaDesc")
                    .title("Ignored Title"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .describedby(vec![ref_id(22)])
                    .child(
                        NodeBuilder::element("span")
                            .dom_id(22)
                            .child(NodeBuilder::text("Descendant Note")),
                    ),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(23)
                    .describedby(vec![ref_id(23)])
                    .child(NodeBuilder::text("Self Note")),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(24)
                    .describedby(vec![ref_id(12)]),
            ),
    )?;

    let cache = tree.name_cache();
    let vis = NameVisibility::VisibleOnly;
    let ext = cache.description(node_id(&tree, "doc-main", 20)?, vis);
    assert_eq!(
        ext.text(),
        "Part One Hidden Two Unrendered",
        "aria-describedby joins visible, hidden direct, and unrendered refs and beats aria-description/title"
    );
    assert!(
        !ext.derived_from_content(),
        "external describedby sets derived_from_content false"
    );

    let desc_ref = cache.description(node_id(&tree, "doc-main", 21)?, vis);
    assert_eq!(
        desc_ref.text(),
        "Descendant Note",
        "descendant describedby resolves text"
    );
    assert!(
        desc_ref.derived_from_content(),
        "descendant describedby sets derived_from_content"
    );

    let self_ref = cache.description(node_id(&tree, "doc-main", 23)?, vis);
    assert_eq!(
        self_ref.text(),
        "Self Note",
        "self describedby resolves self text"
    );
    assert!(
        self_ref.derived_from_content(),
        "self describedby sets derived_from_content"
    );

    let vis_ref_id = node_id(&tree, "doc-main", 24)?;
    assert_eq!(
        cache.description(vis_ref_id, vis).text(),
        "Shown",
        "VisibleOnly omits hidden children inside a visible describedby referent"
    );
    assert_eq!(
        cache
            .description(vis_ref_id, NameVisibility::IncludeHidden)
            .text(),
        "Shown Extra",
        "IncludeHidden includes hidden children inside a visible describedby referent"
    );
    Ok(())
}

#[test]
fn descriptions_aria_description_empty_blocks_title_and_shared_title() -> Result<(), Box<dyn Error>>
{
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .aria_description("Direct\u{00A0}Desc")
                    .title("Blocked Title"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .aria_description("")
                    .title("Blocked By Empty"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(12)
                    .title("Shared Tip"),
            ),
    )?;

    let direct =
        tree.accessible_description(node_id(&tree, "doc-main", 10)?, NameVisibility::VisibleOnly);
    assert_eq!(
        direct.text(),
        "Direct Desc",
        "aria-description beats title and normalizes whitespace"
    );
    assert!(
        !direct.derived_from_content(),
        "aria-description leaves derived_from_content false"
    );

    assert!(
        tree.accessible_description(node_id(&tree, "doc-main", 11)?, NameVisibility::VisibleOnly)
            .is_empty(),
        "present empty aria-description=\"\" must block fallback to title"
    );
    assert_eq!(
        visible_name(&tree, 12)?,
        "Shared Tip",
        "empty button uses title as its accessible name"
    );
    assert_eq!(
        tree.accessible_description(node_id(&tree, "doc-main", 12)?, NameVisibility::VisibleOnly)
            .into_text(),
        "Shared Tip",
        "title fallback still provides description even when title was also used as the name"
    );
    Ok(())
}

// ============================================================================
// Error messages
// ============================================================================

#[test]
fn error_messages_aria_invalid_states_multiple_refs_nbsp_and_all_roles()
-> Result<(), Box<dyn Error>> {
    let invalid_false_cases: &[Option<&str>] =
        &[None, Some(""), Some("   "), Some("false"), Some("FALSE")];
    let invalid_true_cases: &[&str] = &["true", "grammar", "spelling", "foo"];

    let mut root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("div")
                .dom_id(2)
                .child(NodeBuilder::text(" \u{00A0}First\u{00A0}err\u{200B} ")),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(3)
                .aria_hidden(true)
                .child(NodeBuilder::text("Hidden\u{00A0}err")),
        );

    for (idx, &attr) in invalid_false_cases.iter().enumerate() {
        let dom = i32::try_from(10 + idx)?;
        let mut node = NodeBuilder::form_control(FormControlType::InputText)
            .tag("input")
            .dom_id(dom)
            .errormessage(vec![ref_id(2)]);
        if let Some(val) = attr {
            node = node.aria_invalid(val);
        }
        root = root.child(node);
    }

    for (idx, &attr) in invalid_true_cases.iter().enumerate() {
        let dom = i32::try_from(20 + idx)?;
        root = root.child(
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .dom_id(dom)
                .aria_invalid(attr)
                .errormessage(vec![
                    ref_id(2),
                    ref_id(2),
                    ref_id(3),
                    ref_unrendered("  Unrendered\u{00A0}msg\u{00AD}  "),
                ]),
        );
    }

    root = root.child(
        NodeBuilder::form_control(FormControlType::InputFile)
            .tag("input")
            .dom_id(40)
            .aria_invalid("true")
            .errormessage(vec![ref_id(3)]),
    );

    let tree = build_tree(root)?;

    for (idx, &attr) in invalid_false_cases.iter().enumerate() {
        let dom = i32::try_from(10 + idx)?;
        assert_eq!(
            tree.accessible_error_message(node_id(&tree, "doc-main", dom)?),
            "",
            "aria-invalid={attr:?} must yield empty error message"
        );
    }

    let expected_multi = "First\u{00A0}err Hidden\u{00A0}err Unrendered\u{00A0}msg";
    for (idx, &attr) in invalid_true_cases.iter().enumerate() {
        let dom = i32::try_from(20 + idx)?;
        assert_eq!(
            tree.accessible_error_message(node_id(&tree, "doc-main", dom)?),
            expected_multi,
            "aria-invalid={attr:?} deduplicates refs, reads hidden/unrendered refs, preserves interior NBSP via as_flat_string, and trims"
        );
    }

    let file_node = node_by_dom(&tree, "doc-main", 40)?;
    assert_eq!(
        file_node.role(),
        Some(Role::Button),
        "<input type=file> computes Role::Button"
    );
    assert_eq!(
        tree.accessible_error_message(file_node.id()),
        "Hidden\u{00A0}err",
        "accessible error message works for all roles including <input type=file>"
    );
    Ok(())
}

// ============================================================================
// Playwright corpus: expect-to-have-accessible.spec.ts (all 11 test blocks)
// ============================================================================

#[test]
fn playwright_corpus_accessible_name_description_and_role() -> Result<(), Box<dyn Error>> {
    // Blocks 1 (`toHaveAccessibleName`), 2 (`toHaveAccessibleDescription`), and 11 (`toHaveRole`).
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(10)
                    .aria_label("Hello"),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(11)
                    .child(NodeBuilder::text("foo\u{00A0}bar\nbaz")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::SelectOne)
                    .tag("select")
                    .dom_id(12)
                    .child(
                        NodeBuilder::element("option")
                            .dom_id(13)
                            .child(NodeBuilder::text("\u{00A0}HTML")),
                    ),
            )
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(20)
                    .aria_description("Hello"),
            )
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(21)
                    .describedby(vec![ref_id(22)]),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(22)
                    .child(NodeBuilder::text("foo\u{00A0}bar\nbaz")),
            )
            .child(
                NodeBuilder::element("div")
                    .role("button")
                    .dom_id(30)
                    .child(NodeBuilder::text("Button!")),
            ),
    )?;

    assert_visible_names(
        &tree,
        &[
            (
                10,
                "Hello",
                "corpus toHaveAccessibleName: aria-label exact text",
            ),
            (
                11,
                "foo bar baz",
                "corpus toHaveAccessibleName: NBSP and newline normalize",
            ),
            (
                13,
                "HTML",
                "corpus toHaveAccessibleName: leading NBSP trims on <option>",
            ),
            (30, "Button!", "corpus toHaveRole: button inner text"),
        ],
    )?;
    assert_eq!(
        tree.accessible_description(node_id(&tree, "doc-main", 20)?, NameVisibility::VisibleOnly)
            .text(),
        "Hello",
        "corpus toHaveAccessibleDescription: aria-description exact text"
    );
    assert_eq!(
        tree.accessible_description(node_id(&tree, "doc-main", 21)?, NameVisibility::VisibleOnly)
            .text(),
        "foo bar baz",
        "corpus toHaveAccessibleDescription: aria-describedby normalizes NBSP and newline"
    );

    let role_div = node_by_dom(&tree, "doc-main", 30)?;
    assert_eq!(
        role_div.role(),
        Some(Role::Button),
        "corpus toHaveRole: <div role=button> computes Role::Button"
    );
    assert_ne!(
        role_div.role(),
        Some(Role::Checkbox),
        "corpus toHaveRole: <div role=button> is not Role::Checkbox"
    );
    Ok(())
}

#[test]
fn playwright_corpus_error_message_single_multi_and_all_roles() -> Result<(), Box<dyn Error>> {
    // Blocks 3 (`toHaveAccessibleErrorMessage`), 4 (multiple refs), and 10 (all roles / file input).
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("form")
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .role("textbox")
                            .dom_id(10)
                            .aria_invalid("true")
                            .errormessage(vec![ref_id(11)]),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(11)
                            .child(NodeBuilder::text("Hello")),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(12)
                            .child(NodeBuilder::text("This should not be considered.")),
                    ),
            )
            .child(
                NodeBuilder::element("form")
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .role("textbox")
                            .dom_id(20)
                            .aria_invalid("true")
                            .errormessage(vec![ref_id(21), ref_id(22)]),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(21)
                            .child(NodeBuilder::text("First error message.")),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(22)
                            .child(NodeBuilder::text("Second error message.")),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(23)
                            .child(NodeBuilder::text("This should not be considered.")),
                    ),
            )
            .child(
                NodeBuilder::element("label")
                    .label_for(30)
                    .child(NodeBuilder::text("File input")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(30)
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(31)]),
            )
            .child(
                NodeBuilder::element("p")
                    .role("alert")
                    .dom_id(31)
                    .child(NodeBuilder::text("File is incorrect")),
            ),
    )?;

    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 10)?),
        "Hello",
        "corpus block 3: single aria-errormessage returns only referenced element text"
    );
    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 20)?),
        "First error message. Second error message.",
        "corpus block 4: multiple aria-errormessage refs join with a single space"
    );

    let file_input = node_by_dom(&tree, "doc-main", 30)?;
    assert_eq!(
        file_input.role(),
        Some(Role::Button),
        "corpus block 10: <input type=file> has Role::Button"
    );
    assert_eq!(
        visible_name(&tree, 30)?,
        "File input",
        "corpus block 10: <input type=file> resolves its <label for>"
    );
    assert_eq!(
        tree.accessible_error_message(file_input.id()),
        "File is incorrect",
        "corpus block 10: <input type=file> resolves its aria-errormessage"
    );
    Ok(())
}

#[test]
fn playwright_corpus_error_message_aria_invalid_matrix() -> Result<(), Box<dyn Error>> {
    // Block 5: `test.describe('toHaveAccessibleErrorMessage should handle aria-invalid attribute')`
    let cases: &[(Option<&str>, &str, &str)] = &[
        (
            None,
            "",
            "corpus block 5a: absent aria-invalid yields empty",
        ),
        (
            Some("false"),
            "",
            "corpus block 5b: aria-invalid=false yields empty",
        ),
        (
            Some(""),
            "",
            "corpus block 5c: aria-invalid=\"\" yields empty",
        ),
        (
            Some("true"),
            "Error message",
            "corpus block 5d: aria-invalid=true shows error message",
        ),
        (
            Some("foo"),
            "Error message",
            "corpus block 5e: unrecognized aria-invalid=foo evaluates as invalid",
        ),
    ];

    let mut root = NodeBuilder::root().dom_id(1).child(
        NodeBuilder::element("div")
            .dom_id(2)
            .child(NodeBuilder::text("Error message")),
    );
    for (idx, &(attr, _, _)) in cases.iter().enumerate() {
        let dom = i32::try_from(10 + idx)?;
        let mut input = NodeBuilder::form_control(FormControlType::InputText)
            .tag("input")
            .role("textbox")
            .dom_id(dom)
            .errormessage(vec![ref_id(2)]);
        if let Some(val) = attr {
            input = input.aria_invalid(val);
        }
        root = root.child(input);
    }

    let tree = build_tree(root)?;

    for (idx, &(_, expected, reason)) in cases.iter().enumerate() {
        let dom = i32::try_from(10 + idx)?;
        assert_eq!(
            tree.accessible_error_message(node_id(&tree, "doc-main", dom)?),
            expected,
            "{reason}"
        );
    }
    Ok(())
}

#[test]
fn playwright_corpus_validity_matrix_and_contract_gap_g6() -> Result<(), Box<dyn Error>> {
    // Blocks 6, 7, 8 (Contract Gap G6), and 9 from `expect-to-have-accessible.spec.ts`.
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(
                NodeBuilder::element("div")
                    .dom_id(2)
                    .child(NodeBuilder::text("Error message")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputNumber)
                    .tag("input")
                    .role("textbox")
                    .dom_id(10)
                    .field_value("101")
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(2)]),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputNumber)
                    .tag("input")
                    .role("textbox")
                    .dom_id(11)
                    .field_value("99")
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(2)]),
            )
            // Block 8 (Contract Gap G6): `validity.valid` is deliberately omitted from the APC
            // wire contract, so `aria-invalid="false"` with an out-of-range value ("101") yields
            // "" rather than consulting constraint validation.
            .child(
                NodeBuilder::form_control(FormControlType::InputNumber)
                    .tag("input")
                    .role("textbox")
                    .dom_id(12)
                    .field_value("101")
                    .aria_invalid("false")
                    .errormessage(vec![ref_id(2)]),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputNumber)
                    .tag("input")
                    .role("textbox")
                    .dom_id(13)
                    .field_value("99")
                    .aria_invalid("false")
                    .errormessage(vec![ref_id(2)]),
            ),
    )?;

    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 10)?),
        "Error message",
        "corpus block 6: validity=false with aria-invalid=true shows error message"
    );
    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 11)?),
        "Error message",
        "corpus block 7: validity=true with aria-invalid=true shows error message"
    );
    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 12)?),
        "",
        "corpus block 8 (Contract Gap G6): validity.valid is omitted from APC, so aria-invalid=false returns empty"
    );
    assert_eq!(
        tree.accessible_error_message(node_id(&tree, "doc-main", 13)?),
        "",
        "corpus block 9: validity=true with aria-invalid=false returns empty"
    );
    Ok(())
}

// ============================================================================
// NameCache memoization, concurrency, QuickJS whitespace parity, 100k stack safety
// ============================================================================

struct NodeExpected {
    id: NodeId,
    name_v: Name,
    name_h: Name,
    desc_v: Description,
    desc_h: Description,
    err: String,
}

fn assert_cache_matches(cache: &NameCache<'_>, row: &NodeExpected) {
    assert_eq!(
        cache.name(row.id, NameVisibility::VisibleOnly),
        &row.name_v,
        "concurrent NameCache visible name must match one-shot query"
    );
    assert_eq!(
        cache.name(row.id, NameVisibility::IncludeHidden),
        &row.name_h,
        "concurrent NameCache hidden name must match one-shot query"
    );
    assert_eq!(
        cache.description(row.id, NameVisibility::VisibleOnly),
        &row.desc_v,
        "concurrent NameCache visible description must match one-shot query"
    );
    assert_eq!(
        cache.description(row.id, NameVisibility::IncludeHidden),
        &row.desc_h,
        "concurrent NameCache hidden description must match one-shot query"
    );
    assert_eq!(
        cache.error_message(row.id),
        row.err.as_str(),
        "concurrent NameCache error_message must match one-shot query"
    );
}

#[test]
fn name_cache_concurrent_scoped_threads_match_oneshot() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-sub").root(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::element("button")
                .dom_id(10)
                .child(NodeBuilder::text("Sub Button")),
        ),
    );
    let tree = build_tree(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::iframe(iframe).dom_id(5))
            .child(
                NodeBuilder::element("span")
                    .dom_id(10)
                    .child(NodeBuilder::text("Main Label")),
            )
            .child(
                NodeBuilder::element("span")
                    .dom_id(11)
                    .aria_hidden(true)
                    .child(NodeBuilder::text("Hidden Desc")),
            )
            .child(
                NodeBuilder::element("label")
                    .dom_id(12)
                    .label_for(20)
                    .child(NodeBuilder::text("Native")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(20)
                    .describedby(vec![ref_id(11)])
                    .aria_invalid("true")
                    .errormessage(vec![ref_id(11)]),
            )
            .child(
                NodeBuilder::element("button")
                    .dom_id(21)
                    .labelledby(vec![ref_id(10)])
                    .title("Button Tip"),
            ),
    )?;

    let cache = tree.name_cache();
    let expected: Vec<NodeExpected> = tree
        .nodes()
        .iter()
        .map(|node| {
            let id = node.id();
            NodeExpected {
                id,
                name_v: tree.accessible_name(id, NameVisibility::VisibleOnly),
                name_h: tree.accessible_name(id, NameVisibility::IncludeHidden),
                desc_v: tree.accessible_description(id, NameVisibility::VisibleOnly),
                desc_h: tree.accessible_description(id, NameVisibility::IncludeHidden),
                err: tree.accessible_error_message(id),
            }
        })
        .collect();

    thread::scope(|scope| {
        for worker_idx in 0..8_usize {
            let cache_ref = &cache;
            let expected_ref = &expected;
            scope.spawn(move || {
                let len = expected_ref.len();
                for step in 0..len {
                    let idx = if worker_idx % 2 == 0 {
                        step
                    } else {
                        len - 1 - step
                    };
                    assert_cache_matches(cache_ref, &expected_ref[idx]);
                }
            });
        }
    });
    Ok(())
}

#[derive(Deserialize)]
struct JsWhitespacePair {
    flat: String,
    normalized: String,
}

fn whitespace_corpus() -> Vec<String> {
    let mut corpus: Vec<String> = [
        "",
        " ",
        "   \t\r\n\x0b\x0c  ",
        "\r\n",
        "a\r\nb",
        "a\r\n\r\nb",
        "\u{00A0}",
        " \u{00A0} ",
        "a\u{00A0}b",
        "a\u{00A0}\u{00A0}b",
        "a \u{00A0} b",
        "a  \t\u{00A0}\r\n  b",
        "\u{00A0}HTML",
        "HTML\u{00A0}",
        "\u{00A0} foo \u{00A0} bar \u{00A0}",
        "\u{200B}",
        "\u{00AD}",
        "\u{200B}\u{00AD}",
        "a\u{200B}b",
        "a\u{00AD}b",
        "a \u{200B} b",
        "a \u{00AD} b",
        "\u{200B} hello \u{00AD}",
        "\u{200B}\u{00A0}a",
        "a\u{00A0}\u{200B} \u{00A0}b",
        "a\u{200B}\u{00A0}\u{00AD}b",
        "a\r\u{200B}\nb",
        "\u{FEFF}",
        "\u{FEFF}hello\u{FEFF}",
        "a\u{FEFF}b",
        "a\u{FEFF}\u{00A0}\u{FEFF}b",
        "\u{FEFF}\u{200B}\u{FEFF}",
        "\u{0085}",
        "\u{0085}hello\u{0085}",
        "a\u{0085}b",
        " \u{0085} ",
        "\u{00A0}\u{0085}\u{00A0}",
        "a \u{0085} b",
        "a\u{1680}b\u{2000}c\u{2001}d\u{2002}e\u{2003}f",
        "g\u{2004}h\u{2005}i\u{2006}j\u{2007}k\u{2008}l\u{2009}m\u{200A}n",
        "p\u{2028}q\u{2029}r\u{202F}s\u{205F}t\u{3000}u",
        "\u{180E}mvs\u{180E}",
        "a\u{200C}b\u{200D}c\u{2060}d",
        "e\u{0301}  \u{00A0}  東京\u{3000}大阪",
        "\u{FEFF}  \u{200B}foo\r\n\t  \u{00AD}bar\u{00A0}\u{200B}  baz\u{3000}\u{0085}\u{2028}qux  \u{00A0}\u{FEFF}",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    for code in 0x00_u32..=0x20_u32 {
        if let Some(ch) = char::from_u32(code) {
            corpus.push(format!("x{ch}y"));
            corpus.push(format!("{ch}x{ch}"));
        }
    }
    corpus
}

#[test]
fn quickjs_differential_as_flat_string_and_normalize_whitespace_parity()
-> Result<(), Box<dyn Error>> {
    use rquickjs::CatchResultExt;

    // Exact expressions from Playwright 1.64.1:
    // - `packages/injected/src/roleUtils.ts:395` (`asFlatString`)
    // - `packages/isomorphic/stringUtils.ts:89` (`normalizeWhiteSpace`)
    let script = r"
        const inputs = JSON.parse(input);
        function asFlatString(s) {
            return s.split('\u00A0')
                .map(chunk => chunk.replace(/\r\n/g, '\n').replace(/[\u200b\u00ad]/g, '').replace(/\s\s*/g, ' '))
                .join('\u00A0')
                .trim();
        }
        function normalizeWhiteSpace(text) {
            return text.replace(/[\u200b\u00ad]/g, '').trim().replace(/\s+/g, ' ');
        }
        JSON.stringify(inputs.map(s => ({
            flat: asFlatString(s),
            normalized: normalizeWhiteSpace(s),
        })));
    ";

    let corpus = whitespace_corpus();
    let input_json = serde_json::to_string(&corpus)?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let output: String = context.with(|ctx| -> Result<String, String> {
        ctx.globals()
            .set("input", input_json)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.eval::<String, _>(script)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
    })?;
    let expected: Vec<JsWhitespacePair> = serde_json::from_str(&output)?;

    assert_eq!(
        corpus.len(),
        expected.len(),
        "QuickJS oracle must return one pair per corpus input"
    );
    for (sample, js) in corpus.iter().zip(expected.iter()) {
        assert_eq!(
            tree::as_flat_string(sample),
            js.flat,
            "as_flat_string must match Playwright's QuickJS asFlatString for {sample:?}"
        );
        assert_eq!(
            tree::normalize_whitespace(sample),
            js.normalized,
            "normalize_whitespace must match Playwright's QuickJS normalizeWhiteSpace for {sample:?}"
        );
    }
    Ok(())
}

fn build_deep_name_chain(depth: usize, leaf_text: &str) -> AnnotatedPageContent {
    let mut current: Option<ContentNode> = None;
    for index in (1..=depth).rev() {
        let mut children_nodes = Vec::new();
        if let Some(child) = current.take() {
            children_nodes.push(child);
        }
        let attrs = if index == depth {
            ContentAttributes {
                common_ancestor_dom_node_id: i32::try_from(index).ok(),
                attribute_type: Some(i32::from(ContentAttributeType::Text)),
                content_data: Some(ContentData::Text(TextInfo {
                    text_content: Some(leaf_text.to_owned()),
                    text_style: None,
                })),
                ..ContentAttributes::default()
            }
        } else {
            let (attr_type, tag) = if index == 1 {
                (ContentAttributeType::Root, "button")
            } else {
                (ContentAttributeType::Container, "div")
            };
            ContentAttributes {
                common_ancestor_dom_node_id: i32::try_from(index).ok(),
                attribute_type: Some(i32::from(attr_type)),
                browseros: Some(Box::new(BrowserOsContentAttributes {
                    tag: Some(tag.to_owned()),
                    ..BrowserOsContentAttributes::default()
                })),
                ..ContentAttributes::default()
            }
        };
        current = Some(ContentNode {
            children_nodes,
            content_attributes: Some(attrs),
        });
    }

    AnnotatedPageContent {
        version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
        mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        root_node: current,
        main_frame_data: Some(FrameData {
            document_identifier: Some(DocumentIdentifier {
                serialized_token: Some("doc-deep-name".to_owned()),
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

struct DeepNameResult {
    text: String,
    derived_from_content: bool,
}

#[test]
fn deep_100k_chain_computes_name_via_cache_and_drops_on_2mib_stack() -> Result<(), Box<dyn Error>> {
    const DEPTH: usize = 100_000;

    let handle = thread::Builder::new()
        .name("deep-name-100k".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| -> Result<DeepNameResult, String> {
            let apc = build_deep_name_chain(DEPTH, "Deep100kLeaf");
            let tree = SnapshotTree::build(apc, BuildOptions::default())
                .map_err(|err| format!("build failed: {err}"))?;
            let cache = tree.name_cache();
            let name = cache.name(NodeId::new(0), NameVisibility::VisibleOnly);
            let result = DeepNameResult {
                text: name.text().to_owned(),
                derived_from_content: name.derived_from_content(),
            };
            drop(cache);
            drop(tree);
            Ok(result)
        })?;

    let result = handle
        .join()
        .map_err(|payload| format!("deep-name-100k thread panicked: {payload:?}"))??;
    assert_eq!(
        result.text, "Deep100kLeaf",
        "100,000-deep chain must compute leaf text via NameCache on a 2 MiB stack without overflow"
    );
    assert!(
        result.derived_from_content,
        "100,000-deep button name gathered from deepest text node must set derived_from_content"
    );
    Ok(())
}
