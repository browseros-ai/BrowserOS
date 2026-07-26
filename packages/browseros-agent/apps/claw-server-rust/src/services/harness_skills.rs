use std::{fs, path::Path};

use harness_integrations::SkillSpec;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

const EMBEDDED_BROWSERCLAW_SKILL: &str =
    include_str!("../../resources/skills/browserclaw/SKILL.md");

#[derive(Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

/// Loads the active signed resource copy, falling back to the compiled asset when unavailable.
pub fn load_browserclaw_skill(resources_dir: &Path) -> AppResult<SkillSpec> {
    let path = resources_dir.join("skills/browserclaw/SKILL.md");
    match fs::read_to_string(&path) {
        Ok(content) => match parse_browserclaw_skill(content) {
            Ok(spec) => return Ok(spec),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "invalid BrowserClaw skill resource; using embedded fallback");
            }
        },
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "BrowserClaw skill resource unavailable; using embedded fallback");
        }
    }
    parse_browserclaw_skill(EMBEDDED_BROWSERCLAW_SKILL.to_string()).map_err(|error| {
        AppError::Internal(format!(
            "embedded BrowserClaw skill failed validation: {error}"
        ))
    })
}

fn parse_browserclaw_skill(content: String) -> Result<SkillSpec, String> {
    let normalized = content.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next() != Some("---") {
        return Err("frontmatter must begin with `---`".to_string());
    }
    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in &mut lines {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line);
    }
    if !closed {
        return Err("frontmatter is missing its closing `---`".to_string());
    }
    let frontmatter: SkillFrontmatter = serde_saphyr::from_str(&frontmatter.join("\n"))
        .map_err(|error| format!("frontmatter is not valid YAML: {error}"))?;
    if frontmatter.name != "browserclaw" {
        return Err("frontmatter `name` must be `browserclaw`".to_string());
    }
    if frontmatter.description.trim().is_empty() {
        return Err("frontmatter requires a non-empty `description`".to_string());
    }
    SkillSpec::new("browserclaw", content).map_err(|error| error.to_string())
}

#[cfg(test)]
fn embedded_browserclaw_skill() -> &'static str {
    EMBEDDED_BROWSERCLAW_SKILL
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{embedded_browserclaw_skill, load_browserclaw_skill};

    #[test]
    fn harness_skills_embedded_resource_is_concise_and_actionable()
    -> Result<(), Box<dyn std::error::Error>> {
        let content = embedded_browserclaw_skill();
        assert!(content.starts_with("---\nname: browserclaw\n"));
        assert!(content.contains("description:"));
        assert!(content.contains("task-owned tabs"));
        assert!(content.contains("snapshot -> act -> verify"));
        assert!(content.contains("untrusted data"));
        assert!(content.contains("name_session"));
        assert!(content.contains("unprompted"));
        assert!(content.lines().count() < 60);
        Ok(())
    }

    #[test]
    fn harness_skills_runtime_resource_precedes_the_embedded_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let skill_dir = root.path().join("skills/browserclaw");
        fs::create_dir_all(&skill_dir)?;
        let runtime = "---\nname: browserclaw\ndescription: Runtime copy\n---\nruntime\n";
        fs::write(skill_dir.join("SKILL.md"), runtime)?;

        let loaded = load_browserclaw_skill(root.path())?;

        assert_eq!(loaded.name(), "browserclaw");
        assert_eq!(loaded.content(), runtime);
        Ok(())
    }

    #[test]
    fn harness_skills_missing_unreadable_or_invalid_runtime_uses_the_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let expected = embedded_browserclaw_skill();
        assert_eq!(load_browserclaw_skill(root.path())?.content(), expected);

        let path = root.path().join("skills/browserclaw/SKILL.md");
        fs::create_dir_all(&path)?;
        assert_eq!(load_browserclaw_skill(root.path())?.content(), expected);
        fs::remove_dir(&path)?;

        fs::write(&path, "---\nname: wrong\ndescription: Wrong name\n---\n")?;
        assert_eq!(load_browserclaw_skill(root.path())?.content(), expected);

        fs::write(
            &path,
            "---\nname: browserclaw\ndescription: [\n---\nmalformed\n",
        )?;
        assert_eq!(load_browserclaw_skill(root.path())?.content(), expected);

        fs::write(
            &path,
            "---\nname: browserclaw\ndescription: |\n---\nempty description\n",
        )?;
        assert_eq!(load_browserclaw_skill(root.path())?.content(), expected);
        Ok(())
    }
}
