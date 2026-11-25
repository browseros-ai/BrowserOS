//! Offline tests for [`neo_snapshot_apc::tree`] hidden flags, WAI-ARIA role computation,
//! WAI-ARIA states, Playwright stand-in table agreement, and stack-safety stress tests.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::PathBuf;
use std::{fs, thread};

use neo_snapshot_apc::proto::{
    AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion, AnnotatedRole,
    AxRole, BoundingRect, BrowserOsContentAttributes, BrowserOsPageContentData, BrowserOsRef,
    BrowserOsTriState, ClickabilityReason, ContentAttributeType, ContentAttributes, ContentNode,
    DocumentIdentifier, FormControlType, FrameData, Geometry,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocToken, DomNodeId, FrameBuilder, Hidden, InvalidState,
    MIN_CONTRACT_VERSION, NodeBuilder, NodeKey, Role, SnapshotTree, State, TreeNode, TriState,
};

fn build_tree(root: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-main").root(root).build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn node_by_dom<'a>(
    tree: &'a SnapshotTree,
    doc_token: &str,
    dom_id: i32,
) -> Result<&'a TreeNode, Box<dyn Error>> {
    let token = DocToken::new(doc_token).ok_or("expected non-empty DocToken")?;
    let key = NodeKey::new(token, DomNodeId::new(dom_id));
    tree.node_by_key(&key)
        .ok_or_else(|| format!("missing node ({doc_token}, {dom_id})").into())
}

fn ref_id(dom_id: i32) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: Some(dom_id),
        unrendered_text: None,
    }
}

fn boxed_element(tag: &str) -> NodeBuilder {
    NodeBuilder::element(tag).bounding_box(0, 0, 100, 20)
}

#[test]
fn explicit_roles_round_trip_and_follow_playwright_parsing_rules() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        Role::ALL.len(),
        83,
        "Playwright validRoles must contain exactly 83 concrete WAI-ARIA roles"
    );

    let mut seen_names = BTreeSet::new();
    for &role in Role::ALL {
        let name = role.as_str();
        assert!(
            seen_names.insert(name),
            "Role::ALL must not contain duplicate name {name:?}"
        );
        assert_eq!(Role::parse(name), Some(role));
        assert_eq!(role.to_string(), name);
        assert_eq!(Role::parse(&name.to_ascii_uppercase()), None);
    }

    let root = NodeBuilder::root()
        .child(NodeBuilder::element("div").dom_id(10).role("BUTTON"))
        .child(NodeBuilder::element("button").dom_id(11).role("BUTTON"))
        .child(NodeBuilder::element("div").dom_id(12).role("bogus button"))
        .child(
            NodeBuilder::element("div")
                .dom_id(13)
                .role("heading button"),
        )
        .child(NodeBuilder::element("div").dom_id(14).role("\tbutton "))
        .child(NodeBuilder::element("div").dom_id(15).role("foo\tbutton"))
        .child(
            NodeBuilder::element("div")
                .dom_id(16)
                .role("\u{FEFF}button\u{00A0}"),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(17)
                .role("\u{0085}button"),
        )
        .child(NodeBuilder::element("button").dom_id(18).role("bogus nope"));

    let tree = build_tree(root)?;
    assert_eq!(node_by_dom(&tree, "doc-main", 10)?.role(), None);
    assert_eq!(
        node_by_dom(&tree, "doc-main", 11)?.role(),
        Some(Role::Button)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        Some(Role::Button)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.role(),
        Some(Role::Heading)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 14)?.role(),
        Some(Role::Button)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 15)?.role(),
        None,
        "role tokens split strictly on ASCII space, not tabs"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 16)?.role(),
        Some(Role::Button)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 17)?.role(),
        None,
        "U+0085 NEL is not ECMAScript whitespace and must not be trimmed"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 18)?.role(),
        Some(Role::Button)
    );

    Ok(())
}

#[test]
fn presentation_conflict_focusability_rules() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .child(
            NodeBuilder::element("h1")
                .dom_id(10)
                .role("presentation")
                .focusable(true),
        )
        .child(
            NodeBuilder::element("h1")
                .dom_id(11)
                .role("none")
                .clickability_reason(ClickabilityReason::TabIndex),
        )
        .child(NodeBuilder::element("button").dom_id(12).role("none"))
        .child(
            NodeBuilder::element("button")
                .dom_id(13)
                .role("none")
                .contract_disabled(true)
                .focusable(true),
        )
        .child(
            NodeBuilder::element("a")
                .dom_id(14)
                .href("/home")
                .role("presentation"),
        )
        .child(NodeBuilder::element("a").dom_id(15).role("presentation"));

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 10)?.role(),
        Some(Role::Heading)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 11)?.role(),
        Some(Role::Heading)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        Some(Role::Button)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.role(),
        Some(Role::None),
        "natively disabled button[role=none] is not focusable so none must stick"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 14)?.role(), Some(Role::Link));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 15)?.role(),
        Some(Role::Presentation),
        "a without href is not focusable so presentation must stick"
    );

    Ok(())
}

#[test]
fn presentation_conflict_global_aria_attributes() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .child(
            NodeBuilder::element("p")
                .dom_id(16)
                .role("none")
                .aria_label("prohibited on paragraph"),
        )
        .child(
            NodeBuilder::element("h1")
                .dom_id(17)
                .role("none")
                .aria_label("allowed on heading"),
        )
        .child(
            NodeBuilder::element("h1")
                .dom_id(18)
                .role("presentation")
                .labelledby(vec![ref_id(99)]),
        )
        .child(
            NodeBuilder::element("p")
                .dom_id(19)
                .role("none")
                .describedby(vec![ref_id(99)]),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 16)?.role(),
        Some(Role::None),
        "aria-label is prohibited on paragraph so it must not trigger conflict resolution"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 17)?.role(),
        Some(Role::Heading),
        "aria-label is allowed on heading and must resolve none to Heading"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 18)?.role(),
        Some(Role::Heading),
        "aria-labelledby must resolve presentation to Heading"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 19)?.role(),
        Some(Role::Paragraph),
        "aria-describedby is never label-prohibited so it must override none on paragraph"
    );

    Ok(())
}

#[test]
fn presentation_conflict_own_vs_inherited_aria_hidden() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .child(
            NodeBuilder::element("h1")
                .dom_id(20)
                .role("presentation")
                .aria_hidden(true),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(21)
                .aria_hidden(true)
                .child(
                    NodeBuilder::element("h1")
                        .dom_id(22)
                        .role("presentation")
                        .aria_hidden(true),
                )
                .child(
                    NodeBuilder::element("img")
                        .dom_id(23)
                        .alt("")
                        .aria_hidden(true),
                )
                .child(NodeBuilder::element("img").dom_id(24).alt("")),
        )
        .child(
            NodeBuilder::element("img")
                .dom_id(25)
                .alt("")
                .aria_hidden(true),
        )
        .child(
            NodeBuilder::element("details")
                .dom_id(26)
                .aria_hidden(true)
                .child(
                    NodeBuilder::container().ua_shadow(true).dom_id(27).child(
                        NodeBuilder::element("h1")
                            .dom_id(28)
                            .role("presentation")
                            .aria_hidden(true),
                    ),
                ),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 20)?.role(),
        Some(Role::Heading),
        "own aria-hidden on h1[role=presentation] must trigger conflict resolution"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 22)?.role(),
        Some(Role::Presentation),
        "inherited aria-hidden from parent must not trigger conflict resolution on h1"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 23)?.role(),
        Some(Role::Presentation),
        "inherited aria-hidden (Chromium-propagated) must keep decorative img presentational"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 24)?.role(),
        Some(Role::Presentation),
        "inherited aria-hidden (unpropagated) must keep decorative img presentational"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 25)?.role(),
        Some(Role::Img),
        "own aria-hidden on img[alt=\"\"] counts as global ARIA attr and promotes to img"
    );
    let slotted_h1 = node_by_dom(&tree, "doc-main", 28)?;
    assert!(
        slotted_h1.hidden().is_aria_hidden() && !slotted_h1.hidden().has_own_aria_hidden(),
        "aria-hidden inherited across a ua_shadow slot must not count as an element's own boundary"
    );
    assert_eq!(
        slotted_h1.role(),
        Some(Role::Presentation),
        "inherited aria-hidden across ua_shadow must keep h1[role=presentation] presentational"
    );

    Ok(())
}

