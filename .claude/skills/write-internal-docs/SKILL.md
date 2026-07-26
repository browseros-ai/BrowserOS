---
name: write-internal-docs
description: Write a doc into the private internal-docs repo as Markdown plus a rendered HTML sibling, tidy the repo's structure and index, and open a PR to browseros-ai/internal-docs.
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
disable-model-invocation: true
---

# Write Internal Docs

Write a doc for `.internal-docs/` (private repo `browseros-ai/internal-docs`), as Markdown plus a self-contained HTML sibling, and open a PR. The subject is whatever the user names ("/write-internal-docs nightly signing") or, on a feature branch with no topic, the branch's diff. Companion to `ask-internal`: that skill reads internal-docs, this one writes it. Supersedes the older personal `document-internal` flow.

**Announce at start:** "I'm using the write-internal-docs skill to draft and land an internal doc."

## Hard rules — never do these

- NEVER write inside the user's `.internal-docs/` checkout. All writes happen in the work clone.
- NEVER push to internal-docs `main`. Feature branch + PR only.
- NEVER touch the OSS repo's `.gitmodules` or submodule pointer. The sync workflow moves it after merge.
- NEVER `git add -A` or `git add .` in the work clone. Specific paths only.
- NEVER run a clone-touching command without the guard `[ -d "$CLONE/.git" ]` — a missing clone must fail loudly, not fall through to the OSS repo.
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

## The work clone

Shell state does not survive between Bash calls, so the clone lives at a deterministic path that every snippet re-derives — never `mktemp`, never a cleanup `trap` (it would fire when the first call exits and delete the clone mid-workflow):

```bash
CLONE="${TMPDIR:-/tmp}/internal-docs-<slug>"
```

Substitute the literal slug. Every later snippet starts with this line plus the guard `[ -d "$CLONE/.git" ] || { echo "work clone missing: $CLONE"; exit 1; }`. Cleanup is an explicit `rm -rf "$CLONE"` in Step 8, never automatic.

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
git ls-remote git@github.com:browseros-ai/internal-docs.git HEAD >/dev/null 2>&1 || {
  echo "Cannot reach internal-docs over SSH. Check your keys: ssh -T git@github.com"; exit 0; }
