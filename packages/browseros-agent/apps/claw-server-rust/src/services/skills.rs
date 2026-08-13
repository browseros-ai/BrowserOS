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

/// The embedded product skill owns this directory name; user skills may not
/// reuse it or they would clobber the managed BrowserOS skill.
const RESERVED_SKILL_NAMES: [&str; 1] = ["browserclaw"];
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
}

impl SkillService {
    #[must_use]
    pub fn new(repo: SkillsRepository, harness: Arc<HarnessService>, skills_dir: PathBuf) -> Self {
        Self {
            repo,
            harness,
            skills_dir,
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

    pub async fn create(&self, input: CreateSkill) -> AppResult<SkillView> {
        if !is_valid_skill_name(&input.name) {
            return Err(AppError::bad_request(
                "skill name must contain only lowercase letters, digits, and hyphens",
            ));
        }
        if RESERVED_SKILL_NAMES.contains(&input.name.as_str()) {
            return Err(AppError::conflict("skill name is reserved"));
        }
        if self.repo.exists(&input.name).await? {
            return Err(AppError::conflict("a skill with this name already exists"));
        }

        let CreateSkill {
            name,
            description,
            site,
            steps,
            learned_notes,
            origin,
            source_session_id,
        } = input;
        let content = render_skill_markdown(&name, &description, &steps, &learned_notes);
        // The agent install happens before the row lands, so a failure after it
        // rolls the side effects back to avoid an install with no persisted row.
        let body_path = self.write_body(&name, &content).await?;
        let spec = SkillSpec::new(name.as_str(), content)
            .map_err(|error| AppError::bad_request(error.to_string()))?;
        let linked = match self.harness.install_skill(spec).await {
            Ok(linked) => linked,
            Err(error) => {
                self.rollback_skill(&name).await;
                return Err(error);
            }
        };

        let now = now_ms();
        let model = skills::Model {
            name: name.clone(),
            description,
            site,
            origin: origin.as_str().to_owned(),
            source_session_id,
            version: 1,
            body_path,
            linked_agents_json: linked_agents_json(&linked),
            created_at: now,
            updated_at: now,
        };
        if let Err(error) = self.repo.insert(model.clone()).await {
            self.rollback_skill(&name).await;
            return Err(error);
        }
        Ok(SkillView {
            model,
            linked_agents: linked.into_iter().collect(),
            stats: RunStats::default(),
        })
    }

    pub async fn update(&self, name: &str, input: UpdateSkill) -> AppResult<SkillDetailView> {
        let mut model = self.require(name).await?;
        let mut relinked: Option<BTreeSet<AgentId>> = None;
        let mut previous_body: Option<String> = None;
        let body_path = model.body_path.clone();

        if let Some(body) = input.body {
            previous_body = Some(self.read_body(&body_path).await);
            self.write_body_at(&body_path, &body).await?;
            let spec = SkillSpec::new(name, body)
                .map_err(|error| AppError::bad_request(error.to_string()))?;
            relinked = Some(self.harness.install_skill(spec).await?);
            model.version += 1;
        }
        if let Some(description) = input.description {
            model.description = description;
        }
        // A nullable `site` cannot be explicitly cleared through the current
        // contract; clearing is deferred to a later contract revision.
        if let Some(site) = input.site {
            model.site = Some(site);
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
        if !is_valid_skill_name(&input.name) {
            return Err(AppError::bad_request(
                "skill name must contain only lowercase letters, digits, and hyphens",
            ));
        }
        if RESERVED_SKILL_NAMES.contains(&input.name.as_str()) {
            return Err(AppError::conflict("skill name is reserved"));
        }
        if self.repo.get(&input.name).await?.is_none() {
            return self.create(input).await;
        }
        let CreateSkill {
            name,
            description,
            site,
            steps,
            learned_notes,
            ..
        } = input;
        let content = render_skill_markdown(&name, &description, &steps, &learned_notes);
        let detail = self
            .update(
                &name,
                UpdateSkill {
                    description: Some(description),
                    site,
                    body: Some(content),
                },
            )
            .await?;
        Ok(detail.view)
    }

    pub async fn delete(&self, name: &str) -> AppResult<()> {
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
    let mut out =
        format!("---\nname: {name}\ndescription: {description}\ntools: {SKILL_TOOLS}\n---\n");
    if !steps.is_empty() {
        out.push_str("\n## Steps\n");
        for (index, step) in steps.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", index + 1, step));
        }
    }
    if !learned_notes.is_empty() {
        out.push_str("\n## Learned from past runs\n");
        for note in learned_notes {
            out.push_str(&format!("- {note}\n"));
        }
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
