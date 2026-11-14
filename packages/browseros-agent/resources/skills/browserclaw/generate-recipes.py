"""Regenerate committed skill recipes from the registry. Run from any directory."""

import json
from pathlib import Path

source = Path(__file__).resolve().parent
root = source.parents[4]
recipes = json.loads((source / "recipes.json").read_text())
lines = "\n".join(
    f"- {recipe['title']}: `{recipe['code'].replace(chr(10), ' ')}` {recipe['when']}"
    for recipe in recipes
)
section = f"\n## CDP recipes\n\n{lines}\n"
start, end = "<!-- recipes:start -->", "<!-- recipes:end -->"
for path in [source / "SKILL.md", root / "skills/browseros-neo/SKILL.md"]:
    content = path.read_text()
    assert content.count(start) == content.count(end) == 1, path
    before, remainder = content.split(start)
    _, after = remainder.split(end)
    path.write_text(before + start + section + end + after)
