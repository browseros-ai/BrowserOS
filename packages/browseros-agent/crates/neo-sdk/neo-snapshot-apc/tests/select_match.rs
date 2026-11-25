//! `TreeStage` through the public API over hand-built trees named by the name pass: Playwright
//! 1.64.1's `selectors-role.spec.ts` restated test by test, nested chains, documents, `aria-ref`,
//! the `NameCache` seam, and strict mode against recorded messages.

#[path = "support/prelude_chunks.rs"]
mod prelude_chunks;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;

use neo_snapshot_apc::proto::{AnnotatedRole, BrowserOsTriState, FormControlType};
use neo_snapshot_apc::select::{
    AccessibleText, NestedSelectorBody, ParsedSelector, RefResolver, Scope, SelectorBody,
    SelectorPart, StrictPreview, TreeMatches, TreeStage, TreeStageError,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocId, DocToken, DomNodeId, FrameBuilder, NameCache, NameVisibility,
    NodeBuilder, NodeId, NodeKey, SnapshotTree,
};
use prelude_chunks::Prelude;

const MAIN: &str = "doc-main";

/// Which seam method a stage called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Description,
}

/// One call a stage made to the name seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextRead {
    field: Field,
    node: NodeId,
    visibility: NameVisibility,
}

/// The name pass behind a recorder, for the test about the seam itself: each read is logged, then
/// answered by the [`NameCache`] impl that production passes, so the test sees what the stage
/// asked for, in which order and with which visibility.
struct RecordingText<'a> {
    names: NameCache<'a>,
    reads: RefCell<Vec<TextRead>>,
}

impl AccessibleText for RecordingText<'_> {
    fn name(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str> {
        self.reads.borrow_mut().push(TextRead {
            field: Field::Name,
            node,
            visibility,
        });
        AccessibleText::name(&self.names, node, visibility)
    }

    fn description(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str> {
        self.reads.borrow_mut().push(TextRead {
            field: Field::Description,
            node,
            visibility,
        });
        AccessibleText::description(&self.names, node, visibility)
    }
}

/// The tab's ref table, standing in for `TabRefTable` (engine chunk 5).
#[derive(Default)]
struct Refs(HashMap<String, NodeKey>);

impl RefResolver for Refs {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        self.0.get(body).cloned()
    }
}

/// A hand-built capture and the ref table the stage reads beside it. Names come from the tree's
/// own name pass.
struct Page {
    tree: SnapshotTree,
    refs: Refs,
}

impl Page {
    fn new(root: NodeBuilder) -> Result<Self, Box<dyn Error>> {
        let apc = ApcBuilder::new(MAIN).root(root).build();
        Ok(Self {
            tree: SnapshotTree::build(apc, BuildOptions::default())?,
            refs: Refs::default(),
        })
    }

    fn key(doc: &str, dom: i32) -> Result<NodeKey, Box<dyn Error>> {
        let token = DocToken::new(doc).ok_or("a test document token is never empty")?;
        Ok(NodeKey::new(token, DomNodeId::new(dom)))
    }

    fn node_in(&self, doc: &str, dom: i32) -> Result<NodeId, Box<dyn Error>> {
        Ok(self
            .tree
            .node_by_key(&Self::key(doc, dom)?)
            .ok_or_else(|| format!("no node ({doc}, {dom})"))?
            .id())
    }

    fn node(&self, dom: i32) -> Result<NodeId, Box<dyn Error>> {
        self.node_in(MAIN, dom)
    }

    fn nodes(&self, doms: &[i32]) -> Result<Vec<NodeId>, Box<dyn Error>> {
        doms.iter().map(|&dom| self.node(dom)).collect()
    }

    /// The `index`th child of the main-document node `parent`: how a test finds the option
    /// nodes chunk 5 synthesizes, which have no DOM id.
    fn child(&self, parent: i32, index: usize) -> Result<NodeId, Box<dyn Error>> {
        Ok(self
            .tree
            .children(self.node(parent)?)
            .nth(index)
            .ok_or_else(|| format!("node {parent} has no child {index}"))?
            .id())
    }

    fn document(&self) -> Result<Scope<'static>, Box<dyn Error>> {
        Scope::document(&self.tree, DocId::MAIN).ok_or_else(|| "no main document root".into())
    }

    fn root(&self) -> Result<NodeId, Box<dyn Error>> {
        match self.document()? {
            Scope::Root(root) => Ok(root),
            Scope::After { .. } => Err("a document scope is a root".into()),
        }
    }

    fn try_select_in(
        &self,
        js: &Prelude,
        locator: &str,
        scope: Scope<'_>,
    ) -> Result<Result<TreeMatches, TreeStageError>, Box<dyn Error>> {
        let stage = TreeStage::new(&js.chunk(locator)?)?;
        Ok(stage.select(&self.tree, scope, &self.tree.name_cache(), &self.refs))
    }

    fn try_select(
        &self,
        js: &Prelude,
        locator: &str,
    ) -> Result<Result<TreeMatches, TreeStageError>, Box<dyn Error>> {
        self.try_select_in(js, locator, self.document()?)
    }

    fn select(&self, js: &Prelude, locator: &str) -> Result<TreeMatches, Box<dyn Error>> {
        Ok(self.try_select(js, locator)??)
    }

    /// Asserts that `locator`, run over the main document, selects exactly `expected` in order.
    fn check(
        &self,
        js: &Prelude,
        locator: &str,
        expected: &[NodeId],
        why: &str,
    ) -> Result<(), Box<dyn Error>> {
        let matches = self.select(js, locator)?;
        assert_eq!(matches.nodes(), expected, "{locator}: {why}");
        Ok(())
    }
}

fn el(tag: &str, dom: i32) -> NodeBuilder {
    NodeBuilder::element(tag).dom_id(dom)
}

/// A `div` or `span`: APC writes no `tag` for either, so each is a plain container.
fn plain(dom: i32) -> NodeBuilder {
    NodeBuilder::container().dom_id(dom)
}

fn button(dom: i32, label: &str) -> NodeBuilder {
    el("button", dom).child(NodeBuilder::text(label))
}

fn input(dom: i32, control: FormControlType) -> NodeBuilder {
    NodeBuilder::form_control(control).tag("input").dom_id(dom)
}

/// An `<option>` of a collapsed `<select>` as engine chunk 5 will synthesize it from the select's
/// `select_options`: a node of its own with no DOM id (design 3.5).
fn synthesized_option() -> NodeBuilder {
    NodeBuilder::element("option").without_dom_id()
}

/// A slot of browser-made shadow content, such as a `<details>` element's content slot.
fn ua_slot() -> NodeBuilder {
    NodeBuilder::container().ua_shadow(true)
}

/// `page.locator(<selector>)`, with the selector quoted as a JavaScript string.
fn locator(selector: &str) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "page.locator({})",
        serde_json::to_string(selector)?
    ))
}

/// `selectors-role.spec.ts`, one test per spec test. Each page is built from the facts Chromium
/// sends for the spec's markup, names included, and each expected list is the spec's, by node.
mod selectors_role_spec {
    use super::*;

