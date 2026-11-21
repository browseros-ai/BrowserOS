//! Shared executable CDP examples for MCP instructions and contract cases. Skills
//! commit the same rendered lines so installing them needs only the existing skill path.

use serde::Deserialize;
use std::sync::LazyLock;

pub(crate) const ENABLED: bool = true;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipe {
    // The contract suite addresses scenarios by id; instructions use the visible title.
    #[serde(rename = "id")]
    _id: String,
    pub(crate) title: String,
    pub(crate) when: String,
    pub(crate) code: String,
}

static RECIPES: LazyLock<Vec<Recipe>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../resources/skills/browserclaw/recipes.json"
    ))
    .unwrap_or_else(|error| panic!("invalid embedded CDP recipes: {error}"))
});

pub(crate) fn all() -> &'static [Recipe] {
    if ENABLED { &RECIPES } else { &[] }
}

pub(crate) fn instruction_lines() -> String {
    all()
        .iter()
        .map(|recipe| {
            format!(
                "- {}: `{}` {}",
                recipe.title,
                recipe.code.replace('\n', " "),
                recipe.when
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn each_recipe_is_unique_and_runnable() {
        let instructions = instruction_lines();
        let mut ids = BTreeSet::new();
        let mut titles = BTreeSet::new();
        assert!(!ENABLED || !all().is_empty());
        for recipe in all() {
            assert!(ids.insert(&recipe._id), "duplicate recipe id");
            assert!(titles.insert(&recipe.title), "duplicate recipe title");
            assert!(!recipe._id.is_empty());
            assert!(!recipe.when.is_empty());
            assert!(recipe.code.contains("neo.cdp("));
            assert!(instructions.contains(&recipe.code.replace('\n', " ")));
        }
    }

    #[test]
    fn committed_skill_recipe_sections_match_the_registry() -> anyhow::Result<()> {
        let expected = format!("\n## CDP recipes\n\n{}\n", instruction_lines());
        for (name, skill) in [
            (
                "bundled skill",
                include_str!("../../../../../resources/skills/browserclaw/SKILL.md"),
            ),
            (
                "standalone skill",
                include_str!("../../../../../../../skills/browseros-neo/SKILL.md"),
            ),
        ] {
            let regenerate = "from the repository root run: python3 packages/browseros-agent/resources/skills/browserclaw/generate-recipes.py";
            assert_eq!(
                skill.matches("<!-- recipes:start -->").count(),
                1,
                "{name}: {regenerate}"
            );
            assert_eq!(
                skill.matches("<!-- recipes:end -->").count(),
                1,
                "{name}: {regenerate}"
            );
            let (before, section) = skill
                .split_once("<!-- recipes:start -->")
                .ok_or_else(|| anyhow::anyhow!("{name}: {regenerate}"))?;
            let (actual, after) = section
                .split_once("<!-- recipes:end -->")
                .ok_or_else(|| anyhow::anyhow!("{name}: {regenerate}"))?;
            assert!(
                before.lines().count() + after.lines().count() < 60,
                "{name} hand-written guide must stay under 60 lines"
            );
            assert_eq!(actual, expected, "{name} recipes drifted; {regenerate}");
        }
        Ok(())
    }
}
