---
name: t27-experience-save
description: T27 experience save for trios — records an episode to .trinity/experience/ after a task lands.
argument-hint: [task_id] [issue_url]
---

# T27 experience-save Skill (trios adaptation)

Adapted from `/Users/playra/t27/.claude/skills/experience-save.md`. Saves the outcome of a T27 loop so future agents can reuse the lesson.

## When to Invoke

Call this skill at the end of every T27 PHI LOOP after a successful land/promote.

## Inputs

Required context (gather from t27-queen or the diff):

- `task_id` — e.g. `RECURSION-001`
- `issue_url` — GitHub issue link
- `domain` — e.g. `Kernel`, `UI`, `MCP`
- `agent_chain` — list of agents involved
- `root_cause` — one paragraph
- `fix_pattern` — one paragraph
- `files_changed` — list of relative paths
- `tests_added` — list of test files
- `lessons` — bullet list of reusable insights

## Actions

1. Read `.trinity/experience.md` and `mistakes-catalog.json` if present.
2. Append a markdown summary to `.trinity/experience.md`.
3. Write a JSON episode to `.trinity/experience/YYYY-MM-DD_hh-mm-ss_{task_id}.json`.
4. Append `experience.saved` event to `.trinity/events/akashic-log.jsonl`.

## Episode Markdown Template

```markdown
## {YYYY-MM-DD} {task_id} ({domain})

- **Issue**: {issue_url}
- **Agents**: {agent_chain}
- **Root cause**: {root_cause}
- **Fix pattern**: {fix_pattern}
- **Files changed**: {files_changed}
- **Tests added**: {tests_added}
- **Lessons**:
  - {lesson 1}
  - {lesson 2}
```

## Output Format

```
## T27 experience-save
Episode saved:
- Markdown: .trinity/experience.md
- JSON: .trinity/experience/YYYY-MM-DD_hh-mm-ss_{task_id}.json
- Event: .trinity/events/akashic-log.jsonl
```

## Rules

- Append only; never delete old episodes.
- Keep summaries concise; link to full context.
- Always connect the lesson to a concrete test or file.
