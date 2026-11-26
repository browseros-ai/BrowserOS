//! A collapsed `<select>`'s options as option slots: which selects get them, where they sit in the
//! arena, the facts Playwright's `<option>` rules give them, privacy, and the reverse join.

use std::error::Error;

use neo_snapshot::{AriaChild, AriaNode};
use neo_snapshot_apc::proto::{FormControlType, RedactionDecision, SelectOption};
use neo_snapshot_apc::tree::{
    ApcBuilder, AriaAsk, BuildOptions, DocToken, DomNodeId, ElementPlace, FrameBuilder, NameCache,
    NameVisibility, NodeBuilder, NodeId, NodeKey, OptionPlace, OptionSlot, PrivateNodes, Role,
    SiteKey, SnapshotTree, TreeNode, TriState,
};

const MAIN: &str = "doc-main";
const PRIVATE_ORIGIN: &str = "https://private.example";

/// What Chromium reads of an option besides its text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Marked {
    Plain,
    Selected,
    /// `IsDisabledFormControl()`: its own `disabled`, or its `<optgroup>`'s.
    Disabled,
}

fn entry(text: &str, marked: Marked) -> SelectOption {
    SelectOption {
        value: Some(format!("value-{text}")),
        text: Some(text.to_owned()),
        is_selected: Some(marked == Marked::Selected),
        is_disabled: Some(marked == Marked::Disabled),
    }
}

/// Each option slot's ARIA facts, one list per fact, in option order.
#[derive(Debug, PartialEq, Eq)]
struct OptionFacts {
    hidden: Vec<bool>,
    disabled: Vec<bool>,
    inert: Vec<bool>,
}

fn option_facts(tree: &SnapshotTree, select: NodeId) -> Result<OptionFacts, Box<dyn Error>> {
    let mut facts = OptionFacts {
        hidden: Vec::new(),
        disabled: Vec::new(),
        inert: Vec::new(),
    };
    for option in slots(tree, select) {
        let slot = node(tree, option)?;
        facts.hidden.push(slot.hidden().is_hidden_for_aria());
        facts.disabled.push(slot.state().is_disabled());
        facts.inert.push(slot.state().inert());
    }
    Ok(facts)
}

/// A collapsed `<select>` as Chromium sends it: the options as entries, the shown value as
/// browser-made text.
fn collapsed_select(dom: i32, entries: Vec<SelectOption>) -> NodeBuilder {
    entries
        .into_iter()
        .fold(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .tag("select")
                .dom_id(dom)
                .bounding_box(0, dom * 30, 160, 24),
            NodeBuilder::select_option,
        )
        .child(NodeBuilder::text("shown value").ua_shadow(true))
}

fn fruit(dom: i32) -> NodeBuilder {
    collapsed_select(
        dom,
        vec![
            entry("Apple", Marked::Plain),
            entry("Banana", Marked::Selected),
            entry("Cherry", Marked::Disabled),
        ],
    )
}

fn build(root: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
    let apc = ApcBuilder::new(MAIN).root(root).build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn key(doc: &str, dom: i32) -> Result<NodeKey, Box<dyn Error>> {
    let token = DocToken::new(doc).ok_or("a test document token is never empty")?;
    Ok(NodeKey::new(token, DomNodeId::new(dom)))
}

fn node_in(tree: &SnapshotTree, doc: &str, dom: i32) -> Result<NodeId, Box<dyn Error>> {
    Ok(tree
        .node_by_key(&key(doc, dom)?)
        .ok_or_else(|| format!("no node ({doc}, {dom})"))?
        .id())
}

/// The slots of `select`, in order: its children that are option slots.
fn slots(tree: &SnapshotTree, select: NodeId) -> Vec<NodeId> {
    tree.children(select)
        .map(TreeNode::id)
        .filter(|&child| tree.option_slot(child).is_some())
        .collect()
}

fn node(tree: &SnapshotTree, id: NodeId) -> Result<&TreeNode, Box<dyn Error>> {
    tree.node(id)
        .ok_or_else(|| format!("no node {id:?}").into())
}

fn name(names: &NameCache<'_>, id: NodeId) -> String {
    names
        .name(id, NameVisibility::VisibleOnly)
        .text()
        .to_owned()
}

fn printed_roles(nodes: &[AriaNode]) -> Vec<String> {
    let mut roles = Vec::new();
    let mut stack: Vec<&AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        roles.push(node.role.clone());
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child) = child {
                stack.push(child);
            }
        }
    }
    roles
}

