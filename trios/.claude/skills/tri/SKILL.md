---
name: tri
description: TRI status dashboard for trios. Build health, git state, server status, agent memory. No .sh scripts per L7 UNITY.
argument-hint: [short] [full] [audit] [coverage] [lang:ru|en]
allowed-tools: fs_read, fs_write, fs_edit, shell_execute, fs_list
---

## Mode Detection

Check arguments for mode:
- If arguments contains full -> MODE=FULL
- If arguments contains short -> MODE=COMPACT
- If arguments contains audit -> MODE=AUDIT
- Otherwise -> MODE=COMPACT

## Compact Mode (~15 lines)

Quick trios health check via MCP tools:

```
shell_execute: command = "test -f /Users/playra/BrowserOS-full/trios/trios_app && echo OK || echo MISSING"
shell_execute: command = "curl -s http://127.0.0.1:9105/health"
shell_execute: command = "ls /Users/playra/BrowserOS-full/trios/.claude/agents/*.md 2>/dev/null | wc -l"
shell_execute: command = "ls /Users/playra/BrowserOS-full/trios/.claude/skills/*/SKILL.md 2>/dev/null | wc -l"
```

## Full Mode

Complete diagnostic via MCP:

1. Build Check: Verify trios_app binary
2. Git State: git status via shell_execute
3. Server Health: curl http://127.0.0.1:9105/health
4. File Inventory: find rings/ -name *.swift
5. Agent Health: queen-browseros.md check
6. Skill Health: Count loaded skills
7. Memory: .trinity/experience.md last entry
8. A2A Network: curl http://127.0.0.1:9200/a2a/registry

## Audit Mode

Deep project audit:
- Count LOC per ring layer
- Check uncommitted changes
- Verify build.sh integrity

## Trinity Compliance
- L1 TRACEABILITY: GitHub issue linkage
- L2 GENERATION: No hand-editing generated code
- L3 PURITY: ASCII-only identifiers
- L4 TESTABILITY: Build passes after edits
- L7 UNITY: No .sh/.py scripts; MCP tools only

## Report Format
```
## TRI Status Report

**Mode: {COMPACT|FULL|AUDIT}**
**Time: {timestamp}**

### Build: {PASS|FAIL|UNKNOWN}
### Git: {N} dirty files on {branch}
### Server: MCP={UP|DOWN} Agent={UP|DOWN}
### Agents: {N} active
### Skills: {N} loaded
### Next Action: {recommendation}
```
