use super::*;

const DOCUMENT_NODE: i64 = 9;
const FRAGMENT_NODE: i64 = 11;

/// Builds one captured document column by column, the way Chromium serializes it.
struct Page {
    strings: Vec<String>,
    nodes: Nodes,
    layout: Layout,
    body: usize,
}

impl Page {
    fn new() -> Self {
        let mut page = Self {
            strings: Vec::new(),
            nodes: Nodes::default(),
            layout: Layout::default(),
            body: 0,
        };
        let document = page.node(None, DOCUMENT_NODE, "#document", &[]);
        let html = page.node(Some(document), ELEMENT_NODE, "HTML", &[]);
        page.render(html, "auto", 800.0, 600.0);
        page.body = page.node(Some(html), ELEMENT_NODE, "BODY", &[]);
        page.render(page.body, "auto", 800.0, 600.0);
        page
    }

    fn string(&mut self, value: &str) -> i64 {
        let index = self
            .strings
            .iter()
            .position(|existing| existing == value)
            .unwrap_or_else(|| {
                self.strings.push(value.to_string());
                self.strings.len() - 1
            });
        i64::try_from(index).unwrap_or(i64::MAX)
    }

    fn node(
        &mut self,
        parent: Option<usize>,
        node_type: i64,
        name: &str,
        attributes: &[(&str, &str)],
    ) -> usize {
        let index = self.nodes.node_name.len();
        self.nodes
            .parent_index
            .push(parent.map_or(-1, |parent| parent as i64));
        self.nodes.node_type.push(node_type);
        let name = self.string(name);
        self.nodes.node_name.push(name);
        self.nodes.backend_node_id.push(backend(index));
        let mut flat = Vec::new();
        for (key, value) in attributes {
            flat.push(self.string(key));
            flat.push(self.string(value));
        }
        self.nodes.attributes.push(flat);
        index
    }

    /// A rendered element with the given cursor and size.
    fn element(
        &mut self,
        parent: usize,
        name: &str,
        attributes: &[(&str, &str)],
        cursor: &str,
    ) -> usize {
        let element = self.node(Some(parent), ELEMENT_NODE, name, attributes);
        self.render(element, cursor, 40.0, 20.0);
        element
    }

    fn render(&mut self, node: usize, cursor: &str, width: f64, height: f64) {
        self.layout.node_index.push(node as i64);
        let cursor = self.string(cursor);
        self.layout.styles.push(vec![cursor]);
        self.layout.bounds.push(vec![0.0, 0.0, width, height]);
    }

    fn listener(&mut self, node: usize) -> usize {
        self.nodes.is_clickable.index.push(node as i64);
        node
    }

    fn shadow_root(&mut self, host: usize, kind: &str) -> usize {
        let root = self.node(Some(host), FRAGMENT_NODE, "#document-fragment", &[]);
        self.nodes.shadow_root_type.index.push(root as i64);
        let kind = self.string(kind);
        self.nodes.shadow_root_type.value.push(kind);
        root
    }

    fn hits(self) -> HashMap<i64, Vec<String>> {
        let document = Document {
            nodes: self.nodes,
            layout: self.layout,
        };
        clickables(&document, &self.strings)
    }
}

fn backend(index: usize) -> i64 {
    1000 + index as i64
}