#[test]
fn all_51_static_implicit_tags_summary_and_untagged_elements() -> Result<(), Box<dyn Error>> {
    const STATIC_CASES: &[(&str, Role)] = &[
        ("article", Role::Article),
        ("aside", Role::Complementary),
        ("blockquote", Role::Blockquote),
        ("button", Role::Button),
        ("caption", Role::Caption),
        ("code", Role::Code),
        ("datalist", Role::Listbox),
        ("dd", Role::Definition),
        ("del", Role::Deletion),
        ("details", Role::Group),
        ("dfn", Role::Term),
        ("dialog", Role::Dialog),
        ("dt", Role::Term),
        ("em", Role::Emphasis),
        ("fieldset", Role::Group),
        ("figure", Role::Figure),
        ("h1", Role::Heading),
        ("h2", Role::Heading),
        ("h3", Role::Heading),
        ("h4", Role::Heading),
        ("h5", Role::Heading),
        ("h6", Role::Heading),
        ("hr", Role::Separator),
        ("html", Role::Document),
        ("ins", Role::Insertion),
        ("li", Role::Listitem),
        ("main", Role::Main),
        ("mark", Role::Mark),
        ("math", Role::Math),
        ("menu", Role::List),
        ("meter", Role::Meter),
        ("nav", Role::Navigation),
        ("ol", Role::List),
        ("optgroup", Role::Group),
        ("option", Role::Option),
        ("output", Role::Status),
        ("p", Role::Paragraph),
        ("progress", Role::Progressbar),
        ("search", Role::Search),
        ("strong", Role::Strong),
        ("sub", Role::Subscript),
        ("sup", Role::Superscript),
        ("svg", Role::Img),
        ("table", Role::Table),
        ("tbody", Role::Rowgroup),
        ("textarea", Role::Textbox),
        ("tfoot", Role::Rowgroup),
        ("thead", Role::Rowgroup),
        ("time", Role::Time),
        ("tr", Role::Row),
        ("ul", Role::List),
    ];
    assert_eq!(STATIC_CASES.len(), 51);

    let mut root = NodeBuilder::root().dom_id(1);
    for (offset, &(tag, _)) in STATIC_CASES.iter().enumerate() {
        root = root.child(NodeBuilder::element(tag).dom_id(i32::try_from(offset + 100)?));
    }
    root = root
        .child(NodeBuilder::element("summary").dom_id(500))
        .child(NodeBuilder::container().dom_id(501))
        .child(NodeBuilder::element("div").dom_id(502))
        .child(NodeBuilder::element("span").dom_id(503));

    let tree = build_tree(root)?;
    for (offset, &(tag, expected)) in STATIC_CASES.iter().enumerate() {
        let dom_id = i32::try_from(offset + 100)?;
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.role(),
            Some(expected),
            "static implicit role for <{tag}> must be {expected:?}"
        );
    }
    assert_eq!(node_by_dom(&tree, "doc-main", 500)?.role(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 501)?.role(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 502)?.role(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 503)?.role(), None);

    Ok(())
}

#[test]
fn dynamic_roles_for_links() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(NodeBuilder::element("a").dom_id(10).href(""))
        .child(NodeBuilder::element("a").dom_id(11).href("/docs"))
        .child(
            NodeBuilder::anchor("https://example.com")
                .tag("a")
                .dom_id(12),
        )
        .child(NodeBuilder::element("area").dom_id(13).href(""))
        .child(NodeBuilder::element("area").dom_id(14));

    let tree = build_tree(root)?;
    assert_eq!(node_by_dom(&tree, "doc-main", 10)?.role(), Some(Role::Link));
    assert_eq!(node_by_dom(&tree, "doc-main", 11)?.role(), Some(Role::Link));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        None,
        "AnchorData.url alone without browseros.href must never confer Link role"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 13)?.role(), Some(Role::Link));
    assert_eq!(node_by_dom(&tree, "doc-main", 14)?.role(), None);
    Ok(())
}

#[test]
fn dynamic_roles_for_landmark_ancestors() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(NodeBuilder::element("header").dom_id(20))
        .child(NodeBuilder::element("footer").dom_id(21))
        .child(
            NodeBuilder::element("article")
                .dom_id(22)
                .child(NodeBuilder::element("header").dom_id(23))
                .child(NodeBuilder::element("footer").dom_id(24)),
        )
        .child(
            NodeBuilder::element("aside")
                .dom_id(25)
                .child(NodeBuilder::element("header").dom_id(26)),
        )
        .child(
            NodeBuilder::element("main")
                .dom_id(27)
                .child(NodeBuilder::element("header").dom_id(28)),
        )
        .child(
            NodeBuilder::element("nav")
                .dom_id(30)
                .child(NodeBuilder::element("footer").dom_id(31)),
        )
        .child(
            NodeBuilder::element("section")
                .dom_id(32)
                .child(NodeBuilder::element("header").dom_id(33)),
        )
        .child(
            NodeBuilder::container()
                .dom_id(37)
                .role("complementary")
                .child(NodeBuilder::element("header").dom_id(38)),
        )
        .child(
            NodeBuilder::container()
                .dom_id(39)
                .role("region")
                .child(NodeBuilder::element("footer").dom_id(40)),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 20)?.role(),
        Some(Role::Banner)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 21)?.role(),
        Some(Role::Contentinfo)
    );
    for blocked in [23, 24, 26, 28, 31, 33, 38, 40] {
        assert_eq!(
            node_by_dom(&tree, "doc-main", blocked)?.role(),
            None,
            "landmark node {blocked} inside sectioning/landmark ancestor must have no implicit role"
        );
    }

    Ok(())
}

#[test]
fn dynamic_roles_for_landmark_iframes_and_overrides() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-iframe").root(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::element("header").dom_id(2))
            .child(NodeBuilder::element("footer").dom_id(3)),
    );

    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("main")
                .dom_id(27)
                .child(NodeBuilder::iframe(iframe).tag("iframe").dom_id(29)),
        )
        .child(
            NodeBuilder::element("article")
                .dom_id(34)
                .role("presentation")
                .child(NodeBuilder::element("header").dom_id(35))
                .child(NodeBuilder::element("footer").dom_id(36)),
        )
        .child(
            NodeBuilder::container()
                .dom_id(41)
                .role("  main  ")
                .child(NodeBuilder::element("header").dom_id(42)),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-iframe", 2)?.role(),
        Some(Role::Banner),
        "header inside iframe must not be blocked by outer document's <main>"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-iframe", 3)?.role(),
        Some(Role::Contentinfo),
        "footer inside iframe must not be blocked by outer document's <main>"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 35)?.role(),
        Some(Role::Banner)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 36)?.role(),
        Some(Role::Contentinfo)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 42)?.role(),
        Some(Role::Banner),
        "kAncestorPreventingLandmark matches literal [role=main] without trimming"
    );

    Ok(())
}

#[test]
fn dynamic_roles_for_forms_sections_and_images() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(NodeBuilder::element("form").dom_id(50))
        .child(NodeBuilder::element("form").dom_id(51).aria_label(""))
        .child(NodeBuilder::element("form").dom_id(52).aria_label("Search"))
        .child(
            NodeBuilder::element("form")
                .dom_id(53)
                .labelledby(vec![ref_id(1)]),
        )
        .child(NodeBuilder::element("section").dom_id(54))
        .child(NodeBuilder::element("section").dom_id(55).aria_label(""))
        .child(
            NodeBuilder::element("section")
                .dom_id(56)
                .aria_label("Filters"),
        )
        .child(
            NodeBuilder::element("section")
                .dom_id(57)
                .labelledby(vec![ref_id(1)]),
        )
        .child(NodeBuilder::element("img").dom_id(60))
        .child(NodeBuilder::element("img").dom_id(61).alt("Logo"))
        .child(NodeBuilder::element("img").dom_id(62).alt(""))
        .child(
            NodeBuilder::element("img")
                .dom_id(63)
                .alt("")
                .title("Tooltip"),
        )
        .child(NodeBuilder::element("img").dom_id(64).alt("").title(""))
        .child(
            NodeBuilder::element("img")
                .dom_id(65)
                .alt("")
                .focusable(true),
        )
        .child(
            NodeBuilder::element("img")
                .dom_id(66)
                .alt("")
                .clickability_reason(ClickabilityReason::TabIndex),
        )
        .child(
            NodeBuilder::element("img")
                .dom_id(67)
                .alt("")
                .aria_label(""),
        )
        .child(
            NodeBuilder::element("img")
                .dom_id(68)
                .alt("")
                .describedby(vec![ref_id(1)]),
        );

    let tree = build_tree(root)?;
    assert_eq!(node_by_dom(&tree, "doc-main", 50)?.role(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 51)?.role(), Some(Role::Form));
    assert_eq!(node_by_dom(&tree, "doc-main", 52)?.role(), Some(Role::Form));
    assert_eq!(node_by_dom(&tree, "doc-main", 53)?.role(), Some(Role::Form));
    assert_eq!(node_by_dom(&tree, "doc-main", 54)?.role(), None);
    assert_eq!(
        node_by_dom(&tree, "doc-main", 55)?.role(),
        Some(Role::Region)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 56)?.role(),
        Some(Role::Region)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 57)?.role(),
        Some(Role::Region)
    );

    assert_eq!(node_by_dom(&tree, "doc-main", 60)?.role(), Some(Role::Img));
    assert_eq!(node_by_dom(&tree, "doc-main", 61)?.role(), Some(Role::Img));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 62)?.role(),
        Some(Role::Presentation)
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 63)?.role(), Some(Role::Img));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 64)?.role(),
        Some(Role::Presentation),
        "empty title attribute behaves as falsy in Playwright img check"
    );
    for promoted in [65, 66, 67, 68] {
        assert_eq!(
            node_by_dom(&tree, "doc-main", promoted)?.role(),
            Some(Role::Img),
            "img node {promoted} with focusability or global ARIA attr must keep Img role"
        );
    }

    Ok(())
}