    #[test]
    fn should_detect_roles() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            button(10, "Hello"),
            // `<select multiple size=2>` is a list box (contract field 6).
            el("select", 11).list_box(true),
            el("select", 12),
            el("h3", 13).child(NodeBuilder::text("Heading")),
            el("details", 14).child(el("summary", 15).child(NodeBuilder::text("Hello"))),
            plain(16)
                .role("dialog")
                .child(NodeBuilder::text("I am a dialog")),
        ]))?;
        let cases: [(&str, &[i32]); 8] = [
            ("page.locator(`role=button`)", &[10]),
            ("page.locator(`role=listbox`)", &[11]),
            ("page.locator(`role=combobox`)", &[12]),
            ("page.locator(`role=heading`)", &[13]),
            ("page.locator(`role=group`)", &[14]),
            ("page.locator(`role=dialog`)", &[16]),
            ("page.locator(`role=menuitem`)", &[]),
            ("page.getByRole('menuitem')", &[]),
        ];
        for (locator, expected) in cases {
            page.check(
                &js,
                locator,
                &page.nodes(expected)?,
                "each element has exactly its implicit or explicit role",
            )?;
        }
        Ok(())
    }

    #[test]
    fn should_support_selected() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            el("select", 10).children(vec![
                synthesized_option().selected(false),
                synthesized_option().selected(true),
            ]),
            plain(13).children(vec![
                plain(14).role("option").selected(true),
                plain(15).role("option").selected(false),
            ]),
        ]))?;
        let selected = vec![page.child(10, 1)?, page.node(14)?];
        let unselected = vec![page.child(10, 0)?, page.node(15)?];
        for locator in [
            "page.locator(`role=option[selected]`)",
            "page.locator(`role=option[selected=true]`)",
            "page.getByRole('option', { selected: true })",
        ] {
            page.check(
                &js,
                locator,
                &selected,
                "the selected <option> and the aria-selected div, in tree order",
            )?;
        }
        for locator in [
            "page.locator(`role=option[selected=false]`)",
            "page.getByRole('option', { selected: false })",
        ] {
            page.check(
                &js,
                locator,
                &unselected,
                "the unselected <option> and the aria-selected=false div",
            )?;
        }
        Ok(())
    }

    #[test]
    fn should_support_checked() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            input(10, FormControlType::InputCheckbox).checked_state(BrowserOsTriState::False),
            input(11, FormControlType::InputCheckbox).checked_state(BrowserOsTriState::True),
            // The spec sets `indeterminate` from script; Chromium reports it as mixed.
            input(12, FormControlType::InputCheckbox).checked_state(BrowserOsTriState::Mixed),
            plain(13)
                .role("checkbox")
                .checked_state(BrowserOsTriState::True),
            plain(14)
                .role("checkbox")
                .checked_state(BrowserOsTriState::False),
            // No `aria-checked`, so no checked fact.
            plain(15).role("checkbox"),
        ]))?;
        for locator in [
            "page.locator(`role=checkbox[checked]`)",
            "page.locator(`role=checkbox[checked=true]`)",
            "page.getByRole('checkbox', { checked: true })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[11, 13])?,
                "the checked input and the aria-checked=true div",
            )?;
        }
        for locator in [
            "page.locator(`role=checkbox[checked=false]`)",
            "page.getByRole('checkbox', { checked: false })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[10, 14, 15])?,
                "a checkbox role with no aria-checked counts as unchecked",
            )?;
        }
        page.check(
            &js,
            r#"page.locator(`role=checkbox[checked="mixed"]`)"#,
            &page.nodes(&[12])?,
            "only the indeterminate input is mixed",
        )?;
        page.check(
            &js,
            "page.locator(`role=checkbox`)",
            &page.nodes(&[10, 11, 12, 13, 14, 15])?,
            "with no state filter every checkbox matches",
        )
    }

    #[test]
    fn should_support_pressed() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            button(10, "Hi"),
            button(11, "Hello").pressed_state(BrowserOsTriState::True),
            button(12, "Bye").pressed_state(BrowserOsTriState::False),
            button(13, "Mixed").pressed_state(BrowserOsTriState::Mixed),
        ]))?;
        for locator in [
            "page.locator(`role=button[pressed]`)",
            "page.locator(`role=button[pressed=true]`)",
            "page.getByRole('button', { pressed: true })",
        ] {
            page.check(&js, locator, &page.nodes(&[11])?, "only aria-pressed=true")?;
        }
        for locator in [
            "page.locator(`role=button[pressed=false]`)",
            "page.getByRole('button', { pressed: false })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[10, 12])?,
                "a button with no aria-pressed counts as not pressed",
            )?;
        }
        page.check(
            &js,
            r#"page.locator(`role=button[pressed="mixed"]`)"#,
            &page.nodes(&[13])?,
            "only aria-pressed=mixed",
        )?;
        page.check(
            &js,
            "page.locator(`role=button`)",
            &page.nodes(&[10, 11, 12, 13])?,
            "with no state filter every button matches",
        )
    }

    #[test]
    fn should_support_expanded() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            plain(10).role("treeitem"),
            plain(11)
                .role("treeitem")
                .expanded_state(BrowserOsTriState::True),
            plain(12)
                .role("treeitem")
                .expanded_state(BrowserOsTriState::False),
        ]))?;
        for locator in [
            "page.locator('role=treeitem')",
            "page.getByRole('treeitem')",
        ] {
            page.check(&js, locator, &page.nodes(&[10, 11, 12])?, "every treeitem")?;
        }
        for locator in [
            "page.locator(`role=treeitem[expanded]`)",
            "page.locator(`role=treeitem[expanded=true]`)",
            "page.getByRole('treeitem', { expanded: true })",
        ] {
            page.check(&js, locator, &page.nodes(&[11])?, "only aria-expanded=true")?;
        }
        for locator in [
            "page.locator(`role=treeitem[expanded=false]`)",
            "page.getByRole('treeitem', { expanded: false })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[12])?,
                "an unset aria-expanded is neither true nor false",
            )?;
        }
        // The spec's workaround for `expanded="none"` is a CSS selector, which only the page
        // evaluates.
        let css = js.chunk("page.locator(`[role=treeitem]:not([aria-expanded])`)")?;
        let refused = TreeStage::new(&css)
            .err()
            .ok_or("a CSS chunk is not a tree stage")?;
        assert_eq!(refused.part(), "css", "planning names the CSS part");
        Ok(())
    }

    #[test]
    fn should_support_disabled() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            button(10, "Hi"),
            button(11, "Bye").contract_disabled(true),
            button(12, "Hello").aria_disabled(true),
            button(13, "Oh").aria_disabled(false),
            // Chromium marks a control inside `<fieldset disabled>` disabled itself.
            el("fieldset", 14)
                .contract_disabled(true)
                .child(button(15, "Yay").contract_disabled(true)),
            el("select", 16).children(vec![
                // `<optgroup disabled><option>one</option></optgroup>`: Chromium marks the
                // option disabled through its group, and chunk 5 carries that fact over.
                synthesized_option().contract_disabled(true),
                synthesized_option(),
                synthesized_option().contract_disabled(true),
            ]),
        ]))?;
        let (one, two, three) = (page.child(16, 0)?, page.child(16, 1)?, page.child(16, 2)?);
        for locator in [
            "page.locator(`role=button[disabled]`)",
            "page.locator(`role=button[disabled=true]`)",
            "page.getByRole('button', { disabled: true })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[11, 12, 15])?,
                "native disabled, aria-disabled=true, and inside a disabled fieldset",
            )?;
        }
        for locator in [
            "page.locator(`role=button[disabled=false]`)",
            "page.getByRole('button', { disabled: false })",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[10, 13])?,
                "aria-disabled=false is enabled",
            )?;
        }
        page.check(
            &js,
            "page.getByRole('option', { disabled: true })",
            &[one, three],
            "an option is disabled by its own attribute or its disabled optgroup",
        )?;
        page.check(
            &js,
            "page.getByRole('option', { disabled: false })",
            &[two],
            "only the option in the enabled optgroup",
        )?;
        page.check(
            &js,
            "page.getByRole('option')",
            &[one, two, three],
            "every option",
        )
    }

    /// `toBeDisabled` is `elementState('disabled')`, which calls the same `getAriaDisabled` as the
    /// role engine's `disabled` filter, so the two pages are restated as `[disabled]` queries.
    #[test]
    fn should_inherit_disabled_from_the_ancestor() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(
            NodeBuilder::root().child(plain(10).aria_disabled(true).child(button(11, "Click me!"))),
        )?;
        page.check(
            &js,
            "page.locator('role=button[disabled]')",
            &page.nodes(&[11])?,
            "a button inherits aria-disabled from its span",
        )?;

        let page = Page::new(
            NodeBuilder::root().child(
                plain(10)
                    .aria_disabled(true)
                    .child(el("h1", 11).child(NodeBuilder::text("Heading"))),
            ),
        )?;
        page.check(
            &js,
            "page.locator('role=heading[disabled]')",
            &[],
            "a heading is not in kAriaDisabledRoles, so it inherits nothing",
        )?;
        page.check(
            &js,
            "page.locator('role=heading[disabled=false]')",
            &page.nodes(&[11])?,
            "the heading is enabled",
        )
    }

    /// The spec reaches each button with `getByTestId`, a page part, and asserts
    /// `toBeEnabled`/`toBeDisabled`; both read `getAriaDisabled`, as `[disabled]` does. Chromium
    /// decides the fieldset and legend rules and sends the result as each control's `disabled`.
    #[test]
    fn should_support_disabled_fieldset() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            el("fieldset", 10).contract_disabled(true).children(vec![
                input(11, FormControlType::InputText).contract_disabled(true),
                // inside-fieldset-element
                button(12, "x").contract_disabled(true),
                // inside-legend-element: the first legend's content stays enabled.
                el("legend", 13).child(button(14, "legend")),
            ]),
            el("fieldset", 20).contract_disabled(true).child(
                // nested-inside-legend-element
                el("legend", 21).child(plain(22).child(button(23, "x"))),
            ),
            el("fieldset", 30).contract_disabled(true).children(vec![
                // first-legend-element
                el("legend", 31).child(button(32, "x")),
                // second-legend-element: only the first legend is exempt.
                el("legend", 33).child(button(34, "x").contract_disabled(true)),
            ]),
            el("fieldset", 40).contract_disabled(true).child(
                el("fieldset", 41)
                    .contract_disabled(true)
                    // deep-button
                    .child(button(42, "x").contract_disabled(true)),
            ),
        ]))?;
        page.check(
            &js,
            "page.locator('role=button[disabled=false]')",
            &page.nodes(&[14, 23, 32])?,
            "inside-legend, nested-inside-legend and first-legend are enabled",
        )?;
        page.check(
            &js,
            "page.locator('role=button[disabled]')",
            &page.nodes(&[12, 34, 42])?,
            "inside-fieldset, second-legend and deep-button are disabled",
        )
    }

    #[test]
    fn should_support_level() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            el("h1", 10).child(NodeBuilder::text("Hello")),
            el("h3", 11).child(NodeBuilder::text("Hi")),
            plain(12).role("heading").level(5),
        ]))?;
        let cases: [(&str, i32); 5] = [
            ("page.locator(`role=heading[level=1]`)", 10),
            ("page.getByRole('heading', { level: 1 })", 10),
            ("page.locator(`role=heading[level=3]`)", 11),
            ("page.getByRole('heading', { level: 3 })", 11),
            ("page.locator(`role=heading[level=5]`)", 12),
        ];
        for (locator, expected) in cases {
            page.check(
                &js,
                locator,
                &page.nodes(&[expected])?,
                "the tag's level, or aria-level on a role=heading",
            )?;
        }
        Ok(())
    }

    /// D2: elements with no box have no APC node, so `include-hidden` cannot reach them. The
    /// spec's `include-hidden` lists lose exactly those: `<button hidden>Hello</button>`,
    /// `<button style="display:none">Never</button>`, and `Shadow2`, inside a `display:none`
    /// host. Everything else keeps the spec's lists.
    #[test]
    fn should_filter_hidden_unless_explicitly_asked_for() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            button(10, "Hi"),
            button(12, "Yay").aria_hidden(true),
            button(13, "Nay").aria_hidden(false),
            button(14, "Bye").visibility_hidden(true),
            // `visibility` inherits through computed style, so Chromium reports it on the child.
            plain(15)
                .visibility_hidden(true)
                .child(button(16, "Oh").visibility_hidden(true)),
            plain(17)
                .visibility_hidden(true)
                .child(button(18, "Still here")),
            // host1's open shadow root, flattened under the host.
            plain(20).child(button(21, "Shadow1")),
            input(22, FormControlType::InputText),
            // A closed `<details>`: its content slot carries CONTENT_HIDDEN (design 3.5).
            el("details", 23).children(vec![
                ua_slot().child(el("summary", 24).child(NodeBuilder::text("Open form"))),
                ua_slot()
                    .annotated_role(AnnotatedRole::ContentHidden)
                    .child(el("label", 25).children(vec![
                        NodeBuilder::text("Label"),
                        input(26, FormControlType::InputText),
                    ])),
            ]),
            // `<option style="display:none">Two</option>` is hidden for ARIA upstream and absent
            // here, as D2 drops every element with no box; if chunk 5 synthesizes it from
            // `select_options`, it must mark it hidden.
            el("select", 27).children(vec![
                synthesized_option().visibility_hidden(true),
                synthesized_option(),
            ]),
        ]))?;
        let visible = page.nodes(&[10, 13, 18, 21])?;
        for locator in [
            "page.locator(`role=button`)",
            "page.locator(`role=button[include-hidden=false]`)",
        ] {
            page.check(
                &js,
                locator,
                &visible,
                "aria-hidden and visibility:hidden buttons are skipped",
            )?;
        }
        for locator in [
            "page.locator(`role=button[include-hidden]`)",
            "page.locator(`role=button[include-hidden=true]`)",
        ] {
            page.check(
                &js,
                locator,
                &page.nodes(&[10, 12, 13, 14, 16, 18, 21])?,
                "every button the capture holds; Hello, Never and Shadow2 have no node (D2)",
            )?;
        }
        page.check(
            &js,
            "page.locator(`role=textbox`)",
            &page.nodes(&[22])?,
            "the input inside the closed details is content-hidden",
        )?;
        page.check(
            &js,
            "page.locator(`role=option`)",
            &[page.child(27, 0)?, page.child(27, 1)?],
            "visibility does not hide an option inside a select",
        )
    }

    #[test]
    fn should_support_name() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            plain(10).role("button").aria_label(" Hello "),
            plain(11).role("button").aria_label("Hallo"),
            plain(12).role("button").aria_label("Hello").aria_hidden(true),
            plain(13).role("button").aria_label("123").aria_hidden(true),
            plain(14)
                .role("button")
                .aria_label("foo\"bar")
                .aria_hidden(true),
        ]))?;
        let cases: [(&str, &[i32], &str); 14] = [
            (
                r#"page.locator(`role=button[name="Hello"]`)"#,
                &[10],
                "exact equality after normalizing the name",
            ),
            (
                r#"page.locator(`role=button[name=" \n Hello "]`)"#,
                &[10],
                "the needle is normalized too",
            ),
            (
                "page.getByRole('button', { name: 'Hello' })",
                &[10],
                "a case-insensitive substring that skips the hidden one",
            ),
            (
                r#"page.locator(`role=button[name*="all"]`)"#,
                &[11],
                "a substring",
            ),
            (
                "page.locator(`role=button[name=/^H[ae]llo$/]`)",
                &[10, 11],
                "a regex on the normalized name",
            ),
            (
                "page.getByRole('button', { name: /^H[ae]llo$/ })",
                &[10, 11],
                "a regex on the normalized name",
            ),
            (
                "page.locator(`role=button[name=/h.*o/i]`)",
                &[10, 11],
                "the regex's own i flag",
            ),
            (
                "page.getByRole('button', { name: /h.*o/i })",
                &[10, 11],
                "the regex's own i flag",
            ),
            (
                r#"page.locator(`role=button[name="Hello"][include-hidden]`)"#,
                &[10, 12],
                "include-hidden adds the aria-hidden one",
            ),
            (
                "page.getByRole('button', { name: 'Hello', includeHidden: true })",
                &[10, 12],
                "include-hidden adds the aria-hidden one",
            ),
            (
                "page.getByRole('button', { name: 'hello', includeHidden: true })",
                &[10, 12],
                "getByRole's name is case-insensitive",
            ),
            (
                "page.locator(`role=button[name=Hello]`)",
                &[10],
                "an unquoted value is a case-sensitive string",
            ),
            (
                "page.locator(`role=button[name=123][include-hidden]`)",
                &[13],
                "an unquoted 123 stays the string \"123\"",
            ),
            (
                "page.getByRole('button', { name: '123', includeHidden: true })",
                &[13],
                "a numeric name",
            ),
        ];
        for (locator, expected, why) in cases {
            page.check(&js, locator, &page.nodes(expected)?, why)?;
        }
        Ok(())
    }

    #[test]
    fn should_support_option_name_with_html_whitespace() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        // `<option>&nbsp;HTML</option>`: the option's text keeps the U+00A0, and its name comes
        // from that content.
        let page = Page::new(NodeBuilder::root().child(
            el("select", 10).child(synthesized_option().child(NodeBuilder::text("\u{a0}HTML"))),
        ))?;
        let option = page.child(10, 0)?;
        let count = page
            .select(&js, "page.getByRole('option', { name: 'HTML' })")?
            .len();
        assert_eq!(count, 1, "toHaveCount(1): U+00A0 trims as white space");
        page.check(
            &js,
            "page.getByRole('option', { name: 'HTML', exact: true })",
            &[option],
            "exact equality holds only because the name is whitespace-normalized",
        )
    }

    #[test]
    fn errors() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().child(button(10, "Go")))?;
        let cases = [
            ("page.$('role=[bar]')", "Role must not be empty"),
            (
                "page.$('role=foo[sElected]')",
                r#"Unknown attribute "sElected", must be one of "checked", "description", "disabled", "expanded", "include-hidden", "level", "name", "pressed", "selected""#,
            ),
            (
                "page.$('role=foo[bar . qux=true]')",
                r#"Unknown attribute "bar.qux""#,
            ),
            (
                r#"page.$('role=heading[level="bar"]')"#,
                r#""level" attribute must be compared to a number"#,
            ),
            (
                r#"page.$('role=checkbox[checked="bar"]')"#,
                r#""checked" must be one of true, false, "mixed""#,
            ),
            (
                "page.$('role=checkbox[checked~=true]')",
                "cannot use ~= in attribute with non-string matching value",
            ),
            (
                "page.$('role=button[level=3]')",
                r#""level" attribute is only supported for roles: "heading", "listitem", "row", "treeitem""#,
            ),
            (
                "page.$('role=button[name]')",
                r#""name" attribute must have a value"#,
            ),
            (
                r#"page.$('role=treeitem[expanded="none"]')"#,
                r#""expanded" must be one of true, false"#,
            ),
        ];
        for (locator, expected) in cases {
            let error = page
                .try_select(&js, locator)?
                .err()
                .ok_or_else(|| format!("{locator} must fail"))?;
            let message = error.to_string();
            assert!(
                message.contains(expected),
                "{locator}: the message is upstream's, got {message:?}"
            );
        }
        // The engine parses its body only when a root reaches it, so the second part never runs.
        let lazy = page.select(&js, "page.$('role=nothing >> role=[bar]')")?;
        assert!(
            lazy.is_empty(),
            "no element has role `nothing`, so `role=[bar]` never runs and nothing fails"
        );
        Ok(())
    }

    /// D2: an unslotted light child has no box, so the `include-hidden` list loses `hidden1` and
    /// `hidden2`, the buttons of the hosts whose shadow root has no `<slot>`.
    #[test]
    fn hidden_with_shadow_dom_slots() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().children(vec![
            plain(10).child(NodeBuilder::text("nothing to see here")),
            plain(12).child(NodeBuilder::text("nothing to see here")),
            // A `<slot>` is `display: contents`; its assigned nodes are its flat-tree children.
            plain(14).child(
                el("slot", 15)
                    .display_contents(true)
                    .child(button(16, "visible1")),
            ),
            plain(17).child(
                el("slot", 18)
                    .display_contents(true)
                    .child(plain(19).child(button(20, "visible2"))),
            ),
        ]))?;
        let visible = page.nodes(&[16, 20])?;
        page.check(
            &js,
            "page.locator(`role=button`)",
            &visible,
            "the slotted buttons",
        )?;
        page.check(
            &js,
            "page.locator(`role=button[include-hidden]`)",
            &visible,
            "the unslotted buttons have no node (D2)",
        )
    }

    /// `toBeVisible` is a page assertion; the restatement checks that the locator finds the
    /// output.
    #[test]
    fn should_support_output_accessible_name() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        // Chromium sends a wrapping label's control as its `label_for`, as it does for `for=`.
        let page = Page::new(
            NodeBuilder::root().child(el("label", 10).label_for(11).children(vec![
                NodeBuilder::text("Output1"),
                el("output", 11).child(NodeBuilder::text("output")),
            ])),
        )?;
        page.check(
            &js,
            "page.getByRole('status', { name: 'Output1' })",
            &page.nodes(&[11])?,
            "an <output> is a status named by its label",
        )
    }

    /// `toHaveText` reads text in the page; the restatement checks the count and the nodes.
    #[test]
    fn should_not_match_scope_by_default() -> Result<(), Box<dyn Error>> {
        let js = Prelude::load()?;
        let page = Page::new(NodeBuilder::root().child(el("ul", 10).child(
            el("li", 11).aria_label("Parent list").children(vec![
                NodeBuilder::text("Parent list"),
                el("ul", 12).children(vec![
                    el("li", 13).child(NodeBuilder::text("child 1")),
                    el("li", 14).child(NodeBuilder::text("child 2")),
                ]),
            ]),
        )))?;
        let children = page.select(
            &js,
            "page.getByRole('listitem', { name: 'Parent list' }).getByRole('listitem')",
        )?;
        assert_eq!(children.len(), 2, "toHaveCount(2)");
        assert_eq!(
            children.nodes(),
            page.nodes(&[13, 14])?,
            "the root listitem is not its own descendant"
        );
        Ok(())
    }
}

