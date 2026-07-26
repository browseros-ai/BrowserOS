use crate::{
    FrameId,
    snapshot::{
        ax_types::{AxNode, AxValue},
        refs::{DocumentId, MintRef, RefMap},
        roles::{is_interactive_role, is_root_role, is_skip_role, is_value_role},
    },
};
use serde_json::Value;
use std::collections::HashMap;

const IFRAME_ROLES: &[&str] = &["Iframe", "iframe"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IframeStitch {
    pub backend_node_id: i64,
    pub line_index: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderResult {
    pub text: String,
    pub iframes: Vec<IframeStitch>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SnapshotMode {
    #[default]
    Full,
    Interactive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotOptions {
    pub mode: SnapshotMode,
    pub depth: Option<usize>,
}

pub struct RenderOptions<'a> {
    pub refs: &'a mut RefMap,
    pub frame_id: Option<FrameId>,
    pub document_id: Option<DocumentId>,
    pub cursor_hits: Option<HashMap<i64, Vec<String>>>,
    pub base_depth: usize,
}

pub fn render_snapshot<'a>(nodes: &'a [AxNode], opts: &mut RenderOptions<'a>) -> RenderResult {
    let by_id = nodes
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut ctx = RenderContext {
        by_id,
        lines: Vec::new(),
        iframes: Vec::new(),
        opts,
    };

    for root_id in entry_node_ids(nodes) {
        ctx.visit(&root_id, 0);
    }

    RenderResult {
        text: ctx.lines.join("\n"),
        iframes: ctx.iframes,
    }
}

#[must_use]
pub fn apply_snapshot_options(text: &str, options: SnapshotOptions) -> String {
    let text = match options.mode {
        SnapshotMode::Full => text.to_string(),
        SnapshotMode::Interactive => filter_interactive_lines(text),
    };
    if let Some(max_depth) = options.depth {
        filter_depth_lines(&text, max_depth)
    } else {
        text
    }
}

struct RenderContext<'a, 'b> {
    by_id: HashMap<String, &'a AxNode>,
    lines: Vec<String>,
    iframes: Vec<IframeStitch>,
    opts: &'b mut RenderOptions<'a>,
}

impl RenderContext<'_, '_> {
    fn visit(&mut self, node_id: &str, depth: usize) {
        let Some(node) = self.by_id.get(node_id).copied() else {
            return;
        };

        let role = if node.ignored.unwrap_or(false) {
            None
        } else {
            str_val(node.role.as_ref())
        };
        let name = str_val(node.name.as_ref()).unwrap_or_default();
        let is_cursor_hit = node
            .backend_dom_node_id
            .and_then(|id| {
                self.opts
                    .cursor_hits
                    .as_ref()
                    .map(|hits| hits.contains_key(&id))
            })
            .unwrap_or(false);

        if is_dropped(role.as_deref(), &name, is_cursor_hit) {
            for child_id in node.child_ids.as_deref().unwrap_or(&[]) {
                self.visit(child_id, depth);
            }
            return;
        }

        let Some(role) = role else {
            return;
        };
        let absolute_depth = self.opts.base_depth + depth;
        if IFRAME_ROLES.contains(&role.as_str()) {
            let mut line = format!("{}- iframe", "  ".repeat(absolute_depth));
            if !name.is_empty() {
                line.push(' ');
                line.push_str(&json_quote(&name));
            }
            self.lines.push(line);
            if let Some(backend_node_id) = node.backend_dom_node_id {
                self.iframes.push(IframeStitch {
                    backend_node_id,
                    line_index: self.lines.len() - 1,
                    depth: absolute_depth,
                });
            }
            return;
        }

        let line = format_line(node, &role, &name, absolute_depth, self.opts);
        self.lines.push(line);
        for child_id in node.child_ids.as_deref().unwrap_or(&[]) {
            self.visit(child_id, depth + 1);
        }
    }
}

fn entry_node_ids(nodes: &[AxNode]) -> Vec<String> {
    let roots = nodes
        .iter()
        .filter(|node| {
            str_val(node.role.as_ref())
                .as_deref()
                .is_some_and(is_root_role)
        })
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        nodes
            .first()
            .map(|node| vec![node.node_id.clone()])
            .unwrap_or_default()
    } else {
        roots
    }
}

fn is_dropped(role: Option<&str>, name: &str, is_cursor_hit: bool) -> bool {
    let Some(role) = role else {
        return true;
    };
    if is_skip_role(role) || is_root_role(role) {
        return true;
    }
    (role == "generic" || role == "group") && name.is_empty() && !is_cursor_hit
}

fn filter_interactive_lines(text: &str) -> String {
    let lines = split_rendered_lines(text);
    if lines.is_empty() {
        return String::new();
    }

    let mut keep = vec![false; lines.len()];
    let mut ancestors = Vec::<usize>::new();
    for (index, line) in lines.iter().enumerate() {
        let depth = rendered_depth(line);
        if ancestors.len() > depth {
            ancestors.truncate(depth);
        }

        if index == 0 || has_ref(line) || rendered_role(line) == Some("heading") {
            keep[index] = true;
            for ancestor in &ancestors {
                keep[*ancestor] = true;
            }
        }

        if ancestors.len() == depth {
            ancestors.push(index);
        } else if depth < ancestors.len() {
            ancestors[depth] = index;
        } else {
            ancestors.resize(depth, index);
            ancestors.push(index);
        }
    }

    lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| keep[index].then_some(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn filter_depth_lines(text: &str, max_depth: usize) -> String {
    split_rendered_lines(text)
        .into_iter()
        .filter(|line| rendered_depth(line) <= max_depth)
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_rendered_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

fn rendered_depth(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count() / 2
}

fn rendered_role(line: &str) -> Option<&str> {
    let body = line.trim_start().strip_prefix("- ")?;
    let end = body
        .find(|ch: char| ch.is_whitespace() || ch == '[' || ch == ':')
        .unwrap_or(body.len());
    Some(&body[..end])
}

fn has_ref(line: &str) -> bool {
    line.contains(" [ref=e")
}

fn format_line(
    node: &AxNode,
    role: &str,
    name: &str,
    depth: usize,
    opts: &mut RenderOptions<'_>,
) -> String {
    let mut line = format!("{}- {role}", "  ".repeat(depth));
    if !name.is_empty() {
        line.push(' ');
        line.push_str(&json_quote(name));
    }

    for state in format_states(node) {
        line.push_str(" [");
        line.push_str(&state);
        line.push(']');
    }

    let cursor_reasons = node
        .backend_dom_node_id
        .and_then(|id| opts.cursor_hits.as_ref().and_then(|hits| hits.get(&id)));
    let actionable = node.backend_dom_node_id.is_some()
        && (is_interactive_role(role) || cursor_reasons.is_some());

    if actionable && let Some(backend_node_id) = node.backend_dom_node_id {
        let ref_id = opts.refs.mint(MintRef {
            backend_node_id,
            role,
            name,
            document_id: opts.document_id.as_deref(),
            frame_id: opts.frame_id.as_ref(),
        });
        line.push_str(" [ref=");
        line.push_str(ref_id.as_str());
        line.push(']');
    }
    if cursor_reasons.is_some() {
        line.push_str(" [cursor=pointer]");
    }

    if is_value_role(role)
        && let Some(value) = str_val(node.value.as_ref())
        && !value.is_empty()
    {
        line.push_str(": ");
        line.push_str(&json_quote(&value));
    }

    line
}

fn format_states(node: &AxNode) -> Vec<String> {
    let mut states = Vec::new();
    for prop in node.properties.as_deref().unwrap_or(&[]) {
        let value = prop.value.value.as_ref();
        match prop.name.as_str() {
            "checked" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("checked".to_string());
                } else if value == Some(&Value::String("mixed".to_string())) {
                    states.push("indeterminate".to_string());
                }
            }
            "disabled" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("disabled".to_string());
                }
            }
            "expanded" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("expanded".to_string());
                } else if value == Some(&Value::Bool(false)) {
                    states.push("collapsed".to_string());
                }
            }
            "required" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("required".to_string());
                }
            }
            "selected" => {
                if value == Some(&Value::Bool(true)) {
                    states.push("selected".to_string());
                }
            }
            "level" => {
                if let Some(value) = value {
                    states.push(format!("level={}", value_display(value)));
                }
            }
            _ => {}
        }
    }
    states
}

