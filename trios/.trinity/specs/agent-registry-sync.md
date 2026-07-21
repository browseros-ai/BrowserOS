# Agent registry sync specification

## Scope
Keep `.claude/agents/registry.json` consistent with the actual agent markdown files on disk.

## Invariants
1. Every `agents[]` entry must reference a file that exists.
2. No duplicate agent names.
3. Optional: every `.claude/agents/*.md` file should be registered (future CI check).

## Interface
Registry is read-only at runtime. Validation is a CI/pre-commit check.

## Fix for this wave
Remove the `agent-H` entry because `.claude/agents/agent-H.md` does not exist.

## Tests
- `test -f .claude/agents/agent-H.md` returns false.
- `python -c "import json; ..."` confirms no missing files in registry.

## Change flow
All changes must be justified by this spec. Emergency hand edits require an `// AGENT-V-WAIVER:` block.
