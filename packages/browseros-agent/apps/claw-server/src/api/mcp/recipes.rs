//! One embedded recipe registry feeds MCP instructions, installed skill references,
//! and the live contract suite. No extra resource file is needed in the browser bundle.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Turn off both recipe readers if the SDK's opt-in CDP bridge is held back.
pub(crate) const ENABLED: bool = true;

/// Executable examples use named inputs described by `when`; the contract suite supplies
/// those inputs and runs the same `code`, rather than maintaining a second set of examples.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipe {
    pub(crate) id: String,
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

/// Every agent receives the runnable call even when its harness has no skill reader.
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

pub(crate) fn markdown() -> String {
    let mut content = String::from(
        "# BrowserOS neo recipes\n\nRun these snippets inside MCP `run`. Set the named inputs from live browser results first. Prefer Playwright for page work.\n\n",
    );
    for recipe in all() {
        content.push_str(&format!(
            "<a id=\"{}\"></a>\n\n## {}\n\n{}\n\n```js\n{}\n```\n\n",
            recipe.id, recipe.title, recipe.when, recipe.code
        ));
    }
    format!("{}\n{content}", ownership_header(&content))
}

/// The guide owns only bytes it generated. A changed body loses that proof, even
/// when its surrounding SKILL.md still has a valid harness ownership marker.
pub(crate) fn is_generated_markdown(content: &str) -> bool {
    let Some((header, body)) = content.split_once('\n') else {
        return false;
    };
    header == ownership_header(body)
}

fn ownership_header(body: &str) -> String {
    format!(
        "<!-- browseros-neo generated recipes sha256:{:x} -->",
        Sha256::digest(body.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn each_recipe_is_unique_and_appears_in_both_readers() {
        let instructions = instruction_lines();
        let reference = markdown();
        let mut ids = BTreeSet::new();
        let mut titles = BTreeSet::new();
        assert!(!ENABLED || !all().is_empty());
        for recipe in all() {
            assert!(ids.insert(&recipe.id), "duplicate recipe id");
            assert!(titles.insert(&recipe.title), "duplicate recipe title");
            assert!(!recipe.id.is_empty());
            assert!(!recipe.when.is_empty());
            assert!(recipe.code.contains("neo.cdp("));
            assert!(instructions.contains(&recipe.title));
            assert!(instructions.contains(&recipe.code.replace('\n', " ")));
            assert!(reference.contains(&recipe.title));
            assert!(reference.contains(&recipe.code));
        }
    }

    #[test]
    fn generated_guide_ownership_requires_an_unchanged_body() {
        let reference = markdown();
        assert!(is_generated_markdown(&reference));
        assert!(!is_generated_markdown(&format!("{reference}my notes\n")));
        assert!(!is_generated_markdown("# My own recipes\n"));
        assert!(!is_generated_markdown(
            "<!-- browseros-neo generated recipes sha256:wrong -->\nbody\n"
        ));
    }
}