/// A list page for chains. Each listitem is named by its `aria-label`, since a listitem takes no
/// name from its content, and each button by its text:
///
/// ```text
/// ul 10
///   li 11 "A"   > button 12 "Edit"
///   li 13 "B"   > ul 14 > li 15 "B1" > button 16 "Edit"
///                         li 17 "B2"
///   li 18 "C"   > button 19 "Delete"
/// nav 20 > button 21 "Go", button 22 "Stop"
/// ```
fn list_page() -> Result<Page, Box<dyn Error>> {
    Page::new(NodeBuilder::root().children(vec![
        el("ul", 10).children(vec![
            el("li", 11).aria_label("A").child(button(12, "Edit")),
            el("li", 13).aria_label("B").child(el("ul", 14).children(vec![
                el("li", 15).aria_label("B1").child(button(16, "Edit")),
                el("li", 17).aria_label("B2"),
            ])),
            el("li", 18).aria_label("C").child(button(19, "Delete")),
        ]),
        el("nav", 20).children(vec![button(21, "Go"), button(22, "Stop")]),
    ]))
}

/// The parts of `chunk` after its first, as `ChunkPlan` hands a later stage of one chunk.
fn after_first(chunk: &ParsedSelector) -> ParsedSelector {
    ParsedSelector {
        parts: chunk.parts.iter().skip(1).cloned().collect(),
        capture: None,
    }
}