fn str_val(value: Option<&AxValue>) -> Option<String> {
    value.and_then(|value| value.value.as_ref()?.as_str().map(ToString::to_string))
}

fn value_display(value: &Value) -> String {
    if let Some(number) = value.as_i64() {
        number.to_string()
    } else if let Some(number) = value.as_u64() {
        number.to_string()
    } else if let Some(number) = value.as_f64() {
        number.to_string()
    } else if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    }
}

fn json_quote(value: &str) -> String {
    match serde_json::to_string(value) {
        Ok(quoted) => quoted,
        Err(_err) => "\"\"".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RenderOptions, SnapshotMode, SnapshotOptions, apply_snapshot_options, render_snapshot,
    };
    use crate::snapshot::{AxNode, AxProperty, AxValue, refs::RefMap};
    use serde_json::json;
    use std::collections::HashMap;

    fn ax(node_id: &str, role: &str, children: &[&str]) -> AxNode {
        AxNode {
            node_id: node_id.to_string(),
            role: Some(AxValue::role(role)),
            child_ids: (!children.is_empty())
                .then(|| children.iter().map(|child| (*child).to_string()).collect()),
            ..AxNode::default()
        }
    }

    fn name(value: &str) -> AxValue {
        AxValue::string(value)
    }

    fn render(nodes: &[AxNode], refs: &mut RefMap) -> String {
        let mut opts = RenderOptions {
            refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 0,
        };
        render_snapshot(nodes, &mut opts).text
    }

    fn render_with_snapshot_options(
        nodes: &[AxNode],
        refs: &mut RefMap,
        options: SnapshotOptions,
    ) -> String {
        apply_snapshot_options(&render(nodes, refs), options)
    }

    #[test]
    fn renders_semantic_tree_with_refs_only_on_actionable_nodes() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "6"]),
            ax("2", "navigation", &["3"]),
            ax("3", "link", &[]),
            ax("6", "main", &["7", "8", "9"]),
            ax("7", "heading", &[]),
            ax("8", "button", &[]),
            ax("9", "generic", &["10"]),
            ax("10", "textbox", &[]),
        ];
        nodes[2].name = Some(name("Home"));
        nodes[2].backend_dom_node_id = Some(101);
        nodes[4].name = Some(name("Results"));
        nodes[4].backend_dom_node_id = Some(110);
        nodes[4].properties = Some(vec![AxProperty {
            name: "level".to_string(),
            value: AxValue {
                value_type: "integer".to_string(),
                value: Some(json!(1)),
            },
        }]);
        nodes[5].name = Some(name("Load more"));
        nodes[5].backend_dom_node_id = Some(111);
        nodes[5].properties = Some(vec![AxProperty {
            name: "disabled".to_string(),
            value: AxValue {
                value_type: "boolean".to_string(),
                value: Some(json!(true)),
            },
        }]);
        nodes[7].name = Some(name("Search"));
        nodes[7].backend_dom_node_id = Some(112);
        nodes[7].value = Some(AxValue {
            value_type: "string".to_string(),
            value: Some(json!("abc")),
        });

        let text = render(&nodes, &mut RefMap::new());
        assert_eq!(
            text,
            [
                "- navigation",
                "  - link \"Home\" [ref=e1]",
                "- main",
                "  - heading \"Results\" [level=1]",
                "  - button \"Load more\" [disabled] [ref=e2]",
                "  - textbox \"Search\" [ref=e3]: \"abc\"",
            ]
            .join("\n")
        );
    }

    #[test]
    fn drops_unnamed_generic_wrappers_lifting_children() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "generic", &["3"]),
            ax("3", "button", &[]),
        ];
        nodes[2].name = Some(name("Go"));
        nodes[2].backend_dom_node_id = Some(5);
        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            "- button \"Go\" [ref=e1]"
        );
    }

    #[test]
    fn mints_distinct_nth_for_duplicate_role_name() {
        let mut refs = RefMap::new();
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "button", &[]),
        ];
        nodes[1].name = Some(name("OK"));
        nodes[1].backend_dom_node_id = Some(1);
        nodes[2].name = Some(name("OK"));
        nodes[2].backend_dom_node_id = Some(2);
        let _ = render(&nodes, &mut refs);
        assert_eq!(
            refs.get(&crate::Ref("e1".to_string()))
                .map(|entry| entry.nth),
            Some(0)
        );
        assert_eq!(
            refs.get(&crate::Ref("e2".to_string()))
                .map(|entry| entry.nth),
            Some(1)
        );
    }

    #[test]
    fn reuses_refs_for_same_document_backend_nodes_after_insertion() {
        let mut refs = RefMap::new();
        let mut before = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "link", &[]),
        ];
        before[1].name = Some(name("A"));
        before[1].backend_dom_node_id = Some(1);
        before[2].name = Some(name("B"));
        before[2].backend_dom_node_id = Some(2);
        let mut after = vec![
            ax("1", "RootWebArea", &["4", "2", "3"]),
            ax("4", "button", &[]),
            ax("2", "button", &[]),
            ax("3", "link", &[]),
        ];
        after[1].name = Some(name("X"));
        after[1].backend_dom_node_id = Some(3);
        after[2].name = Some(name("A"));
        after[2].backend_dom_node_id = Some(1);
        after[3].name = Some(name("B"));
        after[3].backend_dom_node_id = Some(2);

        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: Some("main:loader-1".to_string()),
            cursor_hits: None,
            base_depth: 0,
        };
        let first = render_snapshot(&before, &mut opts).text;
        refs.begin_snapshot();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: Some("main:loader-1".to_string()),
            cursor_hits: None,
            base_depth: 0,
        };
        let second = render_snapshot(&after, &mut opts).text;

        assert_eq!(first, "- button \"A\" [ref=e1]\n- link \"B\" [ref=e2]");
        assert_eq!(
            second,
            [
                "- button \"X\" [ref=e3]",
                "- button \"A\" [ref=e1]",
                "- link \"B\" [ref=e2]"
            ]
            .join("\n")
        );
    }

    #[test]
    fn marks_cursor_augmented_non_aria_nodes_actionable() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "generic", &[])];
        nodes[1].name = Some(name("Fake button"));
        nodes[1].backend_dom_node_id = Some(42);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(HashMap::from([(42, vec!["onclick".to_string()])])),
            base_depth: 0,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "- generic \"Fake button\" [ref=e1] [cursor=pointer]"
        );
    }

    #[test]
    fn escapes_quotes_in_names() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "button", &[])];
        nodes[1].name = Some(name("Say \"hi\""));
        nodes[1].backend_dom_node_id = Some(1);
        assert_eq!(
            render(&nodes, &mut RefMap::new()),
            "- button \"Say \\\"hi\\\"\" [ref=e1]"
        );
    }

    #[test]
    fn reports_iframe_nodes_as_stitch_points() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2", "3"]),
            ax("2", "button", &[]),
            ax("3", "Iframe", &[]),
        ];
        nodes[1].name = Some(name("Outer"));
        nodes[1].backend_dom_node_id = Some(7);
        nodes[2].backend_dom_node_id = Some(8);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 0,
        };
        let out = render_snapshot(&nodes, &mut opts);
        assert_eq!(out.text, "- button \"Outer\" [ref=e1]\n- iframe");
        assert_eq!(out.iframes[0].backend_node_id, 8);
        assert_eq!(out.iframes[0].line_index, 1);
        assert_eq!(out.iframes[0].depth, 0);
    }

    #[test]
    fn honours_base_depth_for_spliced_child_frames() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "button", &[])];
        nodes[1].name = Some(name("Inner"));
        nodes[1].backend_dom_node_id = Some(1);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: None,
            base_depth: 2,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "    - button \"Inner\" [ref=e1]"
        );
    }

    #[test]
    fn keeps_nameless_generic_when_cursor_interactive() {
        let mut nodes = vec![ax("1", "RootWebArea", &["2"]), ax("2", "generic", &[])];
        nodes[1].backend_dom_node_id = Some(9);
        let mut refs = RefMap::new();
        let mut opts = RenderOptions {
            refs: &mut refs,
            frame_id: None,
            document_id: None,
            cursor_hits: Some(HashMap::from([(9, vec!["cursor:pointer".to_string()])])),
            base_depth: 0,
        };
        assert_eq!(
            render_snapshot(&nodes, &mut opts).text,
            "- generic [ref=e1] [cursor=pointer]"
        );
    }

    #[test]
    fn interactive_mode_keeps_refs_headings_first_line_and_ancestors() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4", "5"]),
            ax("3", "paragraph", &[]),
            ax("4", "section", &["6"]),
            ax("5", "heading", &[]),
            ax("6", "button", &[]),
        ];
        nodes[1].name = Some(name("App"));
        nodes[2].name = Some(name("Intro copy"));
        nodes[3].name = Some(name("Actions"));
        nodes[4].name = Some(name("Results"));
        nodes[5].name = Some(name("Save"));
        nodes[5].backend_dom_node_id = Some(10);

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            },
        );

        assert_eq!(
            text,
            [
                "- main \"App\"",
                "  - section \"Actions\"",
                "    - button \"Save\" [ref=e1]",
                "  - heading \"Results\"",
            ]
            .join("\n")
        );
    }

    #[test]
    fn depth_cap_drops_lines_nested_deeper_than_limit() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3"]),
            ax("3", "section", &["4"]),
            ax("4", "button", &[]),
        ];
        nodes[3].name = Some(name("Deep"));
        nodes[3].backend_dom_node_id = Some(10);

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Full,
                depth: Some(1),
            },
        );

        assert_eq!(text, ["- main", "  - section"].join("\n"));
    }

    #[test]
    fn interactive_filter_runs_before_depth_cap() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "5"]),
            ax("3", "section", &["4"]),
            ax("4", "button", &[]),
            ax("5", "paragraph", &[]),
        ];
        nodes[3].name = Some(name("Deep"));
        nodes[3].backend_dom_node_id = Some(10);
        nodes[4].name = Some(name("Static"));

        let text = render_with_snapshot_options(
            &nodes,
            &mut RefMap::new(),
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: Some(1),
            },
        );

        assert_eq!(text, ["- main", "  - section"].join("\n"));
    }

    #[test]
    fn refs_are_identical_across_full_and_interactive_modes() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4", "5"]),
            ax("3", "paragraph", &[]),
            ax("4", "button", &[]),
            ax("5", "link", &[]),
        ];
        nodes[2].name = Some(name("Static"));
        nodes[3].name = Some(name("Save"));
        nodes[3].backend_dom_node_id = Some(10);
        nodes[4].name = Some(name("Home"));
        nodes[4].backend_dom_node_id = Some(11);

        let mut full_refs = RefMap::new();
        let full = render_with_snapshot_options(
            &nodes,
            &mut full_refs,
            SnapshotOptions {
                mode: SnapshotMode::Full,
                depth: None,
            },
        );
        let mut interactive_refs = RefMap::new();
        let interactive = render_with_snapshot_options(
            &nodes,
            &mut interactive_refs,
            SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            },
        );

        assert_eq!(
            full_refs
                .entries_in_order()
                .into_iter()
                .map(|entry| (entry.ref_id.as_str().to_string(), entry.backend_node_id))
                .collect::<Vec<_>>(),
            interactive_refs
                .entries_in_order()
                .into_iter()
                .map(|entry| (entry.ref_id.as_str().to_string(), entry.backend_node_id))
                .collect::<Vec<_>>()
        );
        assert!(full.contains("paragraph \"Static\""));
        assert!(!interactive.contains("paragraph \"Static\""));
        assert!(interactive.contains("button \"Save\" [ref=e1]"));
        assert!(interactive.contains("link \"Home\" [ref=e2]"));
    }

    #[test]
    fn default_snapshot_options_preserve_full_output_byte_for_byte() {
        let mut nodes = vec![
            ax("1", "RootWebArea", &["2"]),
            ax("2", "main", &["3", "4"]),
            ax("3", "paragraph", &[]),
            ax("4", "button", &[]),
        ];
        nodes[2].name = Some(name("Static"));
        nodes[3].name = Some(name("Save"));
        nodes[3].backend_dom_node_id = Some(10);

        let mut refs = RefMap::new();
        let full = render(&nodes, &mut refs);
        let with_defaults = apply_snapshot_options(&full, SnapshotOptions::default());

        assert_eq!(with_defaults, full);
    }
}