```

**Done when:** submodule present, `gh` authenticated, and SSH reaches internal-docs — or the skill stopped with the fix command.

### Step 1: Scope the doc

Establish four facts. Take them from the user's invocation; derive what you can before asking.

1. **Subject** — the specific thing the user named, or the current branch's diff (`git diff main...HEAD --stat` plus the PR body) when invoked from a feature branch with no topic. For a named subject, research it first: grep the codebase and `.internal-docs/`, read the files that own it. If a doc on it already exists, this run updates that doc instead of creating a twin.
2. **Type and target dir** — `setup/` (runbook), `features/` (shipped feature), `architecture/` (cross-cutting subsystem), `designs/` (decision or RFC). Branch heuristics: `feat/*` → features, `rfc/*`/`design/*` → designs. Unclear → ask one question.
3. **Filename** — short kebab-case slug. Features prefix `YYYY-MM-`, designs prefix `YYYY-MM-DD-` (matches the existing tree).
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

Create the work clone (user's checkout stays clean):

```bash
CLONE="${TMPDIR:-/tmp}/internal-docs-<slug>"
rm -rf "$CLONE"
git clone -b main git@github.com:browseros-ai/internal-docs.git "$CLONE"
git -C "$CLONE" checkout -b "docs/<slug>"
```

Write the approved `.md` into the clone at the Step 1 path. Then render its sibling `<same-path>.html`:

1. Read `reference/html-template.html` from this skill's folder.
2. Convert the Markdown body to HTML — with `pandoc` if installed (`pandoc -f gfm -t html <doc>.md`), by hand otherwise. Either way the YAML frontmatter is stripped from the body; it never renders.
3. Fill the template's slots: `{{TITLE}}` from the frontmatter `title:`, `{{SOURCE_MD}}` with the md filename, `{{BODY}}` with the converted body. Keep it self-contained: no external URLs, scripts, or fonts.

The README index line comes later, in Step 7 — Step 6's tidy commit also touches `README.md`, and the new doc's index line must ride the doc commit, not the tidy commit.

**Done when:** `.md` and `.html` exist in the clone, and the `.html` contains no `http` reference except links that were in the doc body itself.

### Step 6: Tidy pass

Sweep the whole clone for drift — from inside it:

```bash
CLONE="${TMPDIR:-/tmp}/internal-docs-<slug>"
[ -d "$CLONE/.git" ] || { echo "work clone missing: $CLONE"; exit 1; }
cd "$CLONE"
IDX="${TMPDIR:-/tmp}/internal-docs-<slug>-index.txt"
# Index entries, with commented-out placeholders stripped first. Per-line strip on
# purpose: a range delete (sed '/<!--/,/-->/d') swallows live entries between
# non-adjacent comment lines.
sed 's/<!--.*-->//' README.md | grep -o '([a-z-]*/[^)]*\.md)' | tr -d '()' | sort > "$IDX"
# 1. Dead links: live index entries pointing at files that do not exist
while read -r f; do [ -f "$f" ] || echo "dead: $f"; done < "$IDX"
# 2. Docs on disk missing from the index
find setup features architecture designs -name '*.md' 2>/dev/null | sort | comm -13 "$IDX" -
# 3. Misfiled docs: read anything whose filename or frontmatter suggests the wrong dir
```

Interpret before acting:

- Commented-out index lines are placeholders the maintainers left on purpose — the `sed` excludes them; never delete or uncomment them.
- A doc with any ancestor directory whose own `README.md` is in the index (e.g. everything under `architecture/rust-port/`, including its `reference/` subdir) is subtree-indexed, not drift. Skip it.
- The doc this run is adding is not yet indexed by design — its line lands in Step 7. Skip it.
- What remains is real drift: add the missing index line, repoint or remove the dead link, `git mv` the misfiled doc and fix its entry. A moved doc's `.html` sibling and its `(html)` index link move with it.

Commit tidy changes here, separate from the doc, so the reviewer sees them apart. No findings → no commit:

```bash
git -C "$CLONE" add <each tidied path> README.md
git -C "$CLONE" commit -m "chore(docs): tidy structure and index"
```

**Done when:** all three checks ran from the clone and every finding is fixed in the tidy commit, skipped by the rules above, or listed for the PR body as deliberately left.

### Step 7: Open the PR

Add the new doc's line to the clone's `README.md` index, under the matching section:

```markdown
- [<Title>](<dir>/<file>.md) ([html](<dir>/<file>.html)): <one-line hook>
```

The `(html)` link is this skill's addition to the index convention; older entries lack it, and the tidy pass backfills nothing — html siblings appear as docs get touched.

Then commit the doc (tidy changes were committed in Step 6) and open the PR:

```bash
CLONE="${TMPDIR:-/tmp}/internal-docs-<slug>"
[ -d "$CLONE/.git" ] || { echo "work clone missing: $CLONE"; exit 1; }
cd "$CLONE"
git add "<dir>/<file>.md" "<dir>/<file>.html" README.md
git commit -m "docs(<type>): <slug>"
git push -u origin "docs/<slug>"
gh pr create -R browseros-ai/internal-docs --base main --head "docs/<slug>" \
  --title "docs(<type>): <slug>" \
  --body "<summary, source branch, related OSS PR, tidy-pass findings if any>"
```

**Done when:** the PR URL is printed.

### Step 8: Report and clean up

Remove the work clone (`rm -rf "$CLONE"`), then report exactly one of:

- **DONE** — md + html written, index updated, PR opened. Print the PR URL.
- **DONE_WITH_CONCERNS** — PR opened, but list concerns (voice check needed 3 passes, tidy findings left unfixed, citations uncertain).
- **BLOCKED** — pre-flight failed, auth failed, or template missing. State exactly what unblocks.

**Done when:** the clone is gone and exactly one status line is printed.

## Common Mistakes

**Drafting before zooming out**
- **Problem:** The doc dives into one function's details; a newcomer can't place it.
- **Fix:** Step 2 is not optional. Map first, then draft.

**Editing the HTML instead of the Markdown**
- **Problem:** The two siblings diverge; the next regeneration silently reverts the edit.
- **Fix:** Edit the `.md`, re-run Step 5's render.

**Touching `.internal-docs/` directly**
- **Problem:** User's submodule HEAD moves; the parent repo shows a dirty state.
- **Fix:** All writes go through the work clone.

**Trusting the tidy greps raw**
- **Problem:** Placeholder entries and subtree-indexed docs read as drift; the "fix" mangles the README.
- **Fix:** Apply Step 6's interpretation rules before touching anything.

**Tidy pass bundled into the doc commit**
- **Problem:** Reviewer can't separate the new doc from moves and index fixes.
- **Fix:** Two commits: `chore(docs): tidy structure and index` (Step 6), then `docs(<type>): <slug>` (Step 7).