#[test]
fn has_and_has_not_keep_element_roots_by_their_nested_query() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    page.check(
        &js,
        "page.getByRole('listitem').filter({ has: page.getByRole('button') })",
        &page.nodes(&[11, 13, 15, 18])?,
        "B holds a button through B1; B2 holds none",
    )?;
    page.check(
        &js,
        "page.getByRole('listitem').filter({ has: page.getByRole('button', { name: 'Edit' }) })",
        &page.nodes(&[11, 13, 15])?,
        "only the items holding an Edit button",
    )?;
    page.check(
        &js,
        "page.getByRole('listitem').filter({ hasNot: page.getByRole('button') })",
        &page.nodes(&[17])?,
        "B2 is the one item with no button",
    )?;

    // Playwright's parser refuses a nested part first, so a stage that starts with one is a later
    // stage of its chunk; run from the document root, it meets the `Document` itself.
    let document = page.document()?;
    let names = page.tree.name_cache();
    let has = TreeStage::new(&after_first(
        &js.chunk("page.getByRole('list').filter({ has: page.getByRole('button') })")?,
    ))?;
    assert!(
        has.select(&page.tree, document, &names, &page.refs)?
            .is_empty(),
        "the document root is not an element, so `internal:has` never keeps it"
    );
    let chain = TreeStage::new(&after_first(
        &js.chunk("page.getByRole('list').locator(page.getByRole('button'))")?,
    ))?;
    assert_eq!(
        chain
            .select(&page.tree, document, &names, &page.refs)?
            .nodes(),
        page.nodes(&[12, 16, 19, 21, 22])?,
        "`internal:chain` has no element check, so it runs from the document"
    );
    Ok(())
}

