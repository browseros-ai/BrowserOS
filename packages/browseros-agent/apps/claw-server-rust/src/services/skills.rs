use crate::{
    db::{
        SkillsRepository,
        entities::{skill_runs, skills},
    },
    error::{AppError, AppResult, IoPath},
    services::harness::HarnessService,
};
use harness_integrations::{AgentId, SkillSpec};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

/// The embedded product skill owns this directory name; user skills may not
/// reuse it or they would clobber the managed BrowserOS skill.
const RESERVED_SKILL_NAMES: [&str; 1] = ["browserclaw"];

/// Every user- and agent-authored skill is namespaced under this prefix so its
/// on-disk and linked-agent directory can never collide with a user's own
/// skill, and so a user can list them all by typing `/neo` in their agent.
const NEO_SKILL_PREFIX: &str = "neo-";
const DEFAULT_RUN_LIMIT: u64 = 25;
const MAX_RUN_LIMIT: u64 = 100;

/// Where the agent gets told to drive the browser from a skill's frontmatter.
const SKILL_TOOLS: &str = "browseros-neo";

/// Skill creation surface, independent of transport. The MCP tool and the REST
/// handler both build this.
pub struct CreateSkill {
    pub name: String,
    pub description: String,
    pub site: Option<String>,
    pub steps: Vec<String>,
    pub learned_notes: Vec<String>,
    pub origin: SkillOrigin,
    pub source_session_id: Option<String>,
}

pub struct UpdateSkill {
    pub description: Option<String>,
    pub site: Option<String>,
    pub steps: Option<Vec<String>>,
    pub learned_notes: Option<Vec<String>>,
    pub body: Option<String>,
}

#[derive(Clone, Copy)]
pub enum SkillOrigin {
    Agent,
    Manual,
    Directory,
}

impl SkillOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Manual => "manual",
            Self::Directory => "directory",
        }
    }
}

/// Aggregated run stats for the list and detail views.
#[derive(Default)]
pub struct RunStats {
    pub run_count: i64,
    pub clean_run_count: i64,
    pub first_run_tokens: Option<i64>,
    pub latest_run_tokens: Option<i64>,
    pub last_run_at: Option<i64>,
}

pub struct SkillView {
    pub model: skills::Model,
    pub linked_agents: Vec<AgentId>,
    pub stats: RunStats,
}

pub struct SkillDetailView {
    pub view: SkillView,
    pub body: String,
    pub runs: Vec<skill_runs::Model>,
}

/// Owns user skills end to end: persists rows, keeps the canonical SKILL.md on
/// disk, and links each skill into the connected agents through the harness.
pub struct SkillService {
    repo: SkillsRepository,
    harness: Arc<HarnessService>,
    skills_dir: PathBuf,
    /// Serializes mutations so the disk, the harness, and the row stay
    /// consistent, and so a get-then-create decision cannot race a
    /// concurrent create of the same new name.
    mutate: Mutex<()>,
}

impl SkillService {
    #[must_use]
    pub fn new(repo: SkillsRepository, harness: Arc<HarnessService>, skills_dir: PathBuf) -> Self {
        Self {
            repo,
            harness,
            skills_dir,
            mutate: Mutex::new(()),
        }
    }

    pub async fn list(&self) -> AppResult<Vec<SkillView>> {
        let models = self.repo.all().await?;
        let runs = self.repo.all_runs().await?;
        let mut by_skill: HashMap<String, Vec<skill_runs::Model>> = HashMap::new();
        for run in runs {
            by_skill
                .entry(run.skill_name.clone())
                .or_default()
                .push(run);
        }
        Ok(models
            .into_iter()
            .map(|model| {
                let skill_runs = by_skill.remove(&model.name).unwrap_or_default();
                let stats = stats_from_runs(&skill_runs);
                let linked_agents = parse_linked_agents(&model.linked_agents_json);
                SkillView {
                    model,
                    linked_agents,
                    stats,
                }
            })
            .collect())
    }

