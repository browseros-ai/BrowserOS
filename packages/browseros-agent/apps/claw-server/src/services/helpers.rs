//! Persistence seam for code-mode helpers: a reusable flow a script saves for a
//! host and a later script loads by name. Each helper is a self-documenting
//! Markdown file at `<browserclaw_dir>/helpers/<host>/<name>.md`: YAML
//! frontmatter (provenance, freshness, call shape, inputs), a human-and-agent
//! readable body, and a fenced `js` block holding the function source. The host
//! extracts that source for the runtime, so the engine still hot-loads a plain
//! function expression; all Markdown handling stays here.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

const HELPERS_DIR: &str = "helpers";
const HELPER_EXTENSION: &str = "md";

/// Upper bound on a helper's source so a script cannot fill the disk. Bounds the
/// JS source itself, not the rendered file, so the limit is stable as the
/// surrounding Markdown grows.
pub const MAX_HELPER_BYTES: usize = 64 * 1024;

/// Provenance, freshness, and call shape a helper file carries in its
/// frontmatter, so a later reader (human or agent) can judge staleness and
/// origin and call it correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperMeta {
    pub name: String,
    pub host: String,
    #[serde(rename = "lastVerified")]
    pub last_verified: i64,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub candidate: bool,
    /// Whether the macro opens its own page (`(browser, inputs)`, returns a page)
    /// versus acting on a page passed to it (`(browser, page, inputs)`).
    #[serde(rename = "opensPage", default)]
    pub opens_page: bool,
    /// Input field name to a short human description, e.g. `field0 -> "search
    /// query"`. Empty when the helper takes no inputs.
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// One-line human summary of what the helper does.
    #[serde(default)]
    pub description: String,
    /// The conversation/session id that produced this candidate, so distillation
    /// can cap a single session to one candidate per host (empty for hand-saved).
    #[serde(default)]
    pub session: String,
}

/// A single safe path segment: non-empty, not a traversal token, limited to an
/// unsurprising character set so a host or helper name cannot escape the
/// helpers root.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Resolves `<browserclaw_dir>/helpers/<host>/`, or `None` for an unsafe host.
#[must_use]
pub fn helpers_dir(browserclaw_dir: &Path, host: &str) -> Option<PathBuf> {
    is_safe_segment(host).then(|| browserclaw_dir.join(HELPERS_DIR).join(host))
}

fn helper_path(browserclaw_dir: &Path, host: &str, name: &str) -> Option<PathBuf> {
    if !is_safe_segment(name) {
        return None;
    }
    helpers_dir(browserclaw_dir, host).map(|dir| dir.join(format!("{name}.{HELPER_EXTENSION}")))
}

/// Lists helper base names (without the `.md` extension) available for a host,
/// sorted. Missing directory or unsafe host yields an empty list.
#[must_use]
pub fn list_helpers(browserclaw_dir: &Path, host: &str) -> Vec<String> {
    let Some(dir) = helpers_dir(browserclaw_dir, host) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(HELPER_EXTENSION))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Reads a helper's raw Markdown file, or `None` for an unsafe host/name or a
/// missing file. This is the full self-documenting doc a reader sees.
#[must_use]
pub fn read_helper(browserclaw_dir: &Path, host: &str, name: &str) -> Option<String> {
    let path = helper_path(browserclaw_dir, host, name)?;
    fs::read_to_string(path).ok()
}