#[test]
fn and_or_chain_and_describe_follow_query_selector_all() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    page.check(
        &js,
        "page.getByRole('button').and(page.getByRole('button', { name: 'Edit' }))",
        &page.nodes(&[12, 16])?,
        "the buttons that are also Edit buttons",
    )?;
    page.check(
        &js,
        "page.getByRole('button', { name: 'Go' }).or(page.getByRole('listitem', { name: 'A' }))",
        &page.nodes(&[11, 21])?,
        "the union is sorted in document order, so A comes before Go",
    )?;
    page.check(
        &js,
        "page.getByRole('listitem', { name: 'B', exact: true }).locator(page.getByRole('button'))",
        &page.nodes(&[16])?,
        "the chained locator runs inside B",
    )?;
    page.check(
        &js,
        "page.getByRole('button').describe('every button')",
        &page.nodes(&[12, 16, 19, 21, 22])?,
        "describe is the identity on elements",
    )?;
    page.check(
        &js,
        &locator("internal:describe=\"x\"")?,
        &[],
        "a describe that leads runs on the document root, which is not an element",
    )?;
    let ul = page.node(10)?;
    let from_element =
        page.try_select_in(&js, &locator("internal:describe=\"x\"")?, Scope::Root(ul))??;
    assert_eq!(
        from_element.nodes(),
        [ul],
        "from an element root, describe returns that element"
    );
    Ok(())
}

/// `sortInDOMOrder` finds no parent for the `Document` or for `<html>`, so each starts its own
/// tree, visited in the order first appended. An `nth` that keeps the query root brings the
/// `Document` into a union.
#[test]
fn or_places_the_document_where_sort_in_dom_order_does() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    let document = page.root()?;
    let buttons = page.nodes(&[12, 16, 19, 21, 22])?;

    let mut document_last = buttons.clone();
    document_last.push(document);
    page.check(
        &js,
        "page.getByRole('button').or(page.locator('nth=-0'))",
        &document_last,
        "the Document joins the union after the buttons, so its tree is visited after <html>'s",
    )?;

    let mut document_first = vec![document];
    document_first.extend(&buttons);
    page.check(
        &js,
        "page.locator('nth=0').or(page.getByRole('button'))",
        &document_first,
        "the Document is the union's first member, so its tree is visited first",
    )?;

    page.check(
        &js,
        "page.getByRole('button').or(page.locator('nth=0')).first()",
        &page.nodes(&[12])?,
        "a later nth reads the union's order: the first button, not the Document",
    )?;
    page.check(
        &js,
        "page.getByRole('button').or(page.locator('nth=0')).last()",
        &[document],
        "the Document is last",
    )
}

#[test]
fn nth_takes_one_element_by_javascript_slice_rules() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    // The buttons are 12, 16, 19, 21, 22.
    let cases: [(&str, &[i32], &str); 8] = [
        (
            "page.getByRole('button').nth(1)",
            &[16],
            "the second button",
        ),
        ("page.getByRole('button').first()", &[12], "nth=0"),
        (
            "page.getByRole('button').last()",
            &[22],
            "nth=-1 is the last",
        ),
        (
            "page.getByRole('button').nth(-2)",
            &[21],
            "slice(-2, -1) is the second to last",
        ),
        ("page.getByRole('button').nth(5)", &[], "past the end"),
        (
            "page.getByRole('button').nth(1.5)",
            &[16],
            "slice(1.5, 2.5) truncates to slice(1, 2)",
        ),
        (
            "page.getByRole('button').nth(-0.5)",
            &[],
            "slice(-0.5, 0.5) truncates to slice(0, 0)",
        ),
        (
            "page.getByRole('button').nth('abc')",
            &[],
            "+'abc' is NaN, and slice(NaN, NaN) is empty",
        ),
    ];
    for (locator, expected, why) in cases {
        page.check(&js, locator, &page.nodes(expected)?, why)?;
    }
    Ok(())
}