#[test]
fn a_collapsed_select_lists_one_option_per_entry_as_its_last_children() -> Result<(), Box<dyn Error>>
{
    let tree = build(
        NodeBuilder::root()
            .dom_id(1)
            .children(vec![fruit(10), NodeBuilder::button("After").dom_id(20)]),
    )?;
    let select = node_in(&tree, MAIN, 10)?;
    let after = node_in(&tree, MAIN, 20)?;
    let options = slots(&tree, select);
    assert_eq!(options.len(), 3, "one option per select_options entry");

    let children: Vec<NodeId> = tree.children(select).map(TreeNode::id).collect();
    assert_eq!(
        children.get(1..),
        Some(options.as_slice()),
        "the options follow the select's own children, as its last children"
    );
    for (index, &option) in (0_u32..).zip(&options) {
        assert_eq!(
            tree.option_slot(option),
            Some(OptionSlot {
                select,
                index,
                count: 3
            }),
            "the page reaches option {index} as select.options[{index}] of a 3-option list"
        );
        let slot = node(&tree, option)?;
        assert_eq!(slot.role(), Some(Role::Option), "an <option> in a select");
        assert_eq!(slot.dom_node_id(), None, "APC sent no node for it");
        assert_eq!(
            tree.node_key(option),
            None,
            "so it has no key to hydrate by"
        );
        assert!(
            node(&tree, select)?.subtree_range().contains(option),
            "inside the select's subtree, so containment and scans from an ancestor reach it"
        );
        assert!(
            option < after,
            "in tree order, before the select's next sibling"
        );
    }
    assert_eq!(
        tree.node_by_key(&key(MAIN, 20)?).map(TreeNode::id),
        Some(after),
        "the keys of the nodes APC sent are untouched"
    );
    let main = tree.main_document().ok_or("no main document")?;
    assert_eq!(
        main.node_count(),
        tree.nodes().len(),
        "the options belong to the select's document"
    );

    let names = tree.name_cache();
    let read: Vec<(String, Option<bool>, bool, Option<TriState>)> = options
        .iter()
        .map(|&option| {
            let state = tree.node(option).map(TreeNode::state).unwrap_or_default();
            (
                name(&names, option),
                state.selected(),
                state.is_disabled(),
                state.checked(),
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            (
                "Apple".to_owned(),
                Some(false),
                false,
                Some(TriState::False)
            ),
            (
                "Banana".to_owned(),
                Some(true),
                false,
                Some(TriState::False)
            ),
            (
                "Cherry".to_owned(),
                Some(false),
                true,
                Some(TriState::False)
            ),
        ],
        "name from content, getAriaSelected = option.selected, the entry's disabled, and \
         getAriaChecked false with no aria-checked"
    );
    let printed = format!("{tree:?} {:?}", tree.nodes());
    for label in ["Apple", "Banana", "Cherry"] {
        assert!(
            !printed.contains(label),
            "Debug prints structure only, never an option's label"
        );
    }
    Ok(())
}

#[test]
fn only_a_select_whose_options_are_not_nodes_gets_option_slots() -> Result<(), Box<dyn Error>> {
    let frame_with_option = FrameBuilder::new("doc-inner").root(
        NodeBuilder::root()
            .dom_id(1)
            .child(NodeBuilder::element("option").dom_id(2)),
    );
    let frame_with_select =
        FrameBuilder::new("doc-child").root(NodeBuilder::root().dom_id(1).child(fruit(5)));
    let tree = build(NodeBuilder::root().dom_id(1).children(vec![
        // A list box sends its options as nodes.
        NodeBuilder::form_control(FormControlType::SelectMultiple)
            .tag("select")
            .dom_id(10)
            .list_box(true)
            .select_option(entry("Listed", Marked::Selected))
            .child(NodeBuilder::element("option").dom_id(11)),
        // So does any select whose subtree holds a real <option>, as through an <optgroup>.
        collapsed_select(20, vec![entry("Duplicate", Marked::Selected)]).child(
            NodeBuilder::element("optgroup")
                .dom_id(21)
                .child(NodeBuilder::element("option").dom_id(22)),
        ),
        collapsed_select(30, Vec::new()),
        // An <option> in an iframe's document belongs to another select.
        collapsed_select(40, vec![entry("Outer", Marked::Plain)])
            .child(NodeBuilder::iframe(frame_with_option).dom_id(41)),
        NodeBuilder::iframe(frame_with_select).dom_id(50),
    ]))?;
    assert!(
        slots(&tree, node_in(&tree, MAIN, 10)?).is_empty(),
        "a list box's options are its real nodes"
    );
    assert!(
        slots(&tree, node_in(&tree, MAIN, 20)?).is_empty(),
        "an option is never listed twice"
    );
    assert!(
        slots(&tree, node_in(&tree, MAIN, 30)?).is_empty(),
        "an empty list has no options"
    );
    assert_eq!(
        slots(&tree, node_in(&tree, MAIN, 40)?).len(),
        1,
        "an <option> in a nested document does not count for this select"
    );

    let child_select = node_in(&tree, "doc-child", 5)?;
    let child_doc = node(&tree, child_select)?.doc();
    let child_options = slots(&tree, child_select);
    assert_eq!(child_options.len(), 3, "a select in a child document");
    let range = tree
        .document(child_doc)
        .ok_or("no child document")?
        .node_range();
    for option in child_options {
        assert_eq!(
            node(&tree, option)?.doc(),
            child_doc,
            "in its select's document"
        );
        assert!(range.contains(option), "inside that document's range");
    }
    Ok(())
}

/// Playwright's rules for an `<option>` in a `<select>` (`roleUtils.ts`): hidden only by
/// `display:none` or `aria-hidden` on itself or an ancestor, never by visibility;
/// `getAriaDisabled` reads its own `disabled`, its group's, or an inherited `aria-disabled`, and
/// not the select's own `disabled`.
#[test]
fn an_option_takes_its_hidden_and_disabled_facts_by_playwrights_option_rules()
-> Result<(), Box<dyn Error>> {
    let tree = build(NodeBuilder::root().dom_id(1).children(vec![
        fruit(10).aria_hidden(true),
        fruit(20).visibility_hidden(true),
        fruit(30).aria_disabled(true),
        fruit(40).contract_disabled(true),
        fruit(50).inert(true),
    ]))?;
    let facts = |dom: i32| -> Result<OptionFacts, Box<dyn Error>> {
        option_facts(&tree, node_in(&tree, MAIN, dom)?)
    };
    // Cherry, the third option, is disabled by its own entry on every select.
    assert_eq!(
        facts(10)?,
        OptionFacts {
            hidden: vec![true, true, true],
            disabled: vec![false, false, true],
            inert: vec![false, false, false],
        },
        "aria-hidden on the select hides its options"
    );
    assert_eq!(
        facts(20)?.hidden,
        [false, false, false],
        "visibility never hides an option inside a select"
    );
    assert_eq!(
        facts(30)?.disabled,
        [true, true, true],
        "aria-disabled on an ancestor disables every option"
    );
    assert_eq!(
        facts(40)?.disabled,
        [false, false, true],
        "the select's own disabled is not an option's"
    );
    assert_eq!(
        facts(50)?.inert,
        [true, true, true],
        "an option's computed style is its select's, inert included"
    );
    Ok(())
}

#[test]
fn a_private_select_names_its_options_with_bullets() -> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new(MAIN)
        .main_origin(PRIVATE_ORIGIN, false)
        .root(NodeBuilder::root().dom_id(1).child(fruit(10)))
        .build();
    let mut private = PrivateNodes::new();
    private.mark(SiteKey::from_origin(PRIVATE_ORIGIN), DomNodeId::new(10));
    let tree = SnapshotTree::build(apc, BuildOptions::default().with_private_nodes(&private))?;
    let names = tree.name_cache();
    let read: Vec<String> = slots(&tree, node_in(&tree, MAIN, 10)?)
        .into_iter()
        .map(|option| name(&names, option))
        .collect();
    assert_eq!(
        read,
        ["•••••", "••••••", "••••••"],
        "a host-private select's option labels are withheld as one bullet per UTF-16 unit"
    );
    Ok(())
}

