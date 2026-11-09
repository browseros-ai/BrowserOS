//! Neo: keep the hook guide and author type vocabulary tied to the served action table.
//! This test alone writes the fixtures when `NEO_RUNTIME_WRITE_FIXTURES=1` is set.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use crate::actions;

/// Neo: MDX-safe boundaries of the generated hook kinds table; prose stays hand-written.
const TABLE_START: &str = "{/* BEGIN GENERATED HOOK KINDS */}";
/// Neo: the end boundary paired with `TABLE_START` in the author guide.
const TABLE_END: &str = "{/* END GENERATED HOOK KINDS */}";
/// Neo: the command used by the other runtime fixtures, scoped to this author-docs test.
const REGENERATE: &str =
    "NEO_RUNTIME_WRITE_FIXTURES=1 cargo test -p neo-runtime --locked --lib author_docs";

#[test]
fn tests_that_hook_author_docs_match_served_actions() {
    let mut groups = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut kinds = BTreeSet::new();
    for action in actions::served().map(actions::ServedMethod::action) {
        let kind = action.kind().as_str();
        groups
            .entry(action.group().as_str())
            .or_default()
            .insert(kind);
        kinds.insert(kind);
    }

    let mut table = String::from("| Group | Kinds |\n| --- | --- |\n");
    for (group, names) in &groups {
        let names = names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>();
        writeln!(table, "| `{group}` | {} |", names.join(", ")).unwrap();
    }

    let mut declarations = format!(
        "/**\n * Neo: hook filter vocabulary generated from neo-runtime's served actions.\n \
         * Regenerate from packages/browseros-agent:\n * {REGENERATE}\n */\n"
    );
    union(&mut declarations, "HookKind", kinds.into_iter());
    union(&mut declarations, "HookGroup", groups.keys().copied());

    let runtime =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let guide = runtime.join("../../../../docs/neo/hooks.mdx");
    let source = fs::read_to_string(&guide).expect("hook guide exists");
    let (before, generated) = source.split_once(TABLE_START).expect("table start marker");
    let (_, after) = generated.split_once(TABLE_END).expect("table end marker");
    let expected = format!("{before}{TABLE_START}\n\n{table}\n{TABLE_END}{after}");
    fixture(&guide, &expected);
    fixture(
        &runtime.join("../neo-quickjs-injected/hooks-kinds.d.ts"),
        &declarations,
    );
}

fn union(output: &mut String, name: &str, values: impl Iterator<Item = &'static str>) {
    writeln!(output, "\nexport type {name} =").unwrap();
    for value in values {
        writeln!(output, "  | '{value}'").unwrap();
    }
}

fn fixture(path: &Path, expected: &str) {
    if env::var("NEO_RUNTIME_WRITE_FIXTURES").as_deref() == Ok("1") {
        fs::write(path, expected).expect("write hook author fixture");
    }
    assert!(
        fs::read_to_string(path).unwrap_or_default() == expected,
        "{} is stale; run from packages/browseros-agent: {REGENERATE}",
        path.display()
    );
}