const INPUT_ROLE_CASES: &[(Option<FormControlType>, bool, Option<Role>)] = &[
    (None, false, Some(Role::Textbox)),
    (None, true, Some(Role::Combobox)),
    (
        Some(FormControlType::Unspecified),
        false,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::Unspecified),
        true,
        Some(Role::Combobox),
    ),
    (Some(FormControlType::InputText), false, Some(Role::Textbox)),
    (Some(FormControlType::InputText), true, Some(Role::Combobox)),
    (
        Some(FormControlType::InputSearch),
        false,
        Some(Role::Searchbox),
    ),
    (
        Some(FormControlType::InputSearch),
        true,
        Some(Role::Combobox),
    ),
    (
        Some(FormControlType::InputEmail),
        false,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::InputEmail),
        true,
        Some(Role::Combobox),
    ),
    (
        Some(FormControlType::InputTelephone),
        false,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::InputTelephone),
        true,
        Some(Role::Combobox),
    ),
    (Some(FormControlType::InputUrl), false, Some(Role::Textbox)),
    (Some(FormControlType::InputUrl), true, Some(Role::Combobox)),
    (
        Some(FormControlType::InputPassword),
        false,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::InputPassword),
        true,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::InputNumber),
        false,
        Some(Role::Spinbutton),
    ),
    (
        Some(FormControlType::InputNumber),
        true,
        Some(Role::Spinbutton),
    ),
    (Some(FormControlType::InputRange), false, Some(Role::Slider)),
    (
        Some(FormControlType::InputCheckbox),
        false,
        Some(Role::Checkbox),
    ),
    (Some(FormControlType::InputRadio), false, Some(Role::Radio)),
    (
        Some(FormControlType::InputButton),
        false,
        Some(Role::Button),
    ),
    (
        Some(FormControlType::InputSubmit),
        false,
        Some(Role::Button),
    ),
    (Some(FormControlType::InputReset), false, Some(Role::Button)),
    (Some(FormControlType::InputImage), false, Some(Role::Button)),
    (Some(FormControlType::InputFile), false, Some(Role::Button)),
    (Some(FormControlType::InputHidden), false, None),
    (
        Some(FormControlType::InputColor),
        false,
        Some(Role::Textbox),
    ),
    (Some(FormControlType::InputDate), false, Some(Role::Textbox)),
    (Some(FormControlType::InputDate), true, Some(Role::Textbox)),
    (
        Some(FormControlType::InputDatetimeLocal),
        false,
        Some(Role::Textbox),
    ),
    (
        Some(FormControlType::InputMonth),
        false,
        Some(Role::Textbox),
    ),
    (Some(FormControlType::InputTime), false, Some(Role::Textbox)),
    (Some(FormControlType::InputWeek), false, Some(Role::Textbox)),
];

#[test]
fn dynamic_roles_for_all_22_input_types() -> Result<(), Box<dyn Error>> {
    let mut root = NodeBuilder::root().dom_id(1);
    for (offset, &(control_type, has_datalist, _)) in INPUT_ROLE_CASES.iter().enumerate() {
        let mut builder = match control_type {
            Some(kind) => NodeBuilder::form_control(kind).tag("input"),
            None => NodeBuilder::element("input"),
        };
        if has_datalist {
            builder = builder.has_datalist(true);
        }
        root = root.child(builder.dom_id(i32::try_from(offset + 100)?));
    }

    let tree = build_tree(root)?;
    for (offset, &(control_type, has_datalist, expected)) in INPUT_ROLE_CASES.iter().enumerate() {
        let dom_id = i32::try_from(offset + 100)?;
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.role(),
            expected,
            "input ({control_type:?}, has_datalist={has_datalist}) must resolve to {expected:?}"
        );
    }

    Ok(())
}

#[test]
fn dynamic_roles_for_select_and_td_cells() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .tag("select")
                .dom_id(10),
        )
        .child(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .tag("select")
                .list_box(false)
                .dom_id(11),
        )
        .child(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .tag("select")
                .list_box(true)
                .dom_id(12),
        )
        .child(
            NodeBuilder::form_control(FormControlType::SelectMultiple)
                .tag("select")
                .dom_id(13),
        )
        .child(
            NodeBuilder::form_control(FormControlType::SelectMultiple)
                .tag("select")
                .list_box(false)
                .dom_id(14),
        )
        .child(
            NodeBuilder::element("table").dom_id(20).child(
                NodeBuilder::element("tr")
                    .dom_id(21)
                    .child(NodeBuilder::element("td").dom_id(22)),
            ),
        )
        .child(
            NodeBuilder::element("table")
                .dom_id(23)
                .role("table")
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(24)
                        .child(NodeBuilder::element("td").dom_id(25)),
                ),
        )
        .child(
            NodeBuilder::element("table").dom_id(26).role("grid").child(
                NodeBuilder::element("tr")
                    .dom_id(27)
                    .child(NodeBuilder::element("td").dom_id(28)),
            ),
        )
        .child(
            NodeBuilder::element("table")
                .dom_id(29)
                .role("treegrid")
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(30)
                        .child(NodeBuilder::element("td").dom_id(31)),
                ),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 10)?.role(),
        Some(Role::Combobox)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 11)?.role(),
        Some(Role::Combobox)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.role(),
        Some(Role::Listbox)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.role(),
        Some(Role::Listbox),
        "select multiple without browseros.list_box must fall back to Listbox"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 14)?.role(),
        Some(Role::Combobox),
        "explicit browseros.list_box=false must override SelectMultiple fallback"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 22)?.role(), Some(Role::Cell));
    assert_eq!(node_by_dom(&tree, "doc-main", 25)?.role(), Some(Role::Cell));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 28)?.role(),
        Some(Role::Gridcell)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 31)?.role(),
        Some(Role::Gridcell)
    );

    Ok(())
}

#[test]
fn dynamic_roles_for_th_scope() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root().dom_id(1).child(
        NodeBuilder::element("table").dom_id(30).child(
            NodeBuilder::element("tr")
                .dom_id(31)
                .child(NodeBuilder::element("th").dom_id(32).scope("col"))
                .child(NodeBuilder::element("th").dom_id(33).scope("colgroup"))
                .child(NodeBuilder::element("th").dom_id(34).scope("row"))
                .child(NodeBuilder::element("th").dom_id(35).scope("rowgroup"))
                .child(
                    NodeBuilder::element("td")
                        .dom_id(36)
                        .child(NodeBuilder::text("value")),
                )
                .child(NodeBuilder::element("th").dom_id(37).scope("COL")),
        ),
    );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 32)?.role(),
        Some(Role::Columnheader)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 33)?.role(),
        Some(Role::Columnheader)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 34)?.role(),
        Some(Role::Rowheader)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 35)?.role(),
        Some(Role::Rowheader)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 37)?.role(),
        Some(Role::Rowheader),
        "uppercase scope=COL must be ignored so adjacent non-empty td makes it Rowheader"
    );
    Ok(())
}