    pub async fn get(&self, name: &str) -> AppResult<SkillDetailView> {
        let model = self.require(name).await?;
        let body = self.read_body(&model.body_path).await;
        let runs = self.repo.runs_for(name).await?;
        let stats = stats_from_runs(&runs);
        let linked_agents = parse_linked_agents(&model.linked_agents_json);
        Ok(SkillDetailView {
            view: SkillView {
                model,
                linked_agents,
                stats,
            },
            body,
            runs,
        })
    }

    pub async fn list_runs(
        &self,
        name: &str,
        cursor: Option<i64>,
        limit: Option<u64>,
    ) -> AppResult<(Vec<skill_runs::Model>, Option<i64>)> {
        self.require(name).await?;
        let limit = limit.unwrap_or(DEFAULT_RUN_LIMIT).clamp(1, MAX_RUN_LIMIT);
        self.repo.list_runs(name, cursor, limit).await
    }

    /// Every recorded run across all skills, so the list handler can attach each
    /// skill's token savings without a per-skill query.
    pub async fn all_runs(&self) -> AppResult<Vec<skill_runs::Model>> {
        self.repo.all_runs().await
    }

    pub async fn create(&self, input: CreateSkill) -> AppResult<SkillView> {
        let _guard = self.mutate.lock().await;
        self.create_locked(input).await
    }

    async fn create_locked(&self, input: CreateSkill) -> AppResult<SkillView> {
        let name = normalized_skill_name(&input.name)?;
        let CreateSkill {
            name: _,
            description,
            site,
            steps,
            learned_notes,
            origin,
            source_session_id,
        } = input;
        let content = render_skill_markdown(&name, &description, &steps, &learned_notes);
        let model = self.new_skill_model(&name, description, site, origin, source_session_id);
        self.try_install_new(&name, &content, model)
            .await?
            .ok_or_else(|| AppError::conflict("a skill with this name already exists"))
    }

    /// Reserve the name with an atomic insert, then write the canonical file and
    /// link it into the connected agents. The unique primary key arbitrates
    /// across processes, so a concurrent create of the same new name loses the
    /// insert and returns `Ok(None)` without touching any files or installs. A
    /// failure after a won reservation rolls the row and side effects back.
    async fn try_install_new(
        &self,
        name: &str,
        content: &str,
        mut model: skills::Model,
    ) -> AppResult<Option<SkillView>> {
        if !self.repo.try_insert(model.clone()).await? {
            return Ok(None);
        }
        let linked = match self.write_and_install(name, content).await {
            Ok(linked) => linked,
            Err(error) => {
                let _ = self.repo.delete(name).await;
                self.rollback_skill(name).await;
                return Err(error);
            }
        };
        model.linked_agents_json = linked_agents_json(&linked);
        self.repo.update(model.clone()).await?;
        Ok(Some(SkillView {
            model,
            linked_agents: linked.into_iter().collect(),
            stats: RunStats::default(),
        }))
    }

    async fn write_and_install(&self, name: &str, content: &str) -> AppResult<BTreeSet<AgentId>> {
        self.write_body(name, content).await?;
        let spec = SkillSpec::new(name, content.to_string())
            .map_err(|error| AppError::bad_request(error.to_string()))?;
        self.harness.install_skill(spec).await
    }

