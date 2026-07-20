---
name: write-internal-docs
description: Write a doc into the private internal-docs repo as Markdown plus a rendered HTML sibling, tidy the repo's structure and index, and open a PR to browseros-ai/internal-docs.
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
disable-model-invocation: true
---

# Write Internal Docs

Write a new doc for `.internal-docs/` (private repo `browseros-ai/internal-docs`), as Markdown plus a self-contained HTML sibling, and open a PR. Companion to `ask-internal`: that skill reads internal-docs, this one writes it.

**Announce at start:** "I'm using the write-internal-docs skill to draft and land an internal doc."

## Hard rules — never do these

- NEVER write inside the user's `.internal-docs/` checkout. All writes happen in a tmp clone.
- NEVER push to internal-docs `main`. Feature branch + PR only.
- NEVER touch the OSS repo's `.gitmodules` or submodule pointer. The sync workflow moves it after merge.
- NEVER `git add -A` or `git add .` in the tmp clone. Specific paths only.
- NEVER fabricate content for empty template sections. Empty stays empty.
- NEVER hand-edit an `.html` sibling. The `.md` is the source of truth; regenerate the HTML from it.
- NEVER cite a file or line number you have not actually read.

## Voice rules

Every sentence of doc output follows these. Step 4 enforces them.

- Lead with the point. First sentence answers "what is this?"
- Concrete nouns. Name files, functions, commands. Not "the system".
- Short sentences, average under 20 words. Active voice. No em dashes.
- Banned words: delve, crucial, robust, comprehensive, nuanced, multifaceted, furthermore, moreover, additionally, pivotal, landscape, tapestry, underscore, foster, showcase, intricate, vibrant, fundamental, significant, leverage, utilize.
- No filler intros ("This document describes..."). Start with the substance.
- Feature notes: body 60 lines max. Architecture and design docs have no cap.

## Workflow

### Step 0: Pre-flight

```bash
if git submodule status .internal-docs 2>/dev/null | grep -q '^-'; then
  echo "internal-docs submodule not initialized. Run: git submodule update --init .internal-docs"
  exit 0
fi
[ -d .internal-docs ] && [ -n "$(ls -A .internal-docs 2>/dev/null)" ] || {
  echo ".internal-docs/ missing or empty. Submodule not configured?"; exit 0; }
gh auth status >/dev/null 2>&1 || { echo "gh not authenticated. Run: gh auth login"; exit 0; }
```

**Done when:** submodule present and `gh` authenticated, or the skill stopped with the fix command.

### Step 1: Scope the doc

Establish four facts. Take them from the user's invocation; derive what you can before asking.

1. **Subject** — a stated topic, or the current branch's diff (`git diff main...HEAD --stat` plus the PR body) when invoked from a feature branch with no topic.
2. **Type and target dir** — `setup/` (runbook), `features/` (shipped feature), `architecture/` (cross-cutting subsystem), `designs/` (decision or RFC). Branch heuristics: `feat/*` → features, `rfc/*`/`design/*` → designs. Unclear → ask one question.
3. **Slug** — short kebab-case. Features and designs get a date prefix: `YYYY-MM-<slug>.md`.
4. **Owner** — GitHub handle, default `gh api user --jq .login`.

**Done when:** all four are stated and the target path (`<dir>/<file>.md`) is printed.

### Step 2: Zoom out

Before drafting, go up one layer of abstraction. Map the territory the doc covers: the relevant modules, their callers, and how data flows between them, in the project's domain vocabulary. Read the real files; tie every named module to a path.

This map becomes the doc's first body section, before any detail. A reader who knows nothing about the area gets the shape first, then zooms in.

**Done when:** you can draw the map (ASCII or mermaid, plus 2-4 sentences) and every box in it names a real path you read.

### Step 3: Draft the Markdown

Read the matching template from `.internal-docs/_templates/` (`feature-note.md`, `architecture-note.md`, `design-spec.md`; setup runbooks follow the shape of existing `setup/` docs). Fill it:

- The zoom-out map from Step 2 leads the body, as the first section after the frontmatter (for feature notes, it opens "How it works").
- Every factual claim cites `path/to/file.ts:line` you actually read.
- Sections with nothing real to say stay empty.