#[test]
fn dynamic_roles_for_th_row_count() -> Result<(), Box<dyn Error>> {
    let iframe_table = FrameBuilder::new("doc-table-iframe").root(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::element("table")
                .dom_id(2)
                .child(NodeBuilder::element("tr").dom_id(3))
                .child(NodeBuilder::element("tr").dom_id(4)),
        ),
    );

    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("table").dom_id(40).child(
                NodeBuilder::element("tr")
                    .dom_id(41)
                    .child(NodeBuilder::element("th").dom_id(42)),
            ),
        )
        .child(
            NodeBuilder::element("table")
                .dom_id(43)
                .child(
                    NodeBuilder::element("thead").dom_id(44).child(
                        NodeBuilder::element("tr")
                            .dom_id(45)
                            .child(NodeBuilder::element("th").dom_id(46)),
                    ),
                )
                .child(
                    NodeBuilder::element("tbody").dom_id(47).child(
                        NodeBuilder::element("tr")
                            .dom_id(48)
                            .child(NodeBuilder::element("td").dom_id(49)),
                    ),
                ),
        )
        .child(
            NodeBuilder::element("table").dom_id(50).child(
                NodeBuilder::element("tr").dom_id(51).child(
                    NodeBuilder::element("th")
                        .dom_id(52)
                        .child(
                            NodeBuilder::element("table")
                                .dom_id(53)
                                .child(NodeBuilder::element("tr").dom_id(54))
                                .child(NodeBuilder::element("tr").dom_id(55)),
                        )
                        .child(NodeBuilder::iframe(iframe_table).tag("iframe").dom_id(56)),
                ),
            ),
        )
        .child(NodeBuilder::element("th").dom_id(90));

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 42)?.role(),
        None,
        "lone th in 1-row table has no column to head"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 46)?.role(),
        Some(Role::Columnheader)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 52)?.role(),
        None,
        "nested table rows and iframe table rows must not count toward outer table row count"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 90)?.role(),
        Some(Role::Columnheader),
        "detached th with no tr parent must default to Columnheader"
    );

    Ok(())
}

#[test]
fn dynamic_roles_for_th_sibling_heuristics() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("table").dom_id(60).child(
                NodeBuilder::element("tr")
                    .dom_id(61)
                    .child(NodeBuilder::element("th").dom_id(62))
                    .child(NodeBuilder::element("th").dom_id(63))
                    .child(NodeBuilder::element("th").dom_id(64)),
            ),
        )
        .child(
            NodeBuilder::element("table")
                .dom_id(70)
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(71)
                        .child(NodeBuilder::element("th").dom_id(72))
                        .child(
                            NodeBuilder::element("td")
                                .dom_id(73)
                                .child(NodeBuilder::text("   ").dom_text("real")),
                        ),
                )
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(74)
                        .child(NodeBuilder::element("th").dom_id(75))
                        .child(
                            NodeBuilder::element("td")
                                .dom_id(76)
                                .child(NodeBuilder::element("span").dom_id(77)),
                        ),
                )
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(78)
                        .child(NodeBuilder::element("th").dom_id(79))
                        .child(
                            NodeBuilder::element("td")
                                .dom_id(80)
                                .child(NodeBuilder::text(" \t\n\u{00A0}\u{FEFF} ")),
                        ),
                )
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(81)
                        .child(NodeBuilder::element("th").dom_id(82))
                        .child(
                            NodeBuilder::element("td")
                                .dom_id(83)
                                .child(NodeBuilder::text("\u{0085}")),
                        ),
                )
                .child(
                    NodeBuilder::element("tr")
                        .dom_id(84)
                        .child(NodeBuilder::element("th").dom_id(85))
                        .child(
                            NodeBuilder::element("td")
                                .dom_id(86)
                                .child(NodeBuilder::text("generated").without_dom_id()),
                        ),
                ),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 63)?.role(),
        Some(Role::Columnheader),
        "th between two th siblings must be Columnheader"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 72)?.role(),
        Some(Role::Rowheader),
        "th next to td with non-empty dom_text must be Rowheader"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 75)?.role(),
        Some(Role::Rowheader),
        "th next to td with empty child element must be Rowheader"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 79)?.role(),
        Some(Role::Columnheader),
        "th next to JS-whitespace-only td must default to Columnheader"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 82)?.role(),
        Some(Role::Rowheader),
        "U+0085 NEL is not JS whitespace so td is non-empty and th becomes Rowheader"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 85)?.role(),
        Some(Role::Columnheader),
        "synthetic CSS-generated text (without dom_node_id) must not make an adjacent td non-empty"
    );

    Ok(())
}

#[test]
fn presentation_inheritance_covers_all_10_pairs() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("dl")
                .dom_id(10)
                .role("presentation")
                .child(NodeBuilder::element("dt").dom_id(11))
                .child(NodeBuilder::element("dd").dom_id(12))
                .child(
                    NodeBuilder::container()
                        .dom_id(13)
                        .child(NodeBuilder::element("dt").dom_id(14))
                        .child(NodeBuilder::element("dd").dom_id(15)),
                ),
        )
        .child(
            NodeBuilder::element("ol")
                .dom_id(20)
                .role("none")
                .child(NodeBuilder::element("li").dom_id(21)),
        )
        .child(
            NodeBuilder::element("ul")
                .dom_id(22)
                .role("presentation")
                .child(NodeBuilder::element("li").dom_id(23)),
        )
        .child(
            NodeBuilder::element("menu")
                .dom_id(24)
                .role("presentation")
                .child(NodeBuilder::element("li").dom_id(25)),
        )
        .child(
            NodeBuilder::element("table")
                .dom_id(30)
                .role("presentation")
                .child(
                    NodeBuilder::element("thead").dom_id(31).child(
                        NodeBuilder::element("tr")
                            .dom_id(32)
                            .child(NodeBuilder::element("th").dom_id(33))
                            .child(NodeBuilder::element("th").dom_id(34)),
                    ),
                )
                .child(
                    NodeBuilder::element("tbody").dom_id(35).child(
                        NodeBuilder::element("tr")
                            .dom_id(36)
                            .child(NodeBuilder::element("td").dom_id(37)),
                    ),
                )
                .child(
                    NodeBuilder::element("tfoot").dom_id(38).child(
                        NodeBuilder::element("tr")
                            .dom_id(39)
                            .child(NodeBuilder::element("td").dom_id(40)),
                    ),
                ),
        );

    let tree = build_tree(root)?;
    for dom_id in [11, 12, 14, 15, 23, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40] {
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.role(),
            Some(Role::Presentation),
            "node {dom_id} must inherit Presentation from its required parent"
        );
    }
    assert_eq!(node_by_dom(&tree, "doc-main", 13)?.role(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 21)?.role(), Some(Role::None));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 25)?.role(),
        Some(Role::Listitem),
        "li under menu[role=presentation] must NOT inherit presentation"
    );

    Ok(())
}

#[test]
fn presentation_inheritance_conflict_quirks_and_negative_contract() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("ul")
                .dom_id(50)
                .role("presentation")
                .focusable(true)
                .child(NodeBuilder::element("li").dom_id(51)),
        )
        .child(
            NodeBuilder::element("ul")
                .dom_id(52)
                .role("presentation")
                .aria_label("x")
                .child(NodeBuilder::element("li").dom_id(53)),
        )
        .child(
            NodeBuilder::element("ul")
                .dom_id(54)
                .role("none")
                .aria_label("x")
                .child(NodeBuilder::element("li").dom_id(55)),
        )
        .child(
            NodeBuilder::heading()
                .dom_id(60)
                .annotated_role(AnnotatedRole::Main)
                .aria_role(AxRole::Button),
        )
        .child(
            NodeBuilder::anchor("https://example.com")
                .dom_id(61)
                .annotated_role(AnnotatedRole::Nav)
                .aria_role(AxRole::Link),
        )
        .child(
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .dom_id(62)
                .annotated_role(AnnotatedRole::Header)
                .aria_role(AxRole::Button),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 51)?.role(),
        Some(Role::Listitem),
        "focusable ul[role=presentation] triggers conflict resolution and blocks inheritance"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 52)?.role(), Some(Role::List));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 53)?.role(),
        Some(Role::Presentation),
        "li under ul[role=presentation][aria-label] inherits presentation because aria-label is prohibited on presentation"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 54)?.role(), Some(Role::List));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 55)?.role(),
        Some(Role::Listitem),
        "li under ul[role=none][aria-label] does NOT inherit because none is not label-prohibited"
    );
    for dom_id in [60, 61, 62] {
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.role(),
            None,
            "node {dom_id} without browseros.tag or browseros.role must never derive a role from stock APC fields"
        );
    }

    Ok(())
}