    fn new_skill_model(
        &self,
        name: &str,
        description: String,
        site: Option<String>,
        origin: SkillOrigin,
        source_session_id: Option<String>,
    ) -> skills::Model {
        let now = now_ms();
        skills::Model {
            name: name.to_string(),
            description,
            site,
            origin: origin.as_str().to_owned(),
            source_session_id,
            version: 1,
            body_path: self.canonical_body_path(name),
            linked_agents_json: "[]".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn canonical_body_path(&self, name: &str) -> String {
        self.skills_dir
            .join(name)
            .join("SKILL.md")
            .to_string_lossy()
            .into_owned()
    }

    pub async fn update(&self, name: &str, input: UpdateSkill) -> AppResult<SkillDetailView> {
        let _guard = self.mutate.lock().await;
        self.update_locked(name, input).await
    }

    async fn update_locked(&self, name: &str, input: UpdateSkill) -> AppResult<SkillDetailView> {
        let mut model = self.require(name).await?;
        let mut relinked: Option<BTreeSet<AgentId>> = None;
        let mut previous_body: Option<String> = None;
        let body_path = model.body_path.clone();

        let UpdateSkill {
            description,
            site,
            steps,
            learned_notes,
            body,
        } = input;

        // A raw body and structured fields are two representations of the same
        // file; accepting both would silently persist one and discard the other.
        if body.is_some() && (steps.is_some() || learned_notes.is_some()) {
            return Err(AppError::bad_request(
                "provide either a raw body or structured steps/learnedNotes, not both",
            ));
        }
        // A structured edit replaces both sections at once, so both must be sent
        // together; otherwise the omitted section would be silently erased. An
        // explicit empty array clears a section.
        if steps.is_some() != learned_notes.is_some() {
            return Err(AppError::bad_request(
                "steps and learnedNotes must be provided together",
            ));
        }

        // The description that the rendered/patched frontmatter must carry: the
        // incoming value if present, otherwise the one already stored.
        let effective_description = description
            .clone()
            .unwrap_or_else(|| model.description.clone());

        // Decide what, if anything, to write to disk. A structured edit renders
        // the whole SKILL.md so its frontmatter description can never drift from
        // the stored column. A raw `body` is written verbatim (agent authoring
        // or a manual override). A metadata-only description change patches just
        // the frontmatter line so the file matches the new column without
        // disturbing the rest of the body.
        let rewrite: Option<String> = if steps.is_some() || learned_notes.is_some() {
            Some(render_skill_markdown(
                name,
                &effective_description,
                &steps.unwrap_or_default(),
                &learned_notes.unwrap_or_default(),
            ))
        } else if let Some(raw) = body {
            Some(raw)
        } else if description.is_some() {
            let current = self.read_body(&body_path).await;
            Some(sync_frontmatter_description(
                name,
                &current,
                &effective_description,
            ))
        } else {
            None
        };

        if let Some(content) = rewrite {
            previous_body = Some(self.read_body(&body_path).await);
            self.write_body_at(&body_path, &content).await?;
            let spec = SkillSpec::new(name, content)
                .map_err(|error| AppError::bad_request(error.to_string()))?;
            relinked = Some(self.harness.install_skill(spec).await?);
            model.version += 1;
        }
        if let Some(description) = description {
            model.description = description;
        }
        // An empty string clears the site; a non-empty value sets it; an absent
        // value leaves it unchanged.
        if let Some(site) = site {
            model.site = (!site.is_empty()).then_some(site);
        }
        if let Some(linked) = &relinked {
            model.linked_agents_json = linked_agents_json(linked);
        }
        model.updated_at = now_ms();
        if let Err(error) = self.repo.update(model).await {
            // The durable row was not written, so restore the on-disk body and
            // relink it, keeping the file and agents consistent with the row.
            if let Some(previous) = previous_body {
                let _ = self.write_body_at(&body_path, &previous).await;
                if let Ok(spec) = SkillSpec::new(name, previous) {
                    let _ = self.harness.install_skill(spec).await;
                }
            }
            return Err(error);
        }
        self.get(name).await
    }

    /// Create the skill if its name is free, otherwise rewrite the existing
    /// skill's body from the same inputs. Idempotent on name; this is what the
    /// agent-facing MCP tool calls so re-authoring a task updates it in place.
    pub async fn upsert(&self, input: CreateSkill) -> AppResult<SkillView> {
        let _guard = self.mutate.lock().await;
        let name = normalized_skill_name(&input.name)?;
        let CreateSkill {
            name: _,
            description,
            site,
            steps,
            learned_notes,
            origin,
            source_session_id,
        } = input;
        let content = render_skill_markdown(&name, &description, &steps, &learned_notes);
        let model = self.new_skill_model(
            &name,
            description.clone(),
            site.clone(),
            origin,
            source_session_id,
        );
        // Reserve atomically; if the name is already taken (this run lost the
        // insert or the skill pre-existed), fall through to an in-place update.
        match self.try_install_new(&name, &content, model).await? {
            Some(view) => Ok(view),
            None => {
                let detail = self
                    .update_locked(
                        &name,
                        UpdateSkill {
                            description: Some(description),
                            site,
                            steps: None,
                            learned_notes: None,
                            body: Some(content),
                        },
                    )
                    .await?;
                Ok(detail.view)
            }
        }
    }

    pub async fn delete(&self, name: &str) -> AppResult<()> {
        let _guard = self.mutate.lock().await;
        self.delete_locked(name).await
    }

    async fn delete_locked(&self, name: &str) -> AppResult<()> {
        self.require(name).await?;
        self.harness.uninstall_skill(name).await?;
        self.repo.delete(name).await?;
        // The canonical file is now orphaned; removing it is best-effort so a
        // filesystem hiccup cannot resurrect an already-deleted skill.
        let _ = tokio::fs::remove_dir_all(self.skills_dir.join(name)).await;
        Ok(())
    }

    /// Best-effort removal of a skill's side effects (agent installs and the
    /// canonical file) after a failed create, so nothing is left without a row.
    async fn rollback_skill(&self, name: &str) {
        let _ = self.harness.uninstall_skill(name).await;
        let _ = tokio::fs::remove_dir_all(self.skills_dir.join(name)).await;
    }

    async fn require(&self, name: &str) -> AppResult<skills::Model> {
        self.repo
            .get(name)
            .await?
            .ok_or_else(|| AppError::not_found("skill not found"))
    }

    async fn write_body(&self, name: &str, content: &str) -> AppResult<String> {
        let dir = self.skills_dir.join(name);
        tokio::fs::create_dir_all(&dir).await.with_path(&dir)?;
        let path = dir.join("SKILL.md");
        tokio::fs::write(&path, content).await.with_path(&path)?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn write_body_at(&self, body_path: &str, content: &str) -> AppResult<()> {
        let path = PathBuf::from(body_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.with_path(parent)?;
        }
        tokio::fs::write(&path, content).await.with_path(&path)?;
        Ok(())
    }

    async fn read_body(&self, body_path: &str) -> String {
        tokio::fs::read_to_string(body_path)
            .await
            .unwrap_or_default()
    }
}

fn stats_from_runs(runs: &[skill_runs::Model]) -> RunStats {
    RunStats {
        run_count: runs.len() as i64,
        clean_run_count: runs.iter().filter(|run| run.clean).count() as i64,
        first_run_tokens: runs
            .iter()
            .min_by_key(|run| run.run_number)
            .and_then(|run| run.tokens),
        latest_run_tokens: runs
            .iter()
            .max_by_key(|run| run.run_number)
            .and_then(|run| run.tokens),
        last_run_at: runs.iter().map(|run| run.created_at).max(),
    }
}

fn parse_linked_agents(json: &str) -> Vec<AgentId> {
    serde_json::from_str::<Vec<String>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| AgentId::from_str(&value).ok())
        .collect()
}

fn linked_agents_json(agents: &BTreeSet<AgentId>) -> String {
    let ids = agents
        .iter()
        .map(|agent| agent.as_str())
        .collect::<Vec<_>>();
    serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string())
}

fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Namespace a name under `neo-`. Idempotent: an already-prefixed name is
/// returned unchanged, so an agent may pass either `weather` or `neo-weather`.
pub(crate) fn neo_prefixed(name: &str) -> String {
    if name.starts_with(NEO_SKILL_PREFIX) {
        name.to_owned()
    } else {
        format!("{NEO_SKILL_PREFIX}{name}")
    }
}

/// Validate a raw skill name and return its canonical `neo-`-prefixed form.
fn normalized_skill_name(raw: &str) -> AppResult<String> {
    if !is_valid_skill_name(raw) {
        return Err(AppError::bad_request(
            "skill name must contain only lowercase letters, digits, and hyphens",
        ));
    }
    let name = neo_prefixed(raw);
    if name.len() <= NEO_SKILL_PREFIX.len() {
        return Err(AppError::bad_request(
            "skill name must have a slug after the neo- prefix",
        ));
    }
    if RESERVED_SKILL_NAMES.contains(&name.as_str()) {
        return Err(AppError::conflict("skill name is reserved"));
    }
    Ok(name)
}

fn render_skill_markdown(
    name: &str,
    description: &str,
    steps: &[String],
    learned_notes: &[String],
) -> String {
    // JSON is a subset of YAML, so a JSON-encoded string is a valid, correctly
    // escaped YAML scalar. This keeps descriptions with newlines, colons, or
    // other YAML-sensitive characters from corrupting the frontmatter. The name
    // is already constrained to `[a-z0-9-]`, so it stays a plain scalar.
    let description = serde_json::to_string(description).unwrap_or_else(|_| "\"\"".to_string());
    let mut out = format!(
        "---\nname: {name}\ndescription: {description}\ntools: {SKILL_TOOLS}\n---\n\n## Steps\n"
    );
    // Every run marks itself so BrowserOS neo records the run and its cost.
    out.push_str(&format!(
        "1. Call the mark_skill_run tool with name: {name} so this run is recorded.\n"
    ));
    for (index, step) in steps.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", index + 2, step));
    }
    if !learned_notes.is_empty() {
        out.push_str("\n## Learned from past runs\n");
        for note in learned_notes {
            out.push_str(&format!("- {note}\n"));
        }
    }
    out
}

