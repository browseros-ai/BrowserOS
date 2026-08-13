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

        let content = render_skill_markdown(
            &input.name,
            &input.description,
            &input.steps,
            &input.learned_notes,
        );
        let body_path = self.write_body(&input.name, &content).await?;
        let spec = SkillSpec::new(input.name.as_str(), content)
            .map_err(|error| AppError::bad_request(error.to_string()))?;
        let linked = self.harness.install_skill(spec).await?;

        let now = now_ms();
        let model = skills::Model {
            name: input.name,
            description: input.description,
            site: input.site,
            origin: input.origin.as_str().to_owned(),
            source_session_id: input.source_session_id,
            version: 1,
            body_path,
            linked_agents_json: linked_agents_json(&linked),
            created_at: now,
            updated_at: now,
        };
        self.repo.insert(model.clone()).await?;
        Ok(SkillView {
            model,
            linked_agents: linked.into_iter().collect(),
            stats: RunStats::default(),
        })
    }

    pub async fn update(&self, name: &str, input: UpdateSkill) -> AppResult<SkillDetailView> {
        let mut model = self.require(name).await?;
        let mut relinked: Option<BTreeSet<AgentId>> = None;

        if let Some(body) = input.body {
            self.write_body_at(&model.body_path, &body).await?;
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
        self.repo.update(model).await?;
        self.get(name).await
    }

    pub async fn delete(&self, name: &str) -> AppResult<()> {
        self.require(name).await?;
        self.harness.uninstall_skill(name).await?;
        let skill_dir = self.skills_dir.join(name);
        if let Err(error) = tokio::fs::remove_dir_all(&skill_dir).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(AppError::Io {
                path: Some(skill_dir),
                source: error,
            });
        }
        self.repo.delete(name).await?;
        Ok(())
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