**Done when:** the draft matches the template's sections, opens with the map, and every claim carries a citation.

### Step 4: Voice check

Scan the draft against the voice rules: em dashes, banned words, sentence length, filler intros, the 60-line cap for feature notes. Rewrite offending sentences in place, max 3 passes. Still failing after 3 → stop and report which rules are violated.

**Done when:** a scan finds zero violations, or the failure is reported.

### Step 5: Clone, write, render HTML

Work in a tmp clone so the user's checkout stays clean:

```bash
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
git clone -b main git@github.com:browseros-ai/internal-docs.git "$TMP"
git -C "$TMP" checkout -b "docs/<slug>"
```

Write the approved `.md` into the clone at the Step 1 path. Then render its sibling `<same-path>.html`:

1. Read `reference/html-template.html` from this skill's folder.
2. Convert the Markdown body to HTML — with `pandoc` if installed (`pandoc -f gfm -t html <doc>.md`), by hand otherwise.
3. Fill the template's `{{TITLE}}`, `{{SOURCE_MD}}`, and `{{BODY}}` slots. Keep it self-contained: no external URLs, scripts, or fonts.

Update the clone's `README.md` index — one line under the matching section:

```markdown
- [<Title>](<dir>/<file>.md) ([html](<dir>/<file>.html)): <one-line hook>
```

**Done when:** `.md`, `.html`, and the index line exist in the clone, and the `.html` contains no `http` reference except links that were in the doc body itself.

### Step 6: Tidy pass

Now sweep the whole clone for drift. Three checks:

```bash
# 1. Index drift: docs on disk missing from the README index
comm -13 <(grep -o '([a-z-]*/[^)]*\.md)' README.md | tr -d '()' | sort) \
         <(find setup features architecture designs -name '*.md' 2>/dev/null | sort)
# 2. Dead links: index entries pointing at files that do not exist
grep -o '([a-z-]*/[^)]*\.md)' README.md | tr -d '()' | while read -r f; do [ -f "$f" ] || echo "dead: $f"; done
# 3. Misfiled docs: read anything the filename or frontmatter suggests is in the wrong dir
```

For each finding, fix it: add the missing index line, delete or repoint the dead link, `git mv` the misfiled doc and update its index entry. No findings → skip the commit. Reorg changes go in their own `chore(docs): tidy structure and index` commit, separate from the new doc, so the reviewer sees doc vs reorg apart.

**Done when:** all three checks ran and every finding is either fixed in the clone or listed for the PR body as deliberately left.

### Step 7: Open the PR

```bash
git -C "$TMP" add "<dir>/<file>.md" "<dir>/<file>.html" README.md   # plus tidy-pass paths
git -C "$TMP" commit -m "docs(<type>): <slug>"
git -C "$TMP" push -u origin "docs/<slug>"
gh pr create -R browseros-ai/internal-docs --base main --head "docs/<slug>" \
  --title "docs(<type>): <slug>" \
  --body "<summary, source branch, related OSS PR, tidy-pass findings if any>"
```

**Done when:** the PR URL is printed.

### Step 8: Completion status

Report one of:

- **DONE** — md + html written, index updated, PR opened. Print the PR URL.
- **DONE_WITH_CONCERNS** — PR opened, but list concerns (voice check needed 3 passes, tidy findings left unfixed, citations uncertain).
- **BLOCKED** — pre-flight failed, auth failed, or template missing. State exactly what unblocks.

## Common Mistakes

**Drafting before zooming out**
- **Problem:** The doc dives into one function's details; a newcomer can't place it.
- **Fix:** Step 2 is not optional. Map first, then draft.

**Editing the HTML instead of the Markdown**
- **Problem:** The two siblings diverge; the next regeneration silently reverts the edit.
- **Fix:** Edit the `.md`, re-run Step 5's render.

**Touching `.internal-docs/` directly**
- **Problem:** User's submodule HEAD moves; the parent repo shows a dirty state.
- **Fix:** All writes go through the tmp clone.

**Tidy pass bundled into the doc commit**
- **Problem:** Reviewer can't separate the new doc from moves and index fixes.
- **Fix:** Two commits: `docs(<type>): <slug>` and `chore(docs): tidy structure and index`.
