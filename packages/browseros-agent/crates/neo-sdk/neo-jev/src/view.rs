//! What Jev reads: the snapshot text parsed into candidates and twin groups. Pure.
//!
//! ```text
//! snapshot text ─▶ lines ─▶ twin groups ─▶ role filter ─▶ batches of ≤254 refs ─▶ Choice options
//!                    └──▶ rendered text (hidden copies dropped, nameless controls named) ─▶ state
//! ```
//!
//! A twin group is every ref that means the same element to a person: an `[aria-hidden]` copy and
//! its visible twin, a `generic [cursor=pointer]` wrapper and the control beside it with the same
//! text, links to the same href. Jev's probability splits across twins, so `gate` sums a group.
//! Options go out as bare refs: their labels are already in the state text.

use std::collections::HashMap;

/// Most refs one Choice may carry; Jev allows 255 options and one is `none`.
pub const MAX_BATCH_REFS: usize = 254;

/// How far (in ref'd lines) an `[aria-hidden]` copy looks for its visible twin.
const TWIN_REACH: usize = 8;

/// One parsed snapshot line: `- role "name" [attr] [ref=e12]: text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Indentation level (two spaces each); nesting in the accessibility tree.
    pub depth: usize,
    /// ARIA role, or `text` / `/placeholder` / `/url` for property lines.
    pub role: String,
    /// The accessible name, unquoted.
    pub name: Option<String>,
    /// The element ref, when the line carries one.
    pub r#ref: Option<String>,
    /// Bracketed attributes other than the ref (`aria-hidden`, `cursor=pointer`, `level=2`, ...).
    pub attrs: Vec<String>,
    /// The text after the colon (a value, a nameless link's URL), unquoted.
    pub text: Option<String>,
}

impl Line {
    fn has_attr(&self, attr: &str) -> bool {
        self.attrs.iter().any(|a| a == attr)
    }
}

/// One twin group: the refs a person would call the same element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The ref a verdict names: the visible control, not a wrapper or a hidden copy.
    pub rep: String,
    /// The refs offered to Jev, in page order (hidden copies left out).
    pub offered: Vec<String>,
    /// Every ref whose probability counts for this group, hidden copies included.
    pub members: Vec<String>,
    /// The representative's role.
    pub role: String,
    /// `role "name"` for the trail; a nameless control shows its borrowed name.
    pub label: String,
}

/// The page as Jev sees it, built once per call from the snapshot text.
#[derive(Debug, Clone)]
pub struct View {
    groups: Vec<Group>,
    group_of: HashMap<String, usize>,
    text: String,
    elements: usize,
    has_text: bool,
}