#[test]
fn capture_runs_as_a_trailing_has() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    page.check(
        &js,
        &locator("*role=list >> role=listitem")?,
        &page.nodes(&[10, 14])?,
        "the lists that hold a listitem",
    )?;
    page.check(
        &js,
        &locator("role=list >> *role=listitem >> role=button")?,
        &page.nodes(&[11, 13, 15, 18])?,
        "the listitems that hold a button",
    )?;
    page.check(
        &js,
        &locator("role=list >> *role=listitem")?,
        &page.nodes(&[11, 13, 15, 17, 18])?,
        "a capture on the last part changes nothing",
    )?;
    page.check(
        &js,
        &locator("*role=list >> internal:has=\"role=listitem >> nth=0\"")?,
        &page.nodes(&[10, 14])?,
        "only a top-level `nth` conflicts with the capture",
    )?;

    let error = page
        .try_select(&js, &locator("*role=list >> nth=0")?)?
        .err()
        .ok_or("a capture with nth must fail")?;
    assert_eq!(
        error.to_string(),
        "Can't query n-th element in a request with the capture.",
        "upstream's text"
    );
    let nested = "page.getByRole('list').filter({ has: page.locator('*role=listitem >> nth=0') })";
    assert_eq!(
        page.try_select(&js, nested)?.err(),
        Some(TreeStageError::NthWithCapture),
        "the nested captured selector fails once a list reaches it"
    );
    let unreached = page.select(
        &js,
        "page.getByRole('menu').filter({ has: page.locator('*role=listitem >> nth=0') })",
    )?;
    assert!(
        unreached.is_empty(),
        "with no menu, the nested selector never runs, so nothing fails"
    );
    Ok(())
}

#[test]
fn nested_chains_compose_several_levels_deep() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    page.check(
        &js,
        "page.getByRole('list').filter({ has: page.getByRole('listitem').filter({ hasNot: page.getByRole('button') }) })",
        &page.nodes(&[10, 14])?,
        "both lists hold B2, the item with no button",
    )?;
    page.check(
        &js,
        "page.getByRole('navigation').or(page.getByRole('list').filter({ has: \
         page.getByRole('listitem', { name: 'B2', exact: true }).and(\
         page.getByRole('listitem').filter({ hasNot: page.getByRole('button') })) }))",
        &page.nodes(&[10, 14, 20])?,
        "the lists holding B2, joined with the nav and sorted",
    )?;
    page.check(
        &js,
        "page.getByRole('list').filter({ has: page.getByRole('listitem', { name: 'B1', exact: true }) \
         .locator(page.getByRole('button').and(page.getByRole('button', { name: 'Edit' }))) })",
        &page.nodes(&[10, 14])?,
        "has, chain and and, nested: the lists holding B1's Edit button",
    )
}

#[test]
fn a_later_stage_keeps_its_roots_order_and_and_keeps_the_nested_order() -> Result<(), Box<dyn Error>>
{
    let js = Prelude::load()?;
    let page = list_page()?;
    let root = page.root()?;
    let names = page.tree.name_cache();

    // Out-of-order roots, as a page stage sorted by layout would hand them over.
    let roots = page.nodes(&[18, 11])?;
    let buttons = TreeStage::new(&js.chunk("page.getByRole('button')")?)?;
    let after = buttons.select(
        &page.tree,
        Scope::After {
            root,
            roots: &roots,
        },
        &names,
        &page.refs,
    )?;
    assert_eq!(
        after.nodes(),
        page.nodes(&[19, 12])?,
        "an engine visits roots in the order it gets them: C's button, then A's"
    );

    let edit = TreeStage::new(&after_first(&js.chunk(
        "page.getByRole('button').and(page.getByRole('button', { name: 'Edit' }))",
    )?))?;
    let reversed = page.nodes(&[22, 21, 19, 16, 12])?;
    let anded = edit.select(
        &page.tree,
        Scope::After {
            root,
            roots: &reversed,
        },
        &names,
        &page.refs,
    )?;
    assert_eq!(
        anded.nodes(),
        page.nodes(&[12, 16])?,
        "`internal:and` keeps the nested result's order, not the roots'"
    );

    let or = TreeStage::new(&after_first(&js.chunk(
        "page.getByRole('button', { name: 'Go' }).or(page.getByRole('button', { name: 'Edit' }))",
    )?))?;
    let roots = page.nodes(&[21, 19])?;
    let ored = or.select(
        &page.tree,
        Scope::After {
            root,
            roots: &roots,
        },
        &names,
        &page.refs,
    )?;
    assert_eq!(
        ored.nodes(),
        page.nodes(&[12, 16, 19, 21])?,
        "`internal:or` sorts the union in document order"
    );

    let has = TreeStage::new(&after_first(
        &js.chunk("page.getByRole('listitem').filter({ has: page.getByRole('button') })")?,
    ))?;
    let roots = page.nodes(&[18, 17, 11])?;
    let kept = has.select(
        &page.tree,
        Scope::After {
            root,
            roots: &roots,
        },
        &names,
        &page.refs,
    )?;
    assert_eq!(
        kept.nodes(),
        page.nodes(&[18, 11])?,
        "`internal:has` filters the roots in their order"
    );

    let captured = TreeStage::new(&js.chunk(&locator("*role=list >> nth=0")?)?)?;
    assert_eq!(
        captured
            .select(
                &page.tree,
                Scope::After {
                    root,
                    roots: &roots
                },
                &names,
                &page.refs
            )
            .err(),
        Some(TreeStageError::NthWithCapture),
        "a captured selector with nth fails from a later stage too"
    );
    Ok(())
}

#[test]
fn and_keeps_a_nested_order_that_is_not_document_order() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    // Each list's last listitem: the outer list gives C (18) before the inner list gives B2 (17).
    page.check(
        &js,
        "page.getByRole('listitem').and(page.getByRole('list').locator(page.getByRole('listitem').last()))",
        &page.nodes(&[18, 17])?,
        "`internal:and` keeps the nested result's order even where it is not document order",
    )
}

#[test]
fn an_element_scope_and_unknown_roots() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = list_page()?;
    let b = page.node(13)?;
    let inside_b = page.try_select_in(&js, "page.getByRole('button')", Scope::Root(b))??;
    assert_eq!(
        inside_b.nodes(),
        page.nodes(&[16])?,
        "from an element root, only its descendants"
    );
    let anded = page.try_select_in(
        &js,
        "page.getByRole('button').and(page.getByRole('button', { name: 'Go' }))",
        Scope::Root(b),
    )??;
    assert!(
        anded.is_empty(),
        "`internal:and` runs its nested query from the query root, B, which holds no Go button"
    );

    let missing = NodeId::new(10_000);
    let none = page.try_select_in(&js, "page.getByRole('button')", Scope::Root(missing))??;
    assert!(
        none.is_empty(),
        "a root that is not in the tree matches nothing"
    );
    let root = page.root()?;
    let roots = [missing, page.node(18)?, page.node(18)?];
    let stage = TreeStage::new(&js.chunk("page.getByRole('button')")?)?;
    let after = stage.select(
        &page.tree,
        Scope::After {
            root,
            roots: &roots,
        },
        &page.tree.name_cache(),
        &page.refs,
    )?;
    assert_eq!(
        after.nodes(),
        page.nodes(&[19])?,
        "unknown roots are dropped and repeated ones count once"
    );
    Ok(())
}