#[test]
fn a_select_whose_list_the_mask_cleared_has_no_options() -> Result<(), Box<dyn Error>> {
    let tree = build(
        NodeBuilder::root().dom_id(1).child(
            NodeBuilder::container()
                .dom_id(5)
                .bounding_box(0, 0, 200, 40)
                .redaction(RedactionDecision::RedactedIsSensitivePaymentField)
                .child(fruit(10)),
        ),
    )?;
    let select = node_in(&tree, MAIN, 10)?;
    assert!(
        slots(&tree, select).is_empty(),
        "no option of a cleared list has a place in its select"
    );
    assert!(
        tree.nodes()
            .iter()
            .all(|node| node.role() != Some(Role::Option)),
        "nothing with role option, so getByRole('option') matches none"
    );
    let printed = tree.aria(AriaAsk::default_mode())?;
    assert!(
        !printed_roles(printed.nodes())
            .iter()
            .any(|role| role == "option"),
        "and none prints"
    );
    let names = tree.name_cache();
    for node in tree.nodes() {
        let read = name(&names, node.id());
        assert!(
            !["Apple", "Banana", "Cherry"]
                .iter()
                .any(|label| read.contains(label)),
            "no name reveals a cleared label"
        );
    }
    let place = ElementPlace {
        key: key(MAIN, 900)?,
        option: Some(OptionPlace {
            select: key(MAIN, 10)?,
            index: 0,
            count: 3,
        }),
    };
    assert!(tree.node_at(&place).is_none(), "nor does a join find one");
    Ok(())
}