#[test]
fn states_cover_checked_and_pressed() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("div")
                .dom_id(10)
                .role("checkbox")
                .checked_state(BrowserOsTriState::True),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(11)
                .role("switch")
                .checked_state(BrowserOsTriState::False),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(12)
                .role("menuitemcheckbox")
                .checked_state(BrowserOsTriState::Mixed),
        )
        .child(NodeBuilder::element("div").dom_id(13).role("treeitem"))
        .child(
            NodeBuilder::form_control(FormControlType::InputCheckbox)
                .tag("input")
                .checked(true)
                .dom_id(14),
        )
        .child(
            NodeBuilder::form_control(FormControlType::InputRadio)
                .tag("input")
                .checked(false)
                .dom_id(15),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(16)
                .checked_state(BrowserOsTriState::True),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(20)
                .pressed_state(BrowserOsTriState::True),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(21)
                .pressed_state(BrowserOsTriState::Mixed),
        )
        .child(NodeBuilder::element("button").dom_id(22))
        .child(
            NodeBuilder::element("a")
                .dom_id(23)
                .href("/")
                .pressed_state(BrowserOsTriState::True),
        );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 10)?.state().checked(),
        Some(TriState::True)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 11)?.state().checked(),
        Some(TriState::False)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 12)?.state().checked(),
        Some(TriState::Mixed)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 13)?.state().checked(),
        Some(TriState::False)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 14)?.state().checked(),
        Some(TriState::True),
        "stock FormControlData::is_checked must act as fallback when browseros.checked is absent"
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 15)?.state().checked(),
        Some(TriState::False)
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 16)?.state().checked(), None);

    assert_eq!(
        node_by_dom(&tree, "doc-main", 20)?.state().pressed(),
        Some(TriState::True)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 21)?.state().pressed(),
        Some(TriState::Mixed)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 22)?.state().pressed(),
        Some(TriState::False)
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 23)?.state().pressed(), None);

    Ok(())
}

#[test]
fn states_cover_expanded_and_selected() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("button")
                .dom_id(30)
                .expanded_state(BrowserOsTriState::True),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(31)
                .expanded_state(BrowserOsTriState::False),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(32)
                .expanded_state(BrowserOsTriState::Mixed),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(33)
                .expanded_state(BrowserOsTriState::Unset),
        )
        .child(NodeBuilder::element("button").dom_id(34))
        .child(
            NodeBuilder::element("h1")
                .dom_id(35)
                .expanded_state(BrowserOsTriState::True),
        )
        .child(NodeBuilder::element("option").dom_id(40).selected(true))
        .child(NodeBuilder::element("option").dom_id(41).selected(false))
        .child(NodeBuilder::element("div").dom_id(42).role("tab"))
        .child(NodeBuilder::element("button").dom_id(43).selected(true));

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 30)?.state().expanded(),
        Some(true)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 31)?.state().expanded(),
        Some(false)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 32)?.state().expanded(),
        Some(false)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 33)?.state().expanded(),
        None,
        "Unset aria-expanded must stay None and differ from Some(false)"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 34)?.state().expanded(), None);
    assert_eq!(node_by_dom(&tree, "doc-main", 35)?.state().expanded(), None);

    assert_eq!(
        node_by_dom(&tree, "doc-main", 40)?.state().selected(),
        Some(true)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 41)?.state().selected(),
        Some(false)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 42)?.state().selected(),
        Some(false)
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 43)?.state().selected(), None);

    Ok(())
}

#[test]
fn states_cover_level() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(NodeBuilder::element("h1").dom_id(50))
        .child(NodeBuilder::element("h2").dom_id(51).level(5))
        .child(NodeBuilder::element("h6").dom_id(52))
        .child(
            NodeBuilder::element("div")
                .dom_id(53)
                .role("heading")
                .level(3),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(54)
                .role("heading")
                .level(0),
        )
        .child(NodeBuilder::element("li").dom_id(55))
        .child(NodeBuilder::element("tr").dom_id(56).level(2))
        .child(
            NodeBuilder::element("div")
                .dom_id(57)
                .role("treeitem")
                .level(4),
        )
        .child(
            NodeBuilder::element("h2")
                .dom_id(58)
                .role("button")
                .level(2),
        );

    let tree = build_tree(root)?;
    assert_eq!(node_by_dom(&tree, "doc-main", 50)?.state().level(), Some(1));
    assert_eq!(
        node_by_dom(&tree, "doc-main", 51)?.state().level(),
        Some(2),
        "native h2 heading level must beat aria-level=5"
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 52)?.state().level(), Some(6));
    assert_eq!(node_by_dom(&tree, "doc-main", 53)?.state().level(), Some(3));
    assert_eq!(node_by_dom(&tree, "doc-main", 54)?.state().level(), Some(0));
    assert_eq!(node_by_dom(&tree, "doc-main", 55)?.state().level(), Some(0));
    assert_eq!(node_by_dom(&tree, "doc-main", 56)?.state().level(), Some(2));
    assert_eq!(node_by_dom(&tree, "doc-main", 57)?.state().level(), Some(4));
    assert_eq!(node_by_dom(&tree, "doc-main", 58)?.state().level(), None);

    Ok(())
}

#[test]
fn states_cover_native_disabled() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("button")
                .dom_id(10)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(11)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("select")
                .dom_id(12)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("textarea")
                .dom_id(13)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("option")
                .dom_id(14)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("optgroup")
                .dom_id(15)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("fieldset")
                .dom_id(16)
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(17)
                .role("heading")
                .contract_disabled(true),
        )
        .child(
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .dom_id(25)
                .disabled(true)
                .readonly(true),
        );

    let tree = build_tree(root)?;
    for dom_id in 10..=15 {
        let state = node_by_dom(&tree, "doc-main", dom_id)?.state();
        assert_eq!(
            state.disabled(),
            Some(true),
            "native form control dom_id={dom_id} with contract disabled must report disabled=Some(true)"
        );
        assert!(
            state.is_disabled(),
            "native form control dom_id={dom_id} with contract disabled must report is_disabled=true"
        );
    }
    let fieldset = node_by_dom(&tree, "doc-main", 16)?.state();
    assert_eq!(
        fieldset.disabled(),
        Some(false),
        "<fieldset disabled> must NOT be treated as disabled"
    );
    assert!(!fieldset.is_disabled());

    let disabled_heading = node_by_dom(&tree, "doc-main", 17)?.state();
    assert_eq!(disabled_heading.disabled(), None);
    assert!(
        disabled_heading.is_disabled(),
        "native <button disabled role=heading> must still report is_disabled() == true"
    );

    let stock_disabled_input = node_by_dom(&tree, "doc-main", 25)?.state();
    assert_eq!(
        stock_disabled_input.disabled(),
        Some(false),
        "InteractionInfo.is_disabled and FormControlData.is_readonly must never count as disabled"
    );
    assert!(!stock_disabled_input.is_disabled());

    Ok(())
}

#[test]
fn states_cover_aria_disabled() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-child").root(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::element("button").dom_id(20)),
    );

    let root = NodeBuilder::root().dom_id(1).child(
        NodeBuilder::element("div")
            .dom_id(20)
            .aria_disabled(true)
            .child(NodeBuilder::element("button").dom_id(21))
            .child(
                NodeBuilder::element("button")
                    .dom_id(22)
                    .aria_disabled(false),
            )
            .child(NodeBuilder::element("h1").dom_id(23))
            .child(NodeBuilder::iframe(iframe).tag("iframe").dom_id(24)),
    );

    let tree = build_tree(root)?;
    assert_eq!(
        node_by_dom(&tree, "doc-main", 21)?.state().disabled(),
        Some(true)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 22)?.state().disabled(),
        Some(false)
    );
    assert_eq!(node_by_dom(&tree, "doc-main", 23)?.state().disabled(), None);
    assert!(!node_by_dom(&tree, "doc-main", 23)?.state().is_disabled());
    assert_eq!(
        node_by_dom(&tree, "doc-child", 20)?.state().disabled(),
        Some(false),
        "aria-disabled must not cross iframe document boundaries"
    );

    Ok(())
}