/// Writes a helper file verbatim, creating the host directory. Errors on an
/// unsafe host or name.
pub fn write_helper(
    browserclaw_dir: &Path,
    host: &str,
    name: &str,
    content: &str,
) -> io::Result<()> {
    let path = helper_path(browserclaw_dir, host, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe helper host or name"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Derives the helper host bucket from a page URL: the hostname minus a leading
/// `www.`, or `None` when there is no usable host. Subdomains stay distinct so
/// subdomain-scoped apps do not collide. No URL crate: bounded to http(s) shapes.
#[must_use]
pub fn host_bucket(url: &str) -> Option<String> {
    let (scheme, after_scheme) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host_port.split(':').next().unwrap_or("");
    let bucket = host.strip_prefix("www.").unwrap_or(host);
    is_safe_segment(bucket).then(|| bucket.to_string())
}

/// The exact call form for a helper, derived from its shape and inputs, so the
/// discovery note, the file body, and `readHelper` never drift. Bracket access
/// works for any name; the argument object names each input.
#[must_use]
pub fn call_example(meta: &HelperMeta) -> String {
    let name = &meta.name;
    let inputs_obj = (!meta.inputs.is_empty()).then(|| {
        let fields: Vec<String> = meta
            .inputs
            .iter()
            .map(|(field, desc)| format!("{field}: {}", input_placeholder(desc)))
            .collect();
        format!("{{ {} }}", fields.join(", "))
    });
    match (meta.opens_page, inputs_obj) {
        (true, Some(obj)) => format!("helpers[\"{name}\"](browser, {obj})"),
        (true, None) => format!("helpers[\"{name}\"](browser)"),
        (false, Some(obj)) => format!("helpers[\"{name}\"](browser, page, {obj})"),
        (false, None) => format!("helpers[\"{name}\"](browser, page)"),
    }
}

/// A copy-paste placeholder value for an input, tagged with its description so a
/// reader knows what to fill in.
fn input_placeholder(desc: &str) -> String {
    if desc.is_empty() {
        "\"...\"".to_string()
    } else {
        format!("\"<{desc}>\"")
    }
}

/// Infers a hand-saved helper's call shape from its source: whether it opens its
/// own page (no `page` parameter) and which `inputs.field<n>` it reads. A
/// best-effort heuristic for the `saveHelper` path; the distiller sets these
/// directly from what it recorded.
#[must_use]
pub fn analyze_source(source: &str) -> (bool, BTreeMap<String, String>) {
    let params = param_list(source);
    let opens_page = !params.iter().any(|param| param == "page");
    let mut inputs = BTreeMap::new();
    let needle = "inputs.field";
    let mut from = 0;
    while let Some(pos) = source[from..].find(needle) {
        let start = from + pos + needle.len();
        let digits: String = source[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            inputs.insert(format!("field{digits}"), "input value".to_string());
        }
        from = start;
    }
    (opens_page, inputs)
}

/// The parameter identifiers of the first parenthesized list in a source (the
/// arrow function's params), stripped of default values.
fn param_list(source: &str) -> Vec<String> {
    let Some(open) = source.find('(') else {
        return Vec::new();
    };
    let rest = &source[open + 1..];
    let Some(close) = rest.find(')') else {
        return Vec::new();
    };
    rest[..close]
        .split(',')
        .map(|part| part.split('=').next().unwrap_or("").trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Quotes a string as a JSON scalar, which is a valid YAML double-quoted scalar,
/// so colons, quotes, and newlines survive without a YAML serializer.
fn yaml_str(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn render_frontmatter(meta: &HelperMeta) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", yaml_str(&meta.name)));
    out.push_str(&format!("host: {}\n", yaml_str(&meta.host)));
    out.push_str(&format!("lastVerified: {}\n", meta.last_verified));
    out.push_str(&format!("agent: {}\n", yaml_str(&meta.agent)));
    out.push_str(&format!("candidate: {}\n", meta.candidate));
    out.push_str(&format!("opensPage: {}\n", meta.opens_page));
    out.push_str(&format!("description: {}\n", yaml_str(&meta.description)));
    if !meta.session.is_empty() {
        out.push_str(&format!("session: {}\n", yaml_str(&meta.session)));
    }
    if meta.inputs.is_empty() {
        out.push_str("inputs: {}\n");
    } else {
        out.push_str("inputs:\n");
        for (field, desc) in &meta.inputs {
            out.push_str(&format!("  {}: {}\n", yaml_str(field), yaml_str(desc)));
        }
    }
    out.push_str("---\n");
    out
}

fn render_body(meta: &HelperMeta, source: &str) -> String {
    let mut out = String::new();
    if !meta.description.is_empty() {
        out.push_str(&meta.description);
        out.push_str("\n\n");
    }
    out.push_str("Call it:\n\n");
    out.push_str(&format!("`{}`\n\n", call_example(meta)));
    out.push_str("```js\n");
    out.push_str(source.trim_end());
    out.push_str("\n```\n");
    out
}

/// Renders a helper file: YAML frontmatter, a readable body, and the source in a
/// fenced `js` block.
#[must_use]
pub fn format_helper(meta: &HelperMeta, source: &str) -> String {
    format!(
        "{}\n{}",
        render_frontmatter(meta),
        render_body(meta, source)
    )
}

/// Whether a fence info string names JavaScript (first token `js`/`javascript`).
fn is_js_lang(lang: &str) -> bool {
    matches!(lang.split_whitespace().next(), Some("js" | "javascript"))
}

/// Splits a helper file into its parsed frontmatter (if any) and the source from
/// its first `js` fenced block, using a CommonMark parser so the extraction is
/// robust. A file with no frontmatter yields `None`; one with no `js` fence
/// yields an empty source, so a malformed helper simply does not hot-load.
#[must_use]
pub fn parse_helper(content: &str) -> (Option<HelperMeta>, String) {
    use pulldown_cmark::{CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(content, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut frontmatter = String::new();
    let mut source = String::new();
    let mut in_meta = false;
    let mut in_js = false;
    let mut js_captured = false;
    for event in parser {
        match event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => in_meta = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_meta = false,
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
                if !js_captured && is_js_lang(&lang) =>
            {
                in_js = true;
            }
            Event::End(TagEnd::CodeBlock) if in_js => {
                in_js = false;
                js_captured = true;
            }
            Event::Text(text) => {
                if in_meta {
                    frontmatter.push_str(&text);
                } else if in_js {
                    source.push_str(&text);
                }
            }
            _ => {}
        }
    }
    let meta = (!frontmatter.trim().is_empty())
        .then(|| serde_saphyr::from_str::<HelperMeta>(&frontmatter).ok())
        .flatten();
    (meta, source.trim().to_string())
}

/// Writes a helper as a Markdown doc. Rejects an oversized source.
pub fn save_helper(browserclaw_dir: &Path, meta: &HelperMeta, source: &str) -> io::Result<()> {
    if source.len() > MAX_HELPER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "helper source exceeds the size limit",
        ));
    }
    write_helper(
        browserclaw_dir,
        &meta.host,
        &meta.name,
        &format_helper(meta, source),
    )
}

/// Reads a helper's JS source from its `js` fenced block, ready to eval. `None`
/// for an unsafe host/name or a missing file; empty when the file has no source.
#[must_use]
pub fn read_helper_source(browserclaw_dir: &Path, host: &str, name: &str) -> Option<String> {
    read_helper(browserclaw_dir, host, name).map(|content| parse_helper(&content).1)
}

/// Cheap check for whether any helper exists at all, so the hot-load path can
/// skip a page scan when there is nothing to load.
#[must_use]
pub fn has_any_helpers(browserclaw_dir: &Path) -> bool {
    fs::read_dir(browserclaw_dir.join(HELPERS_DIR))
        .map(|mut entries| entries.any(|entry| entry.is_ok()))
        .unwrap_or(false)
}

/// Cap on auto-distilled candidate helpers kept per host, so reuse-driven
/// candidates do not accumulate unbounded. Small because the per-session cap and
/// shape deduping already keep candidate churn low.
pub const MAX_CANDIDATES_PER_HOST: usize = 5;

/// Deletes a helper file, if present. Used by the distiller to supersede a
/// session's earlier candidate. No-op for an unsafe host/name or a missing file.
pub fn remove_helper(browserclaw_dir: &Path, host: &str, name: &str) {
    if let Some(path) = helper_path(browserclaw_dir, host, name) {
        let _ = fs::remove_file(path);
    }
}

/// Evicts the oldest candidate helpers for a host beyond `keep`. Eligible files
/// are `candidate:true` passed-page helpers; the canonical `opensPage` search
/// helper is protected, as are promoted or hand-saved helpers.
pub fn prune_candidates(browserclaw_dir: &Path, host: &str, keep: usize) {
    let mut candidates: Vec<HelperMeta> = list_helper_meta(browserclaw_dir, host)
        .into_iter()
        .filter(|meta| meta.candidate && !meta.opens_page)
        .collect();
    if candidates.len() <= keep {
        return;
    }
    // Newest first by last-verified; evict the tail.
    candidates.sort_by_key(|meta| std::cmp::Reverse(meta.last_verified));
    for meta in candidates.into_iter().skip(keep) {
        if let Some(path) = helper_path(browserclaw_dir, host, &meta.name) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Lists helpers for a host with their parsed frontmatter, sorted by name. A file
/// missing frontmatter still lists, with default provenance.
#[must_use]
pub fn list_helper_meta(browserclaw_dir: &Path, host: &str) -> Vec<HelperMeta> {
    list_helpers(browserclaw_dir, host)
        .into_iter()
        .filter_map(|name| {
            let content = read_helper(browserclaw_dir, host, &name)?;
            let (meta, _) = parse_helper(&content);
            Some(meta.unwrap_or_else(|| HelperMeta {
                name,
                host: host.to_string(),
                last_verified: 0,
                agent: String::new(),
                candidate: false,
                opens_page: false,
                inputs: BTreeMap::new(),
                description: String::new(),
                session: String::new(),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn search_meta() -> HelperMeta {
        HelperMeta {
            name: "search-amazon".to_string(),
            host: "amazon.in".to_string(),
            last_verified: 1_784_887_201_128,
            agent: "codex".to_string(),
            candidate: true,
            opens_page: true,
            inputs: BTreeMap::from([("field0".to_string(), "search query".to_string())]),
            description: "Opens amazon.in search for a query".to_string(),
            session: "convo-1".to_string(),
        }
    }

    #[test]
    fn write_then_read_round_trips_and_lists() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        write_helper(root, "linkedin.com", "accept-invites", "one")?;
        write_helper(root, "linkedin.com", "messages", "two")?;

        assert_eq!(
            read_helper(root, "linkedin.com", "accept-invites").as_deref(),
            Some("one")
        );
        assert_eq!(
            list_helpers(root, "linkedin.com"),
            vec!["accept-invites".to_string(), "messages".to_string()]
        );
        // Distinct hosts do not collide.
        assert!(list_helpers(root, "docs.google.com").is_empty());
        Ok(())
    }

    #[test]
    fn unsafe_host_or_name_is_rejected() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        assert!(helpers_dir(root, "..").is_none());
        assert!(helpers_dir(root, "a/b").is_none());
        assert!(read_helper(root, "linkedin.com", "../escape").is_none());
        assert!(write_helper(root, "..", "x", "code").is_err());
        assert!(write_helper(root, "linkedin.com", "a/b", "code").is_err());
        Ok(())
    }

    #[test]
    fn missing_host_reads_and_lists_empty() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        assert!(read_helper(root, "linkedin.com", "nope").is_none());
        assert!(list_helpers(root, "linkedin.com").is_empty());
        Ok(())
    }

    #[test]
    fn host_bucket_strips_scheme_www_path_and_port() {
        assert_eq!(
            host_bucket("https://www.linkedin.com/feed").as_deref(),
            Some("linkedin.com")
        );
        assert_eq!(
            host_bucket("https://docs.google.com/document/1").as_deref(),
            Some("docs.google.com")
        );
        assert_eq!(
            host_bucket("http://localhost:3000/app").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            host_bucket("https://user@example.com:8443/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_bucket("about:blank"), None);
        assert_eq!(host_bucket("chrome://newtab"), None);
    }

    #[test]
    fn call_example_matches_shape_and_inputs() {
        // Open-page macro with an input: bracket access, inputs object, no page.
        assert_eq!(
            call_example(&search_meta()),
            "helpers[\"search-amazon\"](browser, { field0: \"<search query>\" })"
        );
        let mut passed = search_meta();
        passed.name = "reply".to_string();
        passed.opens_page = false;
        passed.inputs = BTreeMap::new();
        assert_eq!(call_example(&passed), "helpers[\"reply\"](browser, page)");
        passed.opens_page = true;
        assert_eq!(call_example(&passed), "helpers[\"reply\"](browser)");
    }

    #[test]
    fn analyze_source_infers_shape_and_inputs() {
        let (opens_page, inputs) = analyze_source(
            "async (browser, inputs = {}) => { return inputs.field0 + inputs.field1; }",
        );
        assert!(opens_page);
        assert_eq!(inputs.len(), 2);
        assert!(inputs.contains_key("field0"));
        assert!(inputs.contains_key("field1"));

        let (opens_page, inputs) = analyze_source(
            "async (browser, page, inputs = {}) => { await browser.input(page).fill('e', inputs.field0); }",
        );
        assert!(!opens_page);
        assert_eq!(inputs.len(), 1);

        let (opens_page, inputs) = analyze_source("async (browser, page) => { return 1; }");
        assert!(!opens_page);
        assert!(inputs.is_empty());
    }

    #[test]
    fn save_read_and_list_round_trip_frontmatter_and_extract_source() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let source = "async (browser, inputs = {}) => { return inputs.field0; }";
        save_helper(root, &search_meta(), source)?;

        // The eval body is the source lifted out of the js fence.
        assert_eq!(
            read_helper_source(root, "amazon.in", "search-amazon").as_deref(),
            Some(source)
        );
        // The raw doc a reader sees carries the description and call form.
        let doc = read_helper(root, "amazon.in", "search-amazon").unwrap_or_default();
        assert!(doc.contains("Opens amazon.in search for a query"));
        assert!(
            doc.contains("helpers[\"search-amazon\"](browser, { field0: \"<search query>\" })")
        );
        // Frontmatter round-trips through the parser.
        let listed = list_helper_meta(root, "amazon.in");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "search-amazon");
        assert_eq!(listed[0].last_verified, 1_784_887_201_128);
        assert!(listed[0].candidate);
        assert!(listed[0].opens_page);
        assert_eq!(listed[0].agent, "codex");
        assert_eq!(
            listed[0].inputs.get("field0").map(String::as_str),
            Some("search query")
        );
        assert_eq!(listed[0].description, "Opens amazon.in search for a query");
        assert_eq!(listed[0].session, "convo-1");
        Ok(())
    }

    #[test]
    fn prune_never_evicts_the_opens_page_search_helper() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        // The canonical opensPage search helper (a candidate) must survive a
        // burst of passed-page fragments.
        save_helper(
            root,
            &search_meta(),
            "async (browser, inputs = {}) => { return 1; }",
        )?;
        let fragment = |name: &str, verified: i64| HelperMeta {
            name: name.to_string(),
            host: "amazon.in".to_string(),
            last_verified: verified,
            agent: String::new(),
            candidate: true,
            opens_page: false,
            inputs: BTreeMap::new(),
            description: String::new(),
            session: String::new(),
        };
        for i in 1..=4 {
            save_helper(
                root,
                &fragment(&format!("candidate-{i}"), i),
                "async () => 1",
            )?;
        }
        prune_candidates(root, "amazon.in", 1);
        let names: Vec<String> = list_helper_meta(root, "amazon.in")
            .into_iter()
            .map(|meta| meta.name)
            .collect();
        assert!(
            names.contains(&"search-amazon".to_string()),
            "search helper evicted: {names:?}"
        );
        Ok(())
    }

    #[test]
    fn oversized_source_is_rejected() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let mut meta = search_meta();
        meta.name = "big".to_string();
        let huge = "a".repeat(MAX_HELPER_BYTES + 1);
        assert!(save_helper(dir.path(), &meta, &huge).is_err());
        Ok(())
    }

    #[test]
    fn prune_candidates_caps_candidates_and_keeps_promoted_helpers() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        let helper = |name: &str, verified: i64, candidate: bool| HelperMeta {
            name: name.to_string(),
            host: "h".to_string(),
            last_verified: verified,
            agent: String::new(),
            candidate,
            // Passed-page candidates are the evictable kind.
            opens_page: false,
            inputs: BTreeMap::new(),
            description: String::new(),
            session: String::new(),
        };
        // A promoted (candidate:false) helper must survive pruning.
        save_helper(root, &helper("keep-me", 500, false), "async () => 1")?;
        for i in 1..=3 {
            save_helper(
                root,
                &helper(&format!("candidate-{i}"), i, true),
                "async () => 1",
            )?;
        }
        prune_candidates(root, "h", 1); // keep only the newest candidate
        let names: Vec<String> = list_helper_meta(root, "h")
            .into_iter()
            .map(|meta| meta.name)
            .collect();
        assert!(names.contains(&"keep-me".to_string()));
        assert!(names.contains(&"candidate-3".to_string())); // newest
        assert!(!names.contains(&"candidate-1".to_string()));
        assert!(!names.contains(&"candidate-2".to_string()));
        Ok(())
    }

    #[test]
    fn has_any_helpers_gates_on_a_populated_helpers_root() -> anyhow::Result<()> {
        let dir = tempdir()?;
        assert!(!has_any_helpers(dir.path()));
        save_helper(dir.path(), &search_meta(), "async () => {}")?;
        assert!(has_any_helpers(dir.path()));
        Ok(())
    }

    #[test]
    fn a_file_without_frontmatter_lists_with_defaults_and_extracts_source() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let root = dir.path();
        write_helper(
            root,
            "example.com",
            "legacy",
            "no frontmatter here\n\n```js\nasync () => {}\n```\n",
        )?;
        let listed = list_helper_meta(root, "example.com");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "legacy");
        assert_eq!(listed[0].last_verified, 0);
        // The js fence is still extracted even without frontmatter.
        assert_eq!(
            read_helper_source(root, "example.com", "legacy").as_deref(),
            Some("async () => {}")
        );
        Ok(())
    }
}