#[test]
fn a_pinned_option_finds_its_slot_by_its_select_and_index() -> Result<(), Box<dyn Error>> {
    let tree = build(NodeBuilder::root().dom_id(1).children(vec![
        fruit(10),
        NodeBuilder::form_control(FormControlType::SelectMultiple)
            .tag("select")
            .dom_id(20)
            .list_box(true)
            .child(NodeBuilder::element("option").dom_id(21)),
    ]))?;
    let select = node_in(&tree, MAIN, 10)?;
    let options = slots(&tree, select);
    // The page's <option> has a backendNodeId of its own that no capture node carries.
    let at = |index: u32, count: u32| -> Result<Option<NodeId>, Box<dyn Error>> {
        Ok(tree
            .node_at(&ElementPlace {
                key: key(MAIN, 900)?,
                option: Some(OptionPlace {
                    select: key(MAIN, 10)?,
                    index,
                    count,
                }),
            })
            .map(TreeNode::id))
    };
    assert_eq!(
        at(1, 3)?,
        options.get(1).copied(),
        "select.options[1] is slot 1"
    );
    assert_eq!(
        at(1, 4)?,
        None,
        "a list that grew since the capture may have moved another option to index 1"
    );
    assert_eq!(at(3, 3)?, None, "an index past the list names no option");

    let real_option = ElementPlace {
        key: key(MAIN, 21)?,
        option: Some(OptionPlace {
            select: key(MAIN, 20)?,
            index: 0,
            count: 1,
        }),
    };
    assert_eq!(
        tree.node_at(&real_option).map(TreeNode::id),
        Some(node_in(&tree, MAIN, 21)?),
        "a list box's real option joins by its own key"
    );
    let not_an_option = ElementPlace {
        key: key(MAIN, 10)?,
        option: None,
    };
    assert_eq!(
        tree.node_at(&not_an_option).map(TreeNode::id),
        Some(select),
        "any element APC sent joins by its key"
    );
    Ok(())
}