#[test]
fn states_cover_invalid() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(NodeBuilder::element("input").dom_id(30))
        .child(NodeBuilder::element("input").dom_id(31).aria_invalid(""))
        .child(
            NodeBuilder::element("input")
                .dom_id(32)
                .aria_invalid(" \t\u{FEFF} "),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(33)
                .aria_invalid("false"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(34)
                .aria_invalid("FALSE"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(35)
                .aria_invalid("true"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(36)
                .aria_invalid("grammar"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(37)
                .aria_invalid("spelling"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(38)
                .aria_invalid(" false "),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(39)
                .aria_invalid("TRUE"),
        )
        .child(
            NodeBuilder::element("input")
                .dom_id(40)
                .aria_invalid("other"),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(41)
                .aria_invalid("true"),
        );

    let tree = build_tree(root)?;
    for dom_id in 30..=34 {
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.state().invalid(),
            Some(InvalidState::False)
        );
    }
    assert_eq!(
        node_by_dom(&tree, "doc-main", 35)?.state().invalid(),
        Some(InvalidState::True)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 36)?.state().invalid(),
        Some(InvalidState::Grammar)
    );
    assert_eq!(
        node_by_dom(&tree, "doc-main", 37)?.state().invalid(),
        Some(InvalidState::Spelling)
    );
    for dom_id in [38, 39, 40] {
        assert_eq!(
            node_by_dom(&tree, "doc-main", dom_id)?.state().invalid(),
            Some(InvalidState::True)
        );
    }
    assert_eq!(node_by_dom(&tree, "doc-main", 41)?.state().invalid(), None);
    Ok(())
}

#[test]
fn states_cover_active_inert_and_all_role_gates() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-child").focused_node_id(20).root(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::element("button").dom_id(20)),
    );

    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("button")
                .dom_id(20)
                .child(NodeBuilder::iframe(iframe).tag("iframe").dom_id(42)),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(50)
                .child(NodeBuilder::text("Press").dom_id(50))
                .child(NodeBuilder::container().dom_id(50).ua_shadow(true)),
        )
        .child(
            NodeBuilder::element("body")
                .dom_id(60)
                .inert(true)
                .child(NodeBuilder::element("dialog").dom_id(61)),
        );

    let apc = ApcBuilder::new("doc-main")
        .main_focused_node_id(50)
        .root(root)
        .build();
    let tree = SnapshotTree::build(apc, BuildOptions::default())?;

    assert!(node_by_dom(&tree, "doc-child", 20)?.state().active());
    assert!(
        !node_by_dom(&tree, "doc-main", 20)?.state().active(),
        "iframe focus on DomNodeId(20) must not leak to main doc DomNodeId(20)"
    );
    let focused_button = node_by_dom(&tree, "doc-main", 50)?;
    assert!(focused_button.state().active());
    let children: Vec<&TreeNode> = tree.children(focused_button.id()).collect();
    assert_eq!(children.len(), 2);
    assert!(!children[0].state().active());
    assert!(!children[1].state().active());

    let body = node_by_dom(&tree, "doc-main", 60)?;
    let dialog = node_by_dom(&tree, "doc-main", 61)?;
    assert!(body.state().inert() && body.hidden().is_inert());
    assert!(
        body.hidden().receives_pointer_events(),
        "inert alone must never set pointer_events_none"
    );
    assert!(!dialog.state().inert() && !dialog.hidden().is_inert());

    assert_eq!(Role::ALL.iter().filter(|r| r.supports_checked()).count(), 7);
    assert_eq!(Role::ALL.iter().filter(|r| r.supports_pressed()).count(), 1);
    assert_eq!(
        Role::ALL.iter().filter(|r| r.supports_expanded()).count(),
        16
    );
    assert_eq!(
        Role::ALL.iter().filter(|r| r.supports_selected()).count(),
        7
    );
    assert_eq!(Role::ALL.iter().filter(|r| r.supports_level()).count(), 4);
    assert_eq!(
        Role::ALL.iter().filter(|r| r.supports_disabled()).count(),
        33
    );
    assert_eq!(
        Role::ALL.iter().filter(|r| r.supports_invalid()).count(),
        14
    );
    assert_eq!(
        Role::ALL.iter().filter(|r| r.supports_readonly()).count(),
        14
    );

    Ok(())
}

#[test]
fn hidden_rules_cover_inheritance_visibility_and_ua_shadow() -> Result<(), Box<dyn Error>> {
    let iframe = FrameBuilder::new("doc-vis-iframe").root(
        NodeBuilder::root()
            .dom_id(1)
            .child(boxed_element("button").dom_id(2)),
    );

    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            boxed_element("div")
                .dom_id(10)
                .aria_hidden(true)
                .child(boxed_element("span").dom_id(11).aria_hidden(true))
                .child(
                    NodeBuilder::text("secret")
                        .bounding_box(0, 0, 40, 14)
                        .dom_id(12),
                )
                .child(NodeBuilder::iframe(iframe).tag("iframe").dom_id(13)),
        )
        .child(
            boxed_element("div")
                .dom_id(20)
                .visibility_hidden(true)
                .child(boxed_element("button").dom_id(21)),
        )
        .child(
            boxed_element("details").dom_id(30).child(
                NodeBuilder::container()
                    .dom_id(31)
                    .ua_shadow(true)
                    .annotated_role(AnnotatedRole::ContentHidden)
                    .bounding_box(0, 0, 100, 50)
                    .child(boxed_element("p").dom_id(32)),
            ),
        );

    let tree = build_tree(root)?;
    let aria_parent = node_by_dom(&tree, "doc-main", 10)?.hidden();
    let aria_child = node_by_dom(&tree, "doc-main", 11)?.hidden();
    let aria_text = node_by_dom(&tree, "doc-main", 12)?.hidden();
    let iframe_btn = node_by_dom(&tree, "doc-vis-iframe", 2)?.hidden();
    assert!(aria_parent.is_aria_hidden() && aria_parent.has_own_aria_hidden());
    assert!(aria_parent.is_box_visible() && aria_parent.is_hidden_for_aria());
    assert!(aria_child.is_aria_hidden() && !aria_child.has_own_aria_hidden());
    assert!(
        aria_text.is_aria_hidden() && aria_text.is_hidden_for_aria() && aria_text.is_box_visible()
    );
    assert!(
        !iframe_btn.is_aria_hidden() && !iframe_btn.is_hidden_for_aria(),
        "aria-hidden must stop at iframe document boundaries"
    );

    let vis_parent = node_by_dom(&tree, "doc-main", 20)?.hidden();
    let vis_child = node_by_dom(&tree, "doc-main", 21)?.hidden();
    assert!(
        vis_parent.is_visibility_hidden()
            && !vis_parent.is_box_visible()
            && vis_parent.is_hidden_for_aria()
    );
    assert!(
        !vis_child.is_visibility_hidden()
            && vis_child.is_box_visible()
            && !vis_child.is_hidden_for_aria()
    );

    let slot_node = node_by_dom(&tree, "doc-main", 31)?;
    let details_p = node_by_dom(&tree, "doc-main", 32)?;
    assert!(slot_node.hidden().is_ua_shadow());
    assert!(slot_node.hidden().has_own_content_hidden());
    assert!(slot_node.hidden().is_content_hidden());
    assert!(slot_node.hidden().is_hidden_for_aria());
    assert!(!slot_node.hidden().is_box_visible());
    assert_eq!(slot_node.role(), None);
    assert_eq!(slot_node.state(), State::default());
    assert!(
        !details_p.hidden().is_ua_shadow(),
        "ua_shadow must not inherit to page content beneath the slot"
    );
    assert!(
        details_p.hidden().is_content_hidden() && !details_p.hidden().has_own_content_hidden(),
        "ContentHidden must inherit from ua_shadow slot onto page descendants"
    );
    assert!(details_p.hidden().is_hidden_for_aria() && !details_p.hidden().is_box_visible());

    let debug_dump = format!(
        "{:?} {:?} {:?} {:?}",
        aria_parent,
        Role::Button,
        State::default(),
        node_by_dom(&tree, "doc-main", 12)?
    );
    assert!(
        !debug_dump.contains("secret"),
        "Debug formatting must never expose page text: {debug_dump}"
    );
    assert!(
        Hidden::default().receives_pointer_events(),
        "default Hidden must receive pointer events"
    );

    Ok(())
}