impl View {
    /// Parses `snapshot` and builds the twin groups and the text Jev reads.
    #[must_use]
    pub fn new(snapshot: &str) -> View {
        let raws: Vec<&str> = snapshot.lines().collect();
        let lines: Vec<Option<Line>> = raws.iter().map(|raw| parse_line(raw)).collect();
        let refs: Vec<usize> = (0..lines.len())
            .filter(|&i| {
                lines
                    .get(i)
                    .and_then(Option::as_ref)
                    .is_some_and(|l| l.r#ref.is_some())
            })
            .collect();
        let has_text = lines.iter().flatten().any(|l| l.role == "text");

        let borrowed: HashMap<usize, String> = refs
            .iter()
            .filter_map(|&i| borrowed_name(&lines, i).map(|name| (i, name)))
            .collect();

        // Union-find over positions in `refs`.
        let mut parent: Vec<usize> = (0..refs.len()).collect();
        let mut dropped = vec![false; lines.len()];
        let line_at = |k: usize| {
            refs.get(k)
                .and_then(|&i| lines.get(i))
                .and_then(Option::as_ref)
        };

        // Hidden copies: merge into the nearest visible twin and drop from the text.
        for k in 0..refs.len() {
            let Some(line) = line_at(k) else { continue };
            if !line.has_attr("aria-hidden") {
                continue;
            }
            let after = (k + 1..refs.len().min(k + 1 + TWIN_REACH)).map(Some);
            let before = (k.saturating_sub(TWIN_REACH)..k).rev().map(Some);
            let twin = after.chain(before).flatten().find(|&j| {
                line_at(j).is_some_and(|other| {
                    !other.has_attr("aria-hidden")
                        && other.role == line.role
                        && other.name == line.name
                        && other.text == line.text
                })
            });
            if let Some(j) = twin {
                union(&mut parent, j, k);
                if let Some(&i) = refs.get(k) {
                    drop_subtree(&lines, i, &mut dropped);
                }
            }
        }

        // Links to the same href (a nameless link shows its URL as text).
        let mut by_href: HashMap<&str, usize> = HashMap::new();
        for k in 0..refs.len() {
            let Some(line) = line_at(k) else { continue };
            if line.role != "link" {
                continue;
            }
            let Some(href) = line.text.as_deref().filter(|t| looks_like_href(t)) else {
                continue;
            };
            match by_href.get(href) {
                Some(&first) => union(&mut parent, first, k),
                None => {
                    by_href.insert(href, k);
                }
            }
        }

        // A clickable wrapper beside the control with the same text.
        let live: Vec<usize> = (0..refs.len())
            .filter(|&k| {
                refs.get(k)
                    .is_some_and(|&i| !dropped.get(i).copied().unwrap_or(false))
            })
            .collect();
        for (pos, &k) in live.iter().enumerate() {
            let Some(line) = line_at(k) else { continue };
            if line.role != "generic" || !line.has_attr("cursor=pointer") {
                continue;
            }
            let Some(own) = line.name.as_deref().or(line.text.as_deref()).map(normalize) else {
                continue;
            };
            if own.is_empty() {
                continue;
            }
            let neighbours = [pos.checked_sub(1), Some(pos + 1)];
            for &j in neighbours.iter().flatten().filter_map(|p| live.get(*p)) {
                let Some(other) = line_at(j) else { continue };
                let other_name = other
                    .name
                    .as_deref()
                    .or(other.text.as_deref())
                    .map(normalize);
                if other.role != "generic" && other_name.as_deref() == Some(own.as_str()) {
                    union(&mut parent, j, k);
                    break;
                }
            }
        }

        // Collect groups in page order of their first member.
        let mut groups: Vec<Group> = Vec::new();
        let mut group_of: HashMap<String, usize> = HashMap::new();
        let mut root_group: HashMap<usize, usize> = HashMap::new();
        for k in 0..refs.len() {
            let root = find(&mut parent, k);
            let Some(&i) = refs.get(k) else { continue };
            let Some(line) = line_at(k) else { continue };
            let Some(r) = line.r#ref.clone() else {
                continue;
            };
            let index = *root_group.entry(root).or_insert_with(|| {
                groups.push(Group {
                    rep: String::new(),
                    offered: Vec::new(),
                    members: Vec::new(),
                    role: String::new(),
                    label: String::new(),
                });
                groups.len() - 1
            });
            if let Some(group) = groups.get_mut(index) {
                group.members.push(r.clone());
                if !dropped.get(i).copied().unwrap_or(false) {
                    group.offered.push(r.clone());
                }
            }
            group_of.insert(r, index);
        }
        let by_ref: HashMap<&str, usize> = refs
            .iter()
            .filter_map(|&i| {
                let line = lines.get(i).and_then(Option::as_ref)?;
                Some((line.r#ref.as_deref()?, i))
            })
            .collect();
        for group in &mut groups {
            let pick_rep = |filter: &dyn Fn(&Line) -> bool| {
                group.offered.iter().find(|r| {
                    by_ref
                        .get(r.as_str())
                        .and_then(|&i| lines.get(i))
                        .and_then(Option::as_ref)
                        .is_some_and(filter)
                })
            };
            let rep = pick_rep(&|l: &Line| l.role != "generic")
                .or_else(|| group.offered.first())
                .or_else(|| group.members.first())
                .cloned()
                .unwrap_or_default();
            if let Some(&i) = by_ref.get(rep.as_str())
                && let Some(line) = lines.get(i).and_then(Option::as_ref)
            {
                group.role.clone_from(&line.role);
                group.label = label_of(line, borrowed.get(&i).map(String::as_str));
            }
            group.rep = rep;
        }

        let mut text = String::with_capacity(snapshot.len());
        for (i, raw) in raws.iter().enumerate() {
            if dropped.get(i).copied().unwrap_or(false) {
                continue;
            }
            match (borrowed.get(&i), lines.get(i).and_then(Option::as_ref)) {
                (Some(name), Some(line)) => text.push_str(&render(line, name)),
                _ => text.push_str(raw),
            }
            text.push('\n');
        }

        View {
            groups,
            group_of,
            text,
            elements: refs.len(),
            has_text,
        }
    }

    /// Every twin group in page order.
    #[must_use]
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The group a ref belongs to; `None` for a ref this snapshot never showed.
    #[must_use]
    pub fn group_of(&self, r: &str) -> Option<usize> {
        self.group_of.get(r).copied()
    }

    /// The text Jev reads: the snapshot with merged hidden copies dropped and nameless controls
    /// named.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Snapshot lines that carry a ref.
    #[must_use]
    pub fn elements(&self) -> usize {
        self.elements
    }

    /// The snapshot carried `text:` lines (the `{ text: true }` mode).
    #[must_use]
    pub fn has_text(&self) -> bool {
        self.has_text
    }

    /// Indices of the groups offered to Jev, page order; `role` keeps groups with a member of it.
    #[must_use]
    pub fn candidates(&self, role: Option<&str>) -> Vec<usize> {
        (0..self.groups.len())
            .filter(|&g| {
                self.groups.get(g).is_some_and(|group| {
                    !group.offered.is_empty() && role.is_none_or(|role| group.role == role)
                })
            })
            .collect()
    }

    /// Splits candidate groups into Choice batches of at most `MAX_BATCH_REFS` refs, never
    /// splitting a group, so a group's sum is always taken inside one batch.
    #[must_use]
    pub fn batches(&self, candidates: &[usize]) -> Vec<Vec<usize>> {
        let mut batches: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        let mut size = 0;
        for &g in candidates {
            let n = self.groups.get(g).map_or(0, |group| group.offered.len());
            if size + n > MAX_BATCH_REFS && !current.is_empty() {
                batches.push(std::mem::take(&mut current));
                size = 0;
            }
            current.push(g);
            size += n;
        }
        if !current.is_empty() {
            batches.push(current);
        }
        batches
    }

    /// The refs a batch offers, in page order.
    #[must_use]
    pub fn offered(&self, batch: &[usize]) -> Vec<&str> {
        batch
            .iter()
            .filter_map(|&g| self.groups.get(g))
            .flat_map(|group| group.offered.iter().map(String::as_str))
            .collect()
    }
}

/// Parses one snapshot line; `None` for a line that is not a `- entry`.
#[must_use]
pub fn parse_line(raw: &str) -> Option<Line> {
    let trimmed = raw.trim_start_matches(' ');
    let depth = (raw.len() - trimmed.len()) / 2;
    let entry = trimmed.strip_prefix("- ")?;
    let entry = unquote_single(entry);
    let mut rest = entry.as_str();

    let role_end = rest.find([' ', ':']).unwrap_or(rest.len());
    let (role, after) = rest.split_at(role_end);
    if role.is_empty() {
        return None;
    }
    rest = after;

    let mut name = None;
    if let Some(quoted) = rest.strip_prefix(" \"") {
        let (value, after) = read_quoted(quoted);
        name = Some(value);
        rest = after;
    }

    let mut attrs = Vec::new();
    let mut r#ref = None;
    while let Some(bracketed) = rest.strip_prefix(" [") {
        let Some(close) = bracketed.find(']') else {
            break;
        };
        let (attr, after) = bracketed.split_at(close);
        match attr.strip_prefix("ref=") {
            Some(r) => r#ref = Some(r.to_owned()),
            None => attrs.push(attr.to_owned()),
        }
        rest = after.get(1..).unwrap_or_default();
    }

    let text = rest
        .strip_prefix(':')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| match t.strip_prefix('"') {
            Some(quoted) => read_quoted(quoted).0,
            None => t.to_owned(),
        });

    Some(Line {
        depth,
        role: role.to_owned(),
        name,
        r#ref,
        attrs,
        text,
    })
}

/// A whole entry YAML-quoted in single quotes (`- 'link "a: b" [ref=e1]'`), unwrapped.
fn unquote_single(entry: &str) -> String {
    let Some(body) = entry.strip_prefix('\'') else {
        return entry.to_owned();
    };
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                out.extend(chars);
                break;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Reads a double-quoted string body up to its closing quote; returns it and the rest.
fn read_quoted(body: &str) -> (String, &str) {
    let mut out = String::new();
    let mut escaped = false;
    for (at, c) in body.char_indices() {
        if escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return (out, body.get(at + 1..).unwrap_or_default());
        } else {
            out.push(c);
        }
    }
    (out, "")
}

fn looks_like_href(text: &str) -> bool {
    text.starts_with('/') || text.starts_with("http://") || text.starts_with("https://")
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Roles whose nameless lines are containers, not controls: they never borrow a name.
const CONTAINERS: &[&str] = &[
    "banner",
    "navigation",
    "main",
    "contentinfo",
    "region",
    "search",
    "form",
    "dialog",
    "iframe",
    "list",
    "listitem",
    "complementary",
    "article",
    "generic",
    "group",
    "listbox",
    "tabpanel",
];

/// A nameless control's borrowed name: its placeholder, else the nearest heading after (links,
/// buttons: a card's title follows its image link) or the nearest label text before (inputs).
fn borrowed_name(lines: &[Option<Line>], at: usize) -> Option<String> {
    let line = lines.get(at)?.as_ref()?;
    if line.name.is_some() || CONTAINERS.contains(&line.role.as_str()) {
        return None;
    }
    // Children: the lines right after, one level deeper or more.
    let mut end = at + 1;
    while lines
        .get(end)
        .and_then(Option::as_ref)
        .is_some_and(|child| child.depth > line.depth)
    {
        end += 1;
    }
    let children = lines.get(at + 1..end).unwrap_or_default();
    if let Some(placeholder) = children
        .iter()
        .flatten()
        .find(|c| c.role == "/placeholder")
        .and_then(|c| c.text.clone())
    {
        return Some(placeholder);
    }
    let heading = |l: &Line| {
        (l.role == "heading")
            .then(|| l.name.clone().or_else(|| l.text.clone()))
            .flatten()
    };
    let label = |l: &Line| match l.role.as_str() {
        "text" => l.text.clone(),
        "heading" => l.name.clone().or_else(|| l.text.clone()),
        _ => None,
    };
    let after = || {
        lines
            .get(end..(end + 2).min(lines.len()))
            .unwrap_or_default()
            .iter()
            .flatten()
            .find_map(heading)
    };
    let before = || {
        lines
            .get(at.saturating_sub(3)..at)
            .unwrap_or_default()
            .iter()
            .rev()
            .flatten()
            .find_map(label)
    };
    if matches!(line.role.as_str(), "link" | "button") {
        after().or_else(before)
    } else {
        before().or_else(after)
    }
}

fn label_of(line: &Line, borrowed: Option<&str>) -> String {
    let name = line
        .name
        .as_deref()
        .or(borrowed)
        .or(line.text.as_deref())
        .unwrap_or_default();
    let name: String = name.chars().take(80).collect();
    if name.is_empty() {
        line.role.clone()
    } else {
        format!("{} {}", line.role, quote(&name))
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A nameless control re-rendered with its borrowed name, so Jev reads the name beside the ref.
fn render(line: &Line, name: &str) -> String {
    let mut out = format!("{}- {} {}", "  ".repeat(line.depth), line.role, quote(name));
    for attr in &line.attrs {
        out.push_str(&format!(" [{attr}]"));
    }
    if let Some(r) = &line.r#ref {
        out.push_str(&format!(" [ref={r}]"));
    }
    if let Some(text) = &line.text {
        out.push_str(": ");
        out.push_str(text);
    }
    out
}

fn drop_subtree(lines: &[Option<Line>], at: usize, dropped: &mut [bool]) {
    let Some(depth) = lines.get(at).and_then(Option::as_ref).map(|l| l.depth) else {
        return;
    };
    if let Some(slot) = dropped.get_mut(at) {
        *slot = true;
    }
    let mut i = at + 1;
    while let Some(child) = lines.get(i).and_then(Option::as_ref) {
        if child.depth <= depth {
            break;
        }
        if let Some(slot) = dropped.get_mut(i) {
            *slot = true;
        }
        i += 1;
    }
}

fn find(parent: &mut [usize], k: usize) -> usize {
    let mut root = k;
    while let Some(&up) = parent.get(root) {
        if up == root {
            break;
        }
        root = up;
    }
    let mut node = k;
    while let Some(slot) = parent.get_mut(node) {
        if *slot == root {
            break;
        }
        let next = *slot;
        *slot = root;
        node = next;
    }
    root
}

/// Joins two groups; the earlier position stays the root so groups keep page order.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    let (low, high) = if ra < rb { (ra, rb) } else { (rb, ra) };
    if let Some(slot) = parent.get_mut(high) {
        *slot = low;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_attrs_refs_and_text() {
        let line = parse_line(r#"    - button "Say \"hi\"" [pressed] [ref=e12]: ok"#);
        assert_eq!(
            line,
            Some(Line {
                depth: 2,
                role: "button".into(),
                name: Some("Say \"hi\"".into()),
                r#ref: Some("e12".into()),
                attrs: vec!["pressed".into()],
                text: Some("ok".into()),
            }),
            "quotes inside a name are escaped, so the name ends at the unescaped quote"
        );
        let wrapped = parse_line(r#"- 'link "₹350 ( a: b ) it''s" [ref=f5e774]'"#);
        assert_eq!(
            wrapped.and_then(|l| l.name),
            Some("₹350 ( a: b ) it's".into()),
            "YAML single-quoted entries unwrap, '' becoming '"
        );
        let text = parse_line("  - text: Loading results");
        assert_eq!(
            text.map(|l| (l.role, l.text)),
            Some(("text".into(), Some("Loading results".into()))),
            "a text line keeps its text"
        );
    }
}
