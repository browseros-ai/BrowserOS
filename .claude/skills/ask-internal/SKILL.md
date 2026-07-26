---
name: ask-internal
description: Answer questions about BrowserOS internal stuff (setup, features, architecture, design decisions) by reading the private internal-docs submodule and the codebase. Use for "how do I X", "where is Y", "what is the deal with Z", or any question that mixes ops/setup knowledge with code knowledge. Can execute steps with per-command confirmation.
allowed-tools: Bash, Read, Grep, Glob
disable-model-invocation: true
---

# Ask Internal

Answer team-internal questions by reading `.internal-docs/` and the codebase, synthesizing a direct answer with file:line citations, and optionally running surfaced commands with confirmation.

**Announce at start:** "I'm using the ask-internal skill to answer this from internal-docs and the codebase."

## When to use

- "How do I reset my dogfood profile?"
- "What's the deal with the OpenClaw VM startup?"
- "Where do we configure release signing?"
- Any question whose answer lives in setup runbooks, feature notes, architecture docs, or the code that produced them.

## Hard rules — never do these

- NEVER execute a state-mutating command without per-command `y` confirmation from the user.
- NEVER edit BrowserOS code or docs in response to an ask-internal question. The skill answers; it does not write files.
- NEVER guess. If grep finds nothing useful in docs or code, say so plainly.
- NEVER run this skill if `.internal-docs/` is missing. Stop with the init command.
- NEVER cite a file or line number you have not actually read.

## Voice rules

Apply these voice rules to the synthesized answer:

- Lead with the point.
- Concrete nouns. Name files, functions, commands.
- Short sentences. Active voice. No em dashes.
- Banned words: delve, crucial, robust, comprehensive, nuanced, multifaceted, furthermore, moreover, additionally, pivotal, landscape, tapestry, underscore, foster, showcase, intricate, vibrant, fundamental, significant, leverage, utilize.
- No filler intros.

## Workflow

### Step 0: Pre-flight

```bash
if git submodule status .internal-docs 2>/dev/null | grep -q '^-'; then
  echo "internal-docs submodule not initialized. Run: git submodule update --init .internal-docs"
  exit 0
fi
[ -d .internal-docs ] && [ -n "$(ls -A .internal-docs 2>/dev/null)" ] || {
  echo ".internal-docs/ missing or empty. Submodule not configured?"
  exit 0
}
```

### Step 1: Parse the question

Pull the keywords from the user's question. Drop stop words. Identify intent:

- **Setup-question** ("how do I", "how to", "where do I configure"): bias the search toward `setup/`.
- **Feature-question** ("what is X", "why does X work this way"): bias toward `features/` and `architecture/`.
- **Free-form** ("anything about Y"): search all categories.

### Step 2: Multi-source search

Run grep in parallel across two sources.

**Internal docs:**

```bash
grep -rni --include='*.md' '<keyword>' .internal-docs/
```

Search each keyword separately. Collect top hits by relevance (more keyword matches = higher).

**Codebase (skip vendored Chromium and `node_modules`):**

```bash
grep -rni --include='*.ts' --include='*.tsx' --include='*.js' --include='*.json' --include='*.sh' \
     --exclude-dir=node_modules --exclude-dir=chromium --exclude-dir=.grove \
     '<keyword>' packages/ scripts/ .config/ .github/
```

Read the top 3-5 doc hits and top 3-5 code hits. Do not skim — read the relevant section fully so citations are accurate.

### Step 3: Synthesize answer

Structure the response:

1. **Direct answer.** First sentence answers the question. No preamble.
2. **Steps if applicable.** Numbered list with exact commands.
3. **Citations.** Every factual claim references `path/to/file.md:42` or `path/to/code.ts:117`. Run the voice self-check before printing.

If multiple docs cover the topic at different layers (e.g., a setup runbook and a feature note both mention dogfood profiles), reconcile them in the answer rather than dumping both.

### Step 4: Offer execution (only if commands surfaced)

If Step 3 produced executable commands the user could run, ask:

> Run these for you? (y / n / dry-run)

- **y:** Execute one at a time. For any command that mutates state (writes a file, modifies config, kills a process, deletes anything), ask "run this? <command>" before each. Read-only commands (`ls`, `cat`, `git status`) run without per-command confirmation but still print before running.
- **n:** Skip. Done.
- **dry-run:** Print the full sequence as a `bash` block. Do not execute.

### Step 5: Doc-not-found path

If Step 2 returned nothing useful (no doc hits AND no clear code answer):

1. Tell the user: "No doc covers this. Tangentially relevant files: <list>."
2. Ask: "Draft a short internal-doc outline in this chat?"
3. On yes: write the outline in the response only, using the code-grep findings as context. Do not create files or invoke another skill.

### Step 6: Completion status

Report one of:

- **DONE** — answer delivered, citations verified.
- **DONE_WITH_CONCERNS** — answered, but flag uncertainty (e.g., docs and code disagreed; user should reconcile).
- **BLOCKED** — submodule missing or other pre-flight failure.
- **NEEDS_CONTEXT** — question too vague to search effectively. Ask one clarifying question.

## Citation discipline

Every "X is at Y" claim in the answer must point to a file:line that the skill actually read. Do not approximate. If you didn't read it, don't cite it.

If a doc says one thing and the code says another, surface the conflict explicitly:

> The setup runbook (`setup/dogfood-profile.md:23`) says to delete `~/.cache/browseros/dogfood`, but the actual code path in `packages/cli/src/cleanup.ts:47` removes `~/.local/share/browseros/dogfood`. The doc looks stale. Recommend updating it.

## Common Mistakes

**Skimming and then citing**
- **Problem:** Citation points to a line that doesn't actually contain the claim.
- **Fix:** Read the section fully before citing. If you didn't read line 117, don't cite line 117.

**Executing without per-command confirmation for mutations**
- **Problem:** User says "y" to "run all", skill blasts through `rm -rf`-style commands.
- **Fix:** "y" means "run this sequence with per-mutation confirmations". Per-command y is required for writes.

**Searching only docs, not code**
- **Problem:** Doc says X but code does Y; answer is wrong.
- **Fix:** Always grep both sources in Step 2.

## Red Flags

**Never:**
- Cite a file:line you haven't read.
- Run mutations without per-command confirmation.
- Modify BrowserOS code or docs from this skill.

**Always:**
- Pre-flight check before any search.
- Reconcile doc vs code conflicts in the answer, don't hide them.
- Plain "no doc covers this" when grep is empty — never invent.