#[test]
fn hidden_rules_cover_display_contents() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("div")
                .dom_id(40)
                .display_contents(true)
                .child(boxed_element("span").dom_id(41)),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(42)
                .display_contents(true)
                .child(
                    NodeBuilder::form_control(FormControlType::InputCheckbox)
                        .tag("input")
                        .bounding_box(0, 0, 0, 0)
                        .dom_id(43),
                ),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(44)
                .display_contents(true)
                .child(
                    NodeBuilder::text("boxed")
                        .bounding_box(0, 0, 30, 12)
                        .dom_id(45),
                ),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(46)
                .display_contents(true)
                .child(NodeBuilder::text("unboxed").dom_id(47)),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(48)
                .display_contents(true),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(49)
                .display_contents(true)
                .child(boxed_element("span").dom_id(50).visibility_hidden(true)),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(51)
                .display_contents(true)
                .child(boxed_element("span").dom_id(52).aria_hidden(true)),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(53)
                .display_contents(true)
                .aria_hidden(true)
                .child(boxed_element("span").dom_id(54)),
        );

    let tree = build_tree(root)?;

    let dc_vis = node_by_dom(&tree, "doc-main", 40)?.hidden();
    assert!(
        dc_vis.is_display_contents() && dc_vis.is_box_visible() && !dc_vis.is_hidden_for_aria()
    );

    let dc_zero = node_by_dom(&tree, "doc-main", 42)?.hidden();
    let zero_cb = node_by_dom(&tree, "doc-main", 43)?.hidden();
    assert!(!zero_cb.is_box_visible() && !zero_cb.is_hidden_for_aria());
    assert!(
        !dc_zero.is_box_visible() && !dc_zero.is_hidden_for_aria(),
        "display: contents wrapping 0x0 ARIA-visible control must be box-invisible yet ARIA-visible"
    );

    let dc_boxed_text = node_by_dom(&tree, "doc-main", 44)?.hidden();
    assert!(dc_boxed_text.is_box_visible() && !dc_boxed_text.is_hidden_for_aria());
    let dc_unboxed_text = node_by_dom(&tree, "doc-main", 46)?.hidden();
    assert!(!dc_unboxed_text.is_box_visible() && dc_unboxed_text.is_hidden_for_aria());
    let dc_empty = node_by_dom(&tree, "doc-main", 48)?.hidden();
    assert!(!dc_empty.is_box_visible() && dc_empty.is_hidden_for_aria());
    let dc_vis_hidden = node_by_dom(&tree, "doc-main", 49)?.hidden();
    assert!(!dc_vis_hidden.is_box_visible() && dc_vis_hidden.is_hidden_for_aria());
    let dc_aria_child = node_by_dom(&tree, "doc-main", 51)?.hidden();
    assert!(
        dc_aria_child.is_box_visible() && dc_aria_child.is_hidden_for_aria(),
        "display: contents wrapping boxed aria-hidden child must be box-visible yet ARIA-hidden"
    );
    let dc_aria_self = node_by_dom(&tree, "doc-main", 53)?.hidden();
    assert!(dc_aria_self.is_box_visible() && dc_aria_self.is_hidden_for_aria());

    Ok(())
}

#[test]
fn hidden_rules_cover_display_contents_through_ua_shadow_and_direct_text()
-> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            NodeBuilder::element("div")
                .dom_id(55)
                .display_contents(true)
                .child(
                    NodeBuilder::element("div")
                        .dom_id(56)
                        .display_contents(true)
                        .child(boxed_element("button").dom_id(57)),
                ),
        )
        .child(
            NodeBuilder::element("details")
                .dom_id(80)
                .display_contents(true)
                .child(
                    NodeBuilder::container().ua_shadow(true).dom_id(81).child(
                        NodeBuilder::container()
                            .ua_shadow(true)
                            .dom_id(82)
                            .child(boxed_element("summary").dom_id(83)),
                    ),
                ),
        )
        .child(
            NodeBuilder::element("details")
                .dom_id(84)
                .display_contents(true)
                .child(
                    NodeBuilder::container()
                        .ua_shadow(true)
                        .annotated_role(AnnotatedRole::ContentHidden)
                        .dom_id(85)
                        .child(boxed_element("p").dom_id(86)),
                ),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(87)
                .role("button")
                .display_contents(true)
                .aria_hidden(true)
                .child(
                    NodeBuilder::text("boxed aria-hidden text")
                        .bounding_box(0, 0, 50, 12)
                        .dom_id(88),
                ),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(89)
                .role("button")
                .display_contents(true)
                .annotated_role(AnnotatedRole::ContentHidden)
                .child(
                    NodeBuilder::text("content-hidden text")
                        .bounding_box(0, 0, 50, 12)
                        .dom_id(90),
                ),
        );

    let tree = build_tree(root)?;

    let dc_nested_outer = node_by_dom(&tree, "doc-main", 55)?.hidden();
    assert!(
        dc_nested_outer.is_box_visible() && !dc_nested_outer.is_hidden_for_aria(),
        "nested display: contents wrappers must aggregate visibility from their leaf button"
    );

    let dc_open_details = node_by_dom(&tree, "doc-main", 80)?.hidden();
    assert!(
        dc_open_details.is_box_visible() && !dc_open_details.is_hidden_for_aria(),
        "display: contents details host must aggregate visibility from page children slotted through ua_shadow"
    );

    let dc_closed_details = node_by_dom(&tree, "doc-main", 84)?.hidden();
    assert!(
        !dc_closed_details.is_box_visible() && dc_closed_details.is_hidden_for_aria(),
        "display: contents details host with only ContentHidden ua_shadow descendants must remain invisible and ARIA-hidden"
    );

    let dc_aria_text = node_by_dom(&tree, "doc-main", 87)?.hidden();
    assert!(
        dc_aria_text.is_aria_hidden()
            && dc_aria_text.is_box_visible()
            && !dc_aria_text.is_hidden_for_aria(),
        "display: contents with boxed direct text must follow Playwright's early text-box rule while retaining is_aria_hidden()"
    );

    let dc_content_hidden_text = node_by_dom(&tree, "doc-main", 89)?.hidden();
    assert!(
        dc_content_hidden_text.is_content_hidden()
            && !dc_content_hidden_text.is_box_visible()
            && dc_content_hidden_text.is_hidden_for_aria(),
        "display: contents under ContentHidden has no boxed text and must be invisible and ARIA-hidden"
    );

    Ok(())
}

#[test]
fn hidden_rules_cover_element_exceptions_and_pointer_events() -> Result<(), Box<dyn Error>> {
    let root = NodeBuilder::root()
        .dom_id(1)
        .child(
            boxed_element("select").dom_id(60).child(
                boxed_element("optgroup")
                    .dom_id(61)
                    .child(boxed_element("option").dom_id(62).visibility_hidden(true))
                    .child(boxed_element("option").dom_id(63).aria_hidden(true)),
            ),
        )
        .child(
            boxed_element("datalist")
                .dom_id(64)
                .child(boxed_element("option").dom_id(65).visibility_hidden(true)),
        )
        .child(
            NodeBuilder::element("slot")
                .dom_id(66)
                .display_contents(true)
                .visibility_hidden(true),
        )
        .child(boxed_element("style").dom_id(67))
        .child(boxed_element("script").dom_id(68))
        .child(boxed_element("noscript").dom_id(69))
        .child(boxed_element("template").dom_id(70))
        .child(boxed_element("button").dom_id(71).pointer_events_none(true));

    let tree = build_tree(root)?;
    assert!(
        !node_by_dom(&tree, "doc-main", 62)?
            .hidden()
            .is_hidden_for_aria()
    );
    assert!(
        node_by_dom(&tree, "doc-main", 63)?
            .hidden()
            .is_hidden_for_aria()
    );
    assert!(
        node_by_dom(&tree, "doc-main", 65)?
            .hidden()
            .is_hidden_for_aria()
    );
    let slot_el = node_by_dom(&tree, "doc-main", 66)?.hidden();
    assert!(!slot_el.is_hidden_for_aria() && !slot_el.is_box_visible());
    for dom_id in 67..=70 {
        assert!(
            node_by_dom(&tree, "doc-main", dom_id)?
                .hidden()
                .is_hidden_for_aria(),
            "ignored element dom_id={dom_id} (style/script/noscript/template) must be ARIA-hidden"
        );
    }
    let no_ptr = node_by_dom(&tree, "doc-main", 71)?.hidden();
    assert!(
        no_ptr.is_pointer_events_none() && !no_ptr.receives_pointer_events() && !no_ptr.is_inert()
    );

    Ok(())
}

