---
name: god-mode
description: GOD MODE - Agent oversight dashboard for trios. No .sh/.py scripts per L7 UNITY.
argument-hint: [status|agents|tasks|violations|health]
allowed-tools: fs_read, fs_write, fs_edit, shell_execute, fs_list
---

# GOD MODE - Agent Oversight Dashboard

## L7 UNITY Compliance
No ad-hoc .sh/.py scripts. Use MCP tools or tri CLI only.

## Swarm Status (via MCP tools)

```
shell_execute: "ls /Users/playra/BrowserOS-full/trios/.claude/agents/*.md 2>/dev/null | wc -l"
shell_execute: "ls /Users/playra/BrowserOS-full/trios/.claude/skills/*/SKILL.md 2>/dev/null | wc -l"
shell_execute: "curl -s http://127.0.0.1:9105/health"
```

## Agent Status
shell_execute: "pgrep -la trios_app 2>/dev/null || echo trios_app: not running"
shell_execute: "curl -s http://127.0.0.1:9105/health | head -c 50 || echo MCP: DOWN"

## Git Activity
shell_execute: "cd /Users/playra/BrowserOS-full/trios && git log --oneline -10 --all --graph"
shell_execute: "cd /Users/playra/BrowserOS-full/trios && git branch -a | head -10"

## Rule Violations
- Dirty .swift files without commit -> WARNING
- Build broken > 30 min -> CRITICAL
- MCP server down > 5 min -> CRITICAL

## Report Format
```
## GOD MODE Report

**Status: {HEALTHY|WARNING|CRITICAL}**

### Agents
- {N} definitions, {N} healthy

### Build
- Last: {PASS|FAIL} at {time}

### Violations
- {list or None}

### Actions
- {recommendations}
```

## Trinity Compliance
- L1 TRACEABILITY: GitHub issue linkage
- L3 PURITY: ASCII-only identifiers
- L4 TESTABILITY: Build verification
- L7 UNITY: No .sh/.py scripts
