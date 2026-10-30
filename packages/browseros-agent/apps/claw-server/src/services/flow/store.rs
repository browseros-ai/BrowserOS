//! flow.md: the verified flow beside a skill's SKILL.md. YAML frontmatter (each value written
//! as JSON, which YAML reads as a flow scalar), then one `### step N` heading and fenced `js`
//! block per prose step. The step the flow must not repeat is headed `### step N (commit)`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

pub const FLOW_FILE: &str = "flow.md";
/// Upper bound on one step's source, so a submitted flow cannot fill the disk.
pub const MAX_STEP_BYTES: usize = 16 * 1024;
/// Examples kept per flow: the original run plus the newest others. Bounds verification time.
pub const MAX_EXAMPLES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effects {
    Read,
    Write,
}

impl Effects {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Example {
    pub input: Map<String, Value>,
    /// `original` (the run the skill was saved from), `agent` (picked when saving) or
    /// `handover` (an input that once made the flow hand back).
    pub source: String,
    /// `done`, or `either` when a clean hand-back is also an acceptable answer.
    pub expect: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowMeta {
    pub name: String,
    pub flow_version: i64,
    pub effects: Effects,
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Input name to its type: `string`, `number` or `boolean`.
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub output: BTreeMap<String, String>,
    #[serde(default)]
    pub examples: Vec<Example>,
    #[serde(default)]
    pub source_session: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowStep {
    pub n: u32,
    pub commit: bool,
    pub src: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowDoc {
    pub meta: FlowMeta,
    pub steps: Vec<FlowStep>,
}

impl FlowDoc {
    #[must_use]
    pub fn commit_step(&self) -> Option<u32> {
        self.steps
            .iter()
            .find(|step| step.commit)
            .map(|step| step.n)
    }

    #[must_use]
    pub fn step(&self, n: u32) -> Option<&FlowStep> {
        self.steps.iter().find(|step| step.n == n)
    }

    /// Replaces one step's source, keeping its number and whether it is the commit step.
    pub fn patch_step(&mut self, n: u32, src: &str) -> Result<(), String> {
        let src = src.trim();
        if src.is_empty() {
            return Err(format!("the patch for step {n} is empty"));
        }
        let step = self
            .steps
            .iter_mut()
            .find(|step| step.n == n)
            .ok_or_else(|| format!("the flow has no step {n}"))?;
        step.src = src.to_string();
        Ok(())
    }

    /// Adds an example, keeping the original and the newest others within the cap. An input
    /// already listed is not added twice.
    pub fn push_example(&mut self, example: Example) {
        if self
            .meta
            .examples
            .iter()
            .any(|existing| existing.input == example.input)
        {
            return;
        }
        self.meta.examples.push(example);
        while self.meta.examples.len() > MAX_EXAMPLES {
            let Some(index) = self
                .meta
                .examples
                .iter()
                .position(|existing| existing.source != "original")
            else {
                break;
            };
            self.meta.examples.remove(index);
        }
    }
}

#[must_use]
pub fn flow_path(skill_dir: &Path) -> PathBuf {
    skill_dir.join(FLOW_FILE)
}

fn json_scalar(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[must_use]
pub fn render(doc: &FlowDoc) -> String {
    let meta = &doc.meta;
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", json_scalar(&meta.name)));
    out.push_str(&format!("flowVersion: {}\n", meta.flow_version));
    out.push_str(&format!("effects: {}\n", meta.effects.as_str()));
    out.push_str(&format!("hosts: {}\n", json_scalar(&meta.hosts)));
    out.push_str(&format!("inputs: {}\n", json_scalar(&meta.inputs)));
    out.push_str(&format!("output: {}\n", json_scalar(&meta.output)));
    if meta.examples.is_empty() {
        out.push_str("examples: []\n");
    } else {
        out.push_str("examples:\n");
        for example in &meta.examples {
            out.push_str(&format!("  - {}\n", json_scalar(example)));
        }
    }
    if let Some(session) = &meta.source_session {
        out.push_str(&format!("sourceSession: {}\n", json_scalar(session)));
    }
    out.push_str("---\n");
    for step in &doc.steps {
        let marker = if step.commit { " (commit)" } else { "" };
        let fence = fence_for(&step.src);
        out.push_str(&format!(
            "\n### step {}{marker}\n\n{fence}js\n{}\n{fence}\n",
            step.n,
            step.src.trim_end()
        ));
    }
    out
}

/// A fence longer than any backtick run in the source, so a template literal never closes it.
fn fence_for(src: &str) -> String {
    let longest = src.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// Parses a flow file. Steps come back in step order.
pub fn parse(content: &str) -> Result<FlowDoc, String> {
    use pulldown_cmark::{CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(content, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut frontmatter = String::new();
    let mut heading = String::new();
    let mut code = String::new();
    let (mut in_meta, mut in_heading, mut in_js) = (false, false, false);
    let mut current: Option<(u32, bool)> = None;
    let mut steps = Vec::new();
    for event in parser {
        match event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => in_meta = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_meta = false,
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                current = parse_step_heading(&heading);
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
                if matches!(lang.split_whitespace().next(), Some("js" | "javascript")) =>
            {
                in_js = true;
                code.clear();
            }
            Event::End(TagEnd::CodeBlock) if in_js => {
                in_js = false;
                if let Some((n, commit)) = current.take() {
                    steps.push(FlowStep {
                        n,
                        commit,
                        src: code.trim().to_string(),
                    });
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if in_meta {
                    frontmatter.push_str(&text);
                } else if in_heading {
                    heading.push_str(&text);
                } else if in_js {
                    code.push_str(&text);
                }
            }
            _ => {}
        }
    }
    let meta = serde_saphyr::from_str::<FlowMeta>(&frontmatter)
        .map_err(|error| format!("flow.md frontmatter: {error}"))?;
    steps.sort_by_key(|step| step.n);
    Ok(FlowDoc { meta, steps })
}

fn parse_step_heading(text: &str) -> Option<(u32, bool)> {
    let rest = text.trim().strip_prefix("step ")?;
    let (number, tail) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(number, tail)| (number, tail));
    let n = number.parse().ok()?;
    Some((n, tail.trim() == "(commit)"))
}

pub fn read(skill_dir: &Path) -> io::Result<Option<FlowDoc>> {
    match std::fs::read_to_string(flow_path(skill_dir)) {
        Ok(content) => parse(&content)
            .map(Some)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes through a temp file and rename, so a reader never sees half a flow.
pub fn write(skill_dir: &Path, doc: &FlowDoc) -> io::Result<()> {
    std::fs::create_dir_all(skill_dir)?;
    let path = flow_path(skill_dir);
    let tmp = skill_dir.join(format!(".{FLOW_FILE}.tmp"));
    std::fs::write(&tmp, render(doc))?;
    std::fs::rename(tmp, path)
}

pub fn remove(skill_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(flow_path(skill_dir)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> FlowDoc {
        let input = |q: &str| {
            json!({ "query": q })
                .as_object()
                .cloned()
                .unwrap_or_default()
        };
        FlowDoc {
            meta: FlowMeta {
                name: "neo-shop-add".to_string(),
                flow_version: 3,
                effects: Effects::Write,
                hosts: vec!["127.0.0.1:4455".to_string()],
                inputs: BTreeMap::from([("query".to_string(), "string".to_string())]),
                output: BTreeMap::from([("added".to_string(), "string".to_string())]),
                examples: vec![Example {
                    input: input("sensodyne: \"450g\""),
                    source: "original".to_string(),
                    expect: "done".to_string(),
                }],
                source_session: Some("01J0".to_string()),
            },
            steps: vec![
                FlowStep {
                    n: 1,
                    commit: false,
                    src: "async ({ browser, inputs, state }) => {\n  state.page = await browser.open(`http://x/s?k=${inputs.query}`)\n}".to_string(),
                },
                FlowStep {
                    n: 2,
                    commit: true,
                    src: "async ({ state }) => {\n```\n  await state.page.click(state.ref)\n}"
                        .to_string(),
                },
            ],
        }
    }

    #[test]
    fn a_flow_round_trips_through_markdown() -> anyhow::Result<()> {
        let doc = sample();
        let parsed = parse(&render(&doc)).map_err(anyhow::Error::msg)?;
        assert_eq!(parsed, doc);
        assert_eq!(parsed.commit_step(), Some(2));
        Ok(())
    }

    #[test]
    fn examples_keep_the_original_and_the_newest() {
        let mut doc = sample();
        for i in 0..12 {
            doc.push_example(Example {
                input: json!({ "query": format!("q{i}") })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
                source: "handover".to_string(),
                expect: "either".to_string(),
            });
        }
        assert_eq!(doc.meta.examples.len(), MAX_EXAMPLES);
        assert_eq!(doc.meta.examples[0].source, "original");
        assert_eq!(doc.meta.examples[MAX_EXAMPLES - 1].input["query"], "q11");
        let before = doc.meta.examples.len();
        doc.push_example(doc.meta.examples[3].clone());
        assert_eq!(doc.meta.examples.len(), before);
    }

    #[test]
    fn a_patch_swaps_one_step_and_keeps_the_rest() -> anyhow::Result<()> {
        let mut doc = sample();
        doc.patch_step(
            2,
            "async ({ state }) => { await state.page.click(state.other) }",
        )
        .map_err(anyhow::Error::msg)?;
        assert!(doc.steps[1].src.contains("state.other"));
        assert!(doc.steps[1].commit);
        assert_eq!(doc.steps[0], sample().steps[0]);
        assert!(doc.patch_step(9, "x").is_err());
        assert!(doc.patch_step(1, "  ").is_err());
        Ok(())
    }

    #[test]
    fn files_round_trip_on_disk() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(read(dir.path())?.is_none());
        write(dir.path(), &sample())?;
        assert_eq!(read(dir.path())?, Some(sample()));
        remove(dir.path())?;
        assert!(read(dir.path())?.is_none());
        Ok(())
    }
}