fn build_deep_display_contents_chain(depth: usize) -> AnnotatedPageContent {
    let mut current: Option<ContentNode> = None;
    for index in (1..=depth).rev() {
        let mut children_nodes = Vec::new();
        if let Some(child) = current.take() {
            children_nodes.push(child);
        }
        let attrs = if index == depth {
            ContentAttributes {
                common_ancestor_dom_node_id: i32::try_from(index).ok(),
                attribute_type: Some(i32::from(ContentAttributeType::Container)),
                geometry: Some(Geometry {
                    outer_bounding_box: Some(BoundingRect {
                        x: Some(0),
                        y: Some(0),
                        width: Some(40),
                        height: Some(20),
                        is_screenshot_relative: Some(false),
                    }),
                    ..Geometry::default()
                }),
                browseros: Some(Box::new(BrowserOsContentAttributes {
                    tag: Some("button".to_owned()),
                    ..BrowserOsContentAttributes::default()
                })),
                ..ContentAttributes::default()
            }
        } else {
            ContentAttributes {
                common_ancestor_dom_node_id: i32::try_from(index).ok(),
                attribute_type: Some(i32::from(if index == 1 {
                    ContentAttributeType::Root
                } else {
                    ContentAttributeType::Container
                })),
                browseros: Some(Box::new(BrowserOsContentAttributes {
                    display_contents: Some(true),
                    aria_disabled: (index == 1).then_some(true),
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
                serialized_token: Some("doc-deep-dc".to_owned()),
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

struct DeepChainSummary {
    root_box_visible: bool,
    root_hidden_for_aria: bool,
    leaf_role: Option<Role>,
    leaf_disabled: Option<bool>,
}

#[test]
fn deep_100k_display_contents_chain_builds_without_stack_overflow() -> Result<(), Box<dyn Error>> {
    const DEPTH: usize = 100_000;

    let handle = thread::Builder::new()
        .name("deep-roles-states".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| -> Result<DeepChainSummary, String> {
            let apc = build_deep_display_contents_chain(DEPTH);
            let tree = SnapshotTree::build(apc, BuildOptions::default())
                .map_err(|err| format!("build failed: {err}"))?;
            let root = tree
                .nodes()
                .first()
                .ok_or_else(|| "missing root".to_owned())?;
            let leaf = tree
                .nodes()
                .last()
                .ok_or_else(|| "missing leaf".to_owned())?;
            Ok(DeepChainSummary {
                root_box_visible: root.hidden().is_box_visible(),
                root_hidden_for_aria: root.hidden().is_hidden_for_aria(),
                leaf_role: leaf.role(),
                leaf_disabled: leaf.state().disabled(),
            })
        })?;

    let summary = handle
        .join()
        .map_err(|payload| format!("deep-roles-states thread panicked: {payload:?}"))??;
    assert!(
        summary.root_box_visible,
        "bottom-up display: contents visibility must propagate across 100k levels"
    );
    assert!(
        !summary.root_hidden_for_aria,
        "bottom-up display: contents ARIA visibility must propagate across 100k levels"
    );
    assert_eq!(
        summary.leaf_role,
        Some(Role::Button),
        "deepest button at 100k depth must retain Role::Button"
    );
    assert_eq!(
        summary.leaf_disabled,
        Some(true),
        "top-down aria-disabled must propagate across 100k levels"
    );

    Ok(())
}

fn extract_ts_set(source: &str, const_name: &str) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let marker = format!("const {const_name}:");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("missing {marker} in stand-in roleUtils.ts"))?;
    let after_marker = source.get(start..).ok_or("slice past marker failed")?;
    let open = after_marker
        .find("new Set([")
        .ok_or_else(|| format!("missing new Set([ for {const_name}"))?;
    let rest = after_marker
        .get(open + "new Set([".len()..)
        .ok_or("slice after Set open failed")?;
    let close = rest
        .find("])")
        .ok_or_else(|| format!("missing ]) closing {const_name}"))?;
    let body = rest.get(..close).ok_or("slice Set body failed")?;

    let mut values = BTreeSet::new();
    for segment in body.split('\'').skip(1).step_by(2) {
        values.insert(segment.to_owned());
    }
    Ok(values)
}

fn extract_ts_block<'a>(source: &'a str, const_name: &str) -> Result<&'a str, Box<dyn Error>> {
    let marker = format!("const {const_name}:");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("missing {marker} in stand-in roleUtils.ts"))?;
    let after_marker = source.get(start..).ok_or("slice past block marker")?;
    let assign = after_marker
        .find("= {")
        .ok_or_else(|| format!("missing = {{ for {const_name}"))?;
    let rest = after_marker
        .get(assign + "= {".len()..)
        .ok_or("slice block body start")?;
    let close = rest
        .find("\n}")
        .ok_or_else(|| format!("missing closing brace for {const_name}"))?;
    rest.get(..close).ok_or_else(|| "slice block body".into())
}

fn extract_ts_record(
    source: &str,
    const_name: &str,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut map = BTreeMap::new();
    for line in extract_ts_block(source, const_name)?.lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("malformed record entry in {const_name}: {trimmed}"))?;
        map.insert(
            key.trim().trim_matches('\'').to_ascii_lowercase(),
            value.trim().trim_matches('\'').to_owned(),
        );
    }
    Ok(map)
}

fn extract_ts_parent_map(
    source: &str,
    const_name: &str,
) -> Result<BTreeMap<String, BTreeSet<String>>, Box<dyn Error>> {
    let mut map = BTreeMap::new();
    for line in extract_ts_block(source, const_name)?.lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let (key, rhs) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("malformed entry in {const_name}: {trimmed}"))?;
        let parents: BTreeSet<String> = rhs
            .split('\'')
            .skip(1)
            .step_by(2)
            .map(str::to_ascii_lowercase)
            .collect();
        map.insert(key.trim().trim_matches('\'').to_ascii_lowercase(), parents);
    }
    Ok(map)
}

#[test]
fn rust_role_tables_match_standin_role_utils_verbatim() -> Result<(), Box<dyn Error>> {
    let stub_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../neo-playwright-js/src/stubs/roleUtils.ts");
    let source = fs::read_to_string(&stub_path)?;

    let rust_valid: BTreeSet<String> = Role::ALL
        .iter()
        .map(|role| role.as_str().to_owned())
        .collect();
    assert_eq!(
        rust_valid,
        extract_ts_set(&source, "VALID_ROLES")?,
        "Role::ALL must match stand-in VALID_ROLES verbatim"
    );

    let rust_checked: BTreeSet<String> = Role::ALL
        .iter()
        .copied()
        .filter(|role| role.supports_checked())
        .map(|role| role.as_str().to_owned())
        .collect();
    assert_eq!(
        rust_checked,
        extract_ts_set(&source, "ARIA_CHECKED_ROLES")?,
        "Role::supports_checked must match stand-in ARIA_CHECKED_ROLES verbatim"
    );

    let mut stub_disabled = extract_ts_set(&source, "ARIA_DISABLED_ROLES")?;
    for abstract_role in ["composite", "input", "select"] {
        assert!(
            stub_disabled.remove(abstract_role),
            "expected abstract role {abstract_role:?} in stand-in ARIA_DISABLED_ROLES"
        );
    }
    let rust_disabled: BTreeSet<String> = Role::ALL
        .iter()
        .copied()
        .filter(|role| role.supports_disabled())
        .map(|role| role.as_str().to_owned())
        .collect();
    assert_eq!(
        rust_disabled, stub_disabled,
        "Role::supports_disabled must match stand-in ARIA_DISABLED_ROLES concrete roles verbatim"
    );

    let rust_readonly: BTreeSet<String> = Role::ALL
        .iter()
        .copied()
        .filter(|role| role.supports_readonly())
        .map(|role| role.as_str().to_owned())
        .collect();
    assert_eq!(
        rust_readonly,
        extract_ts_set(&source, "ARIA_READONLY_ROLES")?,
        "Role::supports_readonly must match stand-in ARIA_READONLY_ROLES verbatim"
    );

    let stub_static = extract_ts_record(&source, "STATIC_IMPLICIT_ROLES")?;
    let mut root = NodeBuilder::root().dom_id(1);
    let mut expected_by_dom = Vec::new();
    for (offset, (tag, role_name)) in stub_static.iter().enumerate() {
        let dom_id = i32::try_from(offset + 10)?;
        let expected_role = Role::parse(role_name)
            .ok_or_else(|| format!("unknown stand-in role {role_name:?} for <{tag}>"))?;
        root = root.child(NodeBuilder::element(tag).dom_id(dom_id));
        expected_by_dom.push((dom_id, tag.clone(), expected_role));
    }
    let static_tree = build_tree(root)?;
    for (dom_id, tag, expected_role) in expected_by_dom {
        assert_eq!(
            node_by_dom(&static_tree, "doc-main", dom_id)?.role(),
            Some(expected_role),
            "Rust static implicit role for <{tag}> must match stand-in STATIC_IMPLICIT_ROLES"
        );
    }

    let stub_inheritance = extract_ts_parent_map(&source, "PRESENTATION_INHERITANCE_PARENTS")?;
    assert_eq!(stub_inheritance.len(), 10);
    for (child_tag, parent_tags) in &stub_inheritance {
        for parent_tag in parent_tags {
            let child_node = if child_tag == "div" {
                NodeBuilder::container()
                    .dom_id(20)
                    .child(NodeBuilder::element("dt").dom_id(30))
            } else {
                NodeBuilder::element(child_tag).dom_id(30)
            };
            let parent_node = NodeBuilder::element(parent_tag)
                .dom_id(10)
                .role("presentation")
                .child(child_node);
            let tree = build_tree(NodeBuilder::root().dom_id(1).child(parent_node))?;
            assert_eq!(
                node_by_dom(&tree, "doc-main", 30)?.role(),
                Some(Role::Presentation),
                "stand-in presentation inheritance pair ({child_tag} <- {parent_tag}) must inherit Presentation in Rust"
            );
        }
    }

    Ok(())
}