fn reasons(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

#[test]
fn a_pointer_cursor_a_listener_or_a_tabindex_makes_an_element_a_hit() {
    let mut page = Page::new();
    let body = page.body;
    let pointer = page.element(body, "DIV", &[], "pointer");
    let listener = page.element(body, "DIV", &[], "auto");
    page.listener(listener);
    let focusable = page.element(body, "SPAN", &[("tabindex", "0")], "auto");
    let unfocusable = page.element(body, "SPAN", &[("tabindex", "-1")], "auto");
    let plain = page.element(body, "DIV", &[], "auto");

    let hits = page.hits();

    assert_eq!(
        hits.get(&backend(pointer)),
        Some(&reasons(&["cursor:pointer"]))
    );
    assert_eq!(hits.get(&backend(listener)), Some(&reasons(&["clickable"])));
    assert_eq!(hits.get(&backend(focusable)), Some(&reasons(&["tabindex"])));
    assert!(!hits.contains_key(&backend(unfocusable)));
    assert!(!hits.contains_key(&backend(plain)));
}

#[test]
fn controls_the_accessibility_tree_already_names_are_not_hits() {
    let mut page = Page::new();
    let body = page.body;
    let button = page.element(body, "BUTTON", &[], "pointer");
    let link = page.element(body, "A", &[("href", "/x")], "pointer");
    page.listener(link);
    let role_button = page.element(body, "DIV", &[("role", "Button")], "pointer");
    let presentation = page.element(body, "DIV", &[("role", "presentation")], "pointer");

    let hits = page.hits();

    assert!(!hits.contains_key(&backend(button)));
    assert!(!hits.contains_key(&backend(link)));
    assert!(!hits.contains_key(&backend(role_button)));
    assert!(hits.contains_key(&backend(presentation)));
}

#[test]
fn elements_without_a_visible_box_are_not_hits() {
    let mut page = Page::new();
    let body = page.body;
    let hidden = page.node(Some(body), ELEMENT_NODE, "DIV", &[("tabindex", "0")]);
    let collapsed = page.node(Some(body), ELEMENT_NODE, "DIV", &[]);
    page.render(collapsed, "pointer", 0.0, 20.0);

    let hits = page.hits();

    assert!(!hits.contains_key(&backend(hidden)));
    assert!(!hits.contains_key(&backend(collapsed)));
}

#[test]
fn only_body_content_counts_including_author_shadow_roots() {
    let mut page = Page::new();
    let body = page.body;
    page.render(body, "pointer", 800.0, 600.0);
    let html = page.nodes.parent_index[body] as usize;
    let outside = page.element(html, "DIV", &[("tabindex", "0")], "auto");
    let input = page.element(body, "INPUT", &[], "auto");
    let user_agent = page.shadow_root(input, "user-agent");
    let internal = page.element(user_agent, "DIV", &[("tabindex", "0")], "auto");
    let host = page.element(body, "MY-WIDGET", &[], "auto");
    let closed = page.shadow_root(host, "closed");
    let shadowed = page.element(closed, "DIV", &[("tabindex", "0")], "auto");
    let pseudo = page.element(body, "::before", &[("tabindex", "0")], "auto");
    page.nodes.pseudo_type.index.push(pseudo as i64);
    let before = page.string("before");
    page.nodes.pseudo_type.value.push(before);

    let hits = page.hits();

    assert!(!hits.contains_key(&backend(body)));
    assert!(!hits.contains_key(&backend(outside)));
    assert!(!hits.contains_key(&backend(internal)));
    assert!(hits.contains_key(&backend(shadowed)));
    assert!(!hits.contains_key(&backend(pseudo)));
}

#[test]
fn a_child_inside_a_pointer_target_is_that_target_unless_it_is_its_own() {
    let mut page = Page::new();
    let body = page.body;
    let card = page.element(body, "DIV", &[], "pointer");
    let title = page.element(card, "SPAN", &[], "pointer");
    let tracked = page.element(card, "SPAN", &[], "pointer");
    page.listener(tracked);
    let focusable = page.element(card, "SPAN", &[("tabindex", "0")], "pointer");
    let inline = page.element(card, "SPAN", &[("onclick", "go()")], "pointer");
    page.listener(inline);
    // `display: contents` has no box, so the nearest rendered ancestor is the card.
    let contents = page.node(Some(card), ELEMENT_NODE, "DIV", &[]);
    let nested = page.element(contents, "SPAN", &[], "pointer");
    // A child that names itself and handles clicks is its own control, as a script-set onclick
    // on it would be; a name alone, with no handler, is still part of the card.
    let labelled = page.element(card, "DIV", &[("aria-label", "Delete")], "pointer");
    page.listener(labelled);
    let named_only = page.element(card, "DIV", &[("aria-label", "Caption")], "pointer");
    let blank_label = page.element(card, "DIV", &[("aria-label", " ")], "pointer");
    page.listener(blank_label);

    let hits = page.hits();

    assert!(hits.contains_key(&backend(card)));
    assert!(!hits.contains_key(&backend(title)));
    assert!(!hits.contains_key(&backend(tracked)));
    assert!(!hits.contains_key(&backend(nested)));
    assert_eq!(
        hits.get(&backend(labelled)),
        Some(&reasons(&["cursor:pointer", "clickable"]))
    );
    assert!(!hits.contains_key(&backend(named_only)));
    assert!(!hits.contains_key(&backend(blank_label)));
    assert_eq!(
        hits.get(&backend(focusable)),
        Some(&reasons(&["cursor:pointer", "tabindex"]))
    );
    assert_eq!(
        hits.get(&backend(inline)),
        Some(&reasons(&["cursor:pointer", "clickable"]))
    );
}

#[test]
fn a_listener_that_only_delegates_to_controls_is_not_a_hit() {
    let mut page = Page::new();
    let body = page.body;
    let menu = page.element(body, "UL", &[], "auto");
    page.listener(menu);
    let item = page.element(menu, "LI", &[], "auto");
    page.element(item, "A", &[("href", "/x")], "pointer");
    let list = page.element(body, "UL", &[], "auto");
    page.listener(list);
    let row = page.element(list, "LI", &[("tabindex", "0")], "auto");
    let toggle = page.element(body, "DIV", &[], "auto");
    page.listener(toggle);
    page.element(toggle, "SPAN", &[], "auto");
    let styled = page.element(body, "DIV", &[], "pointer");
    page.listener(styled);
    page.element(styled, "A", &[("href", "/y")], "auto");

    let hits = page.hits();

    assert!(!hits.contains_key(&backend(menu)));
    assert!(!hits.contains_key(&backend(list)));
    assert!(hits.contains_key(&backend(row)));
    assert!(hits.contains_key(&backend(toggle)));
    assert!(hits.contains_key(&backend(styled)));
}

#[test]
fn a_malformed_capture_yields_fewer_hits_not_a_panic() {
    assert!(clickables(&Document::default(), &[]).is_empty());

    let mut page = Page::new();
    let body = page.body;
    let hit = page.element(body, "DIV", &[("tabindex", "0")], "pointer");
    page.nodes.backend_node_id.truncate(hit);
    page.nodes.attributes[body] = vec![99, 100, 101];
    page.layout.styles[0].clear();
    page.layout.bounds[1].truncate(2);
    page.nodes.is_clickable.index.push(-4);

    assert!(page.hits().is_empty());
}