#[test]
fn not_tree_stage_names_the_first_page_part() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let cases = [
        ("page.locator('nav button')", "css"),
        (
            "page.getByRole('list').filter({ has: page.locator('li') }).getByText('x')",
            "css",
        ),
        ("page.getByRole('list').getByText('x')", "internal:text"),
        (
            "page.getByRole('list').filter({ hasText: 'x' })",
            "internal:has-text",
        ),
        (
            r#"page.getByRole('list').locator('left-of="role=button"')"#,
            "left-of",
        ),
        (
            "page.getByRole('list').filter({ has: page.getByRole('listitem').getByTestId('x') })",
            "internal:testid",
        ),
    ];
    for (locator, part) in cases {
        let refused = TreeStage::new(&js.chunk(locator)?)
            .err()
            .ok_or_else(|| format!("{locator} must not be a tree stage"))?;
        assert_eq!(
            refused.part(),
            part,
            "{locator}: the first page part, nested ones included"
        );
    }

    let nested = SelectorBody::Nested(NestedSelectorBody {
        parsed: js.chunk("page.getByRole('button')")?,
        distance: None,
    });
    let mismatched = ParsedSelector {
        parts: vec![SelectorPart {
            name: "role".to_owned(),
            body: nested,
            source: String::new(),
        }],
        capture: None,
    };
    let refused = TreeStage::new(&mismatched)
        .err()
        .ok_or("a role part with a nested body is not a tree part")?;
    assert_eq!(
        refused.part(),
        "role",
        "a body of the wrong shape names its part"
    );
    Ok(())
}

#[test]
fn names_are_read_only_for_nodes_past_every_cheaper_check() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(NodeBuilder::root().children(vec![
        button(10, "Go").aria_description("first"),
        button(11, "Go").aria_hidden(true),
        button(12, "Go").contract_disabled(true),
        plain(13).role("link").aria_label("Go"),
    ]))?;
    let (go, hidden, disabled) = (page.node(10)?, page.node(11)?, page.node(12)?);
    let text = RecordingText {
        names: page.tree.name_cache(),
        reads: RefCell::default(),
    };
    let select = |locator: &str| -> Result<TreeMatches, Box<dyn Error>> {
        let stage = TreeStage::new(&js.chunk(locator)?)?;
        Ok(stage.select(&page.tree, page.document()?, &text, &page.refs)?)
    };
    let read = |field, node, visibility| TextRead {
        field,
        node,
        visibility,
    };

    select("page.getByRole('button', { name: 'Go', disabled: false })")?;
    assert_eq!(
        text.reads.take(),
        [read(Field::Name, go, NameVisibility::VisibleOnly)],
        "the link fails on role, the disabled button on state and the hidden one on hidden-ness, \
         all before the name"
    );

    select("page.getByRole('button', { name: 'Go', disabled: false, includeHidden: true })")?;
    assert_eq!(
        text.reads.take(),
        [
            read(Field::Name, go, NameVisibility::IncludeHidden),
            read(Field::Name, hidden, NameVisibility::IncludeHidden),
        ],
        "include-hidden reaches the hidden button and asks for names with hidden content"
    );

    let described = select("page.locator('role=button[name=\"Go\"][description=\"first\"]')")?;
    assert_eq!(
        described.nodes(),
        [go],
        "the description filter keeps the first"
    );
    assert_eq!(
        text.reads.take(),
        [
            read(Field::Name, go, NameVisibility::VisibleOnly),
            read(Field::Description, go, NameVisibility::VisibleOnly),
            read(Field::Name, disabled, NameVisibility::VisibleOnly),
            read(Field::Description, disabled, NameVisibility::VisibleOnly),
        ],
        "the description is read after the name matched, node by node"
    );
    Ok(())
}

/// Production passes `&tree.name_cache()`, so a filter matches the name pass's answer for the
/// visibility the part asks for: a hidden span inside a button that takes its name from content
/// is in its `IncludeHidden` name and not in its `VisibleOnly` one.
#[test]
fn the_name_cache_hands_the_stage_the_name_pass_answers() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(
        NodeBuilder::root().child(
            el("button", 10)
                .aria_description("Charges the card")
                .children(vec![
                    NodeBuilder::text("Pay "),
                    plain(11).aria_hidden(true).child(NodeBuilder::text("now")),
                ]),
        ),
    )?;
    let pay = page.node(10)?;
    let names = page.tree.name_cache();
    assert_eq!(
        (
            names.name(pay, NameVisibility::VisibleOnly).text(),
            names.name(pay, NameVisibility::IncludeHidden).text(),
        ),
        ("Pay", "Pay now"),
        "the fixture's two names differ, so each case below tells the visibilities apart"
    );
    let cases: [(&str, &[NodeId], &str); 6] = [
        (
            "page.getByRole('button', { name: 'Pay', exact: true })",
            &[pay],
            "without include-hidden the stage reads the VisibleOnly name",
        ),
        (
            "page.getByRole('button', { name: 'Pay now', exact: true })",
            &[],
            "the aria-hidden span is not in the VisibleOnly name",
        ),
        (
            "page.getByRole('button', { name: 'Pay now', exact: true, includeHidden: true })",
            &[pay],
            "include-hidden reads the IncludeHidden name, which holds the hidden span",
        ),
        (
            "page.getByRole('button', { name: 'Pay', exact: true, includeHidden: true })",
            &[],
            "include-hidden does not fall back to the VisibleOnly name",
        ),
        (
            "page.locator('role=button[description=\"Charges the card\"]')",
            &[pay],
            "the description filter reads the name pass's description",
        ),
        (
            "page.locator('role=button[description=\"Pay\"]')",
            &[],
            "the description is not the name",
        ),
    ];
    for (locator, expected, why) in cases {
        page.check(&js, locator, expected, why)?;
    }
    Ok(())
}

/// A main document with a stitched child document between two main-document buttons.
fn framed_page() -> Result<Page, Box<dyn Error>> {
    let child = FrameBuilder::new("doc-child").root(NodeBuilder::root().child(button(1, "Child")));
    Page::new(NodeBuilder::root().children(vec![
        button(10, "Before"),
        NodeBuilder::iframe(child).tag("iframe").dom_id(11),
        button(12, "After"),
    ]))
}

#[test]
fn a_query_stays_in_its_document() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = framed_page()?;
    page.check(
        &js,
        "page.getByRole('button')",
        &page.nodes(&[10, 12])?,
        "the child document's button is skipped, and the scan resumes after the iframe",
    )?;
    let token = DocToken::new("doc-child").ok_or("non-empty token")?;
    let child_doc = page
        .tree
        .document_by_token(&token)
        .ok_or("the child document is stitched")?
        .id();
    let scope = Scope::document(&page.tree, child_doc).ok_or("the child has a root")?;
    let inside = page.try_select_in(&js, "page.getByRole('button')", scope)??;
    assert_eq!(
        inside.nodes(),
        [page.node_in("doc-child", 1)?],
        "the child document's own scope finds its button"
    );
    let iframe = page.node(11)?;
    let from_iframe =
        page.try_select_in(&js, "page.getByRole('button')", Scope::Root(iframe))??;
    assert!(
        from_iframe.is_empty(),
        "an <iframe> element has no descendants in its own document"
    );
    Ok(())
}