/// Rewrite the `description:` line inside a SKILL.md frontmatter block so the
/// on-disk file matches the stored description, leaving the rest of the body
/// untouched. The description is JSON-encoded to stay a valid YAML scalar, the
/// same way `render_skill_markdown` emits it. A body with no `---` frontmatter
/// fence (a raw body written without one) gets a fresh frontmatter block
/// prepended so the description is always present in the file.
fn sync_frontmatter_description(name: &str, body: &str, description: &str) -> String {
    let encoded = serde_json::to_string(description).unwrap_or_else(|_| "\"\"".to_string());
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let closing = lines
        .first()
        .map(|line| line.trim_end())
        .filter(|first| *first == "---")
        .and_then(|_| {
            lines
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, line)| line.trim_end() == "---")
                .map(|(index, _)| index)
        });
    let Some(closing) = closing else {
        let front =
            format!("---\nname: {name}\ndescription: {encoded}\ntools: {SKILL_TOOLS}\n---\n\n");
        return format!("{front}{body}");
    };
    let description_line = lines
        .iter_mut()
        .take(closing)
        .skip(1)
        .find(|line| line.trim_start().starts_with("description:"));
    match description_line {
        Some(line) => *line = format!("description: {encoded}"),
        None => lines.insert(1, format!("description: {encoded}")),
    }
    let mut out = lines.join("\n");
    if body.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{neo_prefixed, normalized_skill_name};

    #[test]
    fn neo_prefixed_is_idempotent() {
        assert_eq!(neo_prefixed("weather"), "neo-weather");
        assert_eq!(neo_prefixed("neo-weather"), "neo-weather");
        // Only a leading prefix is collapsed; an interior "neo-" is untouched.
        assert_eq!(neo_prefixed("weather-neo-check"), "neo-weather-neo-check");
    }

    #[test]
    fn normalized_skill_name_prefixes_and_validates() {
        assert_eq!(
            normalized_skill_name("weather").ok().as_deref(),
            Some("neo-weather")
        );
        assert_eq!(
            normalized_skill_name("neo-weather").ok().as_deref(),
            Some("neo-weather")
        );
        // Charset is rejected before prefixing.
        assert!(normalized_skill_name("Bad Name").is_err());
        assert!(normalized_skill_name("").is_err());
        // A bare prefix has no slug after neo-.
        assert!(normalized_skill_name("neo-").is_err());
    }
}
