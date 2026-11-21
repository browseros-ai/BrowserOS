//! Renders the Playwright CDP recipe registry for MCP instructions. The contract tests
//! use the same registry, and the embedded guide includes a generated copy.

use serde::Deserialize;
use std::sync::LazyLock;

const ENABLED: bool = true;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipe {
    // The contract suite addresses scenarios by id; instructions use the visible title.
    #[serde(rename = "id")]
    _id: String,
    pub(super) title: String,
    when: String,
    code: String,
}

static RECIPES: LazyLock<Vec<Recipe>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../../../resources/skills/browserclaw/recipes.json"
    ))
    .unwrap_or_else(|error| panic!("invalid embedded CDP recipes: {error}"))
});

pub(super) fn all() -> &'static [Recipe] {
    if ENABLED { &RECIPES } else { &[] }
}

pub(super) fn instruction_lines() -> String {
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
    fn embedded_skill_recipe_section_matches_the_registry() -> anyhow::Result<()> {
        let skill =
            include_str!("../../../../../../../resources/skills/browserclaw/SKILL.playwright.md");
        let expected = format!("\n## CDP recipes\n\n{}\n", instruction_lines());
        let regenerate = "from the repository root run: python3 packages/browseros-agent/resources/skills/browserclaw/generate-recipes.py";
        assert_eq!(
            skill.matches("<!-- recipes:start -->").count(),
            1,
            "{regenerate}"
        );
        assert_eq!(
            skill.matches("<!-- recipes:end -->").count(),
            1,
            "{regenerate}"
        );
        let (_, section) = skill
            .split_once("<!-- recipes:start -->")
            .ok_or_else(|| anyhow::anyhow!("{regenerate}"))?;
        let (actual, _) = section
            .split_once("<!-- recipes:end -->")
            .ok_or_else(|| anyhow::anyhow!("{regenerate}"))?;
        assert_eq!(actual, expected, "Playwright recipes drifted. {regenerate}");
        Ok(())
    }
}