#[test]
fn aria_ref_answers_inside_its_scope_and_reports_the_first_stale_ref() -> Result<(), Box<dyn Error>>
{
    let js = Prelude::load()?;
    let mut page = list_page()?;
    for (body, dom) in [("e1", 15), ("e2", 12)] {
        page.refs.0.insert(body.to_owned(), Page::key(MAIN, dom)?);
    }
    // A live ref whose node is not in this capture.
    page.refs.0.insert("e4".to_owned(), Page::key(MAIN, 999)?);

    let held = page.select(
        &js,
        "page.getByRole('listitem').filter({ has: page.locator('aria-ref=e1') })",
    )?;
    assert_eq!(
        held.nodes(),
        page.nodes(&[13, 15])?,
        "B holds B1, and B1 holds itself: the scope is inclusive"
    );
    assert_eq!(held.stale_ref(), None, "every ref resolved");

    let outside = page.select(
        &js,
        "page.getByRole('listitem', { name: 'C', exact: true }).locator('aria-ref=e2')",
    )?;
    assert!(outside.is_empty(), "A's button is outside C");
    assert_eq!(outside.stale_ref(), None, "outside the scope is not stale");

    let inside = page.select(
        &js,
        "page.getByRole('listitem', { name: 'A', exact: true }).locator('aria-ref=e2')",
    )?;
    assert_eq!(inside.nodes(), page.nodes(&[12])?, "A's button is inside A");

    let absent = page.select(&js, "page.locator('aria-ref=e4')")?;
    assert!(
        absent.is_empty(),
        "a node absent from the capture does not match"
    );
    assert_eq!(
        absent.stale_ref(),
        None,
        "only hydration can tell a removed node from one with no box, so it is not stale"
    );

    let stale = page.select(
        &js,
        "page.locator('aria-ref=e98').or(page.locator('aria-ref=e99'))",
    )?;
    assert!(stale.is_empty(), "neither ref names a node");
    assert_eq!(stale.stale_ref(), Some("e98"), "the first stale ref wins");

    let mixed = page.select(
        &js,
        "page.locator('aria-ref=e2').or(page.locator('aria-ref=e99'))",
    )?;
    assert_eq!(
        mixed.nodes(),
        page.nodes(&[12])?,
        "the live ref still matches"
    );
    assert_eq!(
        mixed.stale_ref(),
        Some("e99"),
        "a stale ref inside a nested selector reports into the same run"
    );
    Ok(())
}

#[test]
fn aria_ref_never_crosses_into_another_document() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let mut page = framed_page()?;
    page.refs
        .0
        .insert("f1e1".to_owned(), Page::key("doc-child", 1)?);
    let from_main = page.select(&js, "page.locator('aria-ref=f1e1')")?;
    assert!(
        from_main.is_empty(),
        "the main document's root does not contain the child document's button"
    );
    assert_eq!(from_main.stale_ref(), None, "the ref is live, so not stale");
    Ok(())
}

/// The probe's recorded strict reply (`neo-playwright-js/src/probe/protocol-examples.ts`).
const PROBE_STRICT: &str = "strict mode violation: getByRole('button') resolved to 2 elements:\n    1) <button>Go</button> aka getByRole('button', { name: 'Go' })\n    2) <button>Stop</button> aka getByRole('button', { name: 'Stop' })\n";

/// The live recording in `neo-playwright/tests/reads_live.rs`, as the op reports it.
const LIVE_STRICT: &str = "locator.innerText: Error: strict mode violation: locator('nav button') resolved to 2 elements:\n    \
         1) <button>One</button> aka getByRole('button', { name: 'One' })\n    \
         2) <button>Two</button> aka getByRole('button', { name: 'Two' })\n\
         \nCall log:\n  - waiting for locator('nav button')\n";

fn preview(label: &str) -> StrictPreview {
    StrictPreview {
        preview: format!("<button>{label}</button>"),
        aka: format!("getByRole('button', {{ name: '{label}' }})"),
    }
}

#[test]
fn strict_mode_reproduces_the_probe_recording() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(NodeBuilder::root().children(vec![button(10, "Go"), button(11, "Stop")]))?;
    let all = "page.getByRole('button')";
    let violation = page
        .select(&js, all)?
        .strict()
        .err()
        .ok_or("two buttons violate strict mode")?;
    assert_eq!(violation.count(), 2, "both buttons matched");
    assert_eq!(
        violation.preview_nodes(),
        page.nodes(&[10, 11])?,
        "the page previews the matches in order"
    );
    let locator_text = js.locator_text(all)?;
    assert_eq!(
        locator_text, "getByRole('button')",
        "upstream names the chunk with asLocator"
    );
    assert_eq!(
        violation.message(&locator_text, &[preview("Go"), preview("Stop")]),
        PROBE_STRICT,
        "the message equals the recorded reply byte for byte"
    );
    Ok(())
}

#[test]
fn strict_mode_reproduces_the_live_recording() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(
        NodeBuilder::root()
            .child(el("nav", 10).children(vec![button(11, "One"), button(12, "Two")])),
    )?;
    // `nav button` is CSS; the tree stage that selects the same two buttons stands in for it.
    let violation = page
        .select(&js, "page.getByRole('navigation').getByRole('button')")?
        .strict()
        .err()
        .ok_or("two buttons violate strict mode")?;
    assert_eq!(
        violation.preview_nodes(),
        page.nodes(&[11, 12])?,
        "the two nav buttons, in order"
    );
    let recorded = LIVE_STRICT
        .strip_prefix("locator.innerText: Error: ")
        .and_then(|rest| rest.strip_suffix("\nCall log:\n  - waiting for locator('nav button')\n"))
        .ok_or("the recording wraps the strict message in the op's prefix and call log")?;
    let locator_text = js.locator_text("page.locator('nav button')")?;
    assert_eq!(
        violation.message(&locator_text, &[preview("One"), preview("Two")]),
        recorded,
        "the message equals the live recording byte for byte"
    );
    Ok(())
}

#[test]
fn strict_mode_lists_ten_and_marks_the_rest() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(
        NodeBuilder::root().children((1..=11).map(|n| button(n, &format!("B{n}"))).collect()),
    )?;
    let violation = page
        .select(&js, "page.getByRole('button')")?
        .strict()
        .err()
        .ok_or("eleven buttons violate strict mode")?;
    assert_eq!(violation.count(), 11, "all eleven matched");
    assert_eq!(
        violation.preview_nodes(),
        page.nodes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10])?,
        "upstream previews the first ten"
    );
    let previews: Vec<StrictPreview> = (1..=10).map(|n| preview(&format!("B{n}"))).collect();
    assert_eq!(
        violation.message("getByRole('button')", &previews),
        "strict mode violation: getByRole('button') resolved to 11 elements:\
         \n    1) <button>B1</button> aka getByRole('button', { name: 'B1' })\
         \n    2) <button>B2</button> aka getByRole('button', { name: 'B2' })\
         \n    3) <button>B3</button> aka getByRole('button', { name: 'B3' })\
         \n    4) <button>B4</button> aka getByRole('button', { name: 'B4' })\
         \n    5) <button>B5</button> aka getByRole('button', { name: 'B5' })\
         \n    6) <button>B6</button> aka getByRole('button', { name: 'B6' })\
         \n    7) <button>B7</button> aka getByRole('button', { name: 'B7' })\
         \n    8) <button>B8</button> aka getByRole('button', { name: 'B8' })\
         \n    9) <button>B9</button> aka getByRole('button', { name: 'B9' })\
         \n    10) <button>B10</button> aka getByRole('button', { name: 'B10' })\
         \n    ...\n",
        "ten lines, then `...` for the eleventh"
    );
    Ok(())
}

#[test]
fn strict_mode_accepts_zero_or_one_match() -> Result<(), Box<dyn Error>> {
    let js = Prelude::load()?;
    let page = Page::new(NodeBuilder::root().children(vec![button(10, "Go"), button(11, "Stop")]))?;
    assert_eq!(
        page.select(&js, "page.getByRole('link')")?.strict(),
        Ok(None),
        "nothing matched, so there is nothing to act on and no violation"
    );
    assert_eq!(
        page.select(&js, "page.getByRole('button', { name: 'Go' })")?
            .strict(),
        Ok(Some(page.node(10)?)),
        "one match is the element a strict locator acts on"
    );
    Ok(())
}
