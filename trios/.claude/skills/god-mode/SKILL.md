---
name: god-mode
description: GOD MODE — Agent oversight dashboard for trios. Shows agent status, build health, circuit breakers, git activity, rule violations.
argument-hint: [status|agents|tasks|violations|health]
allowed-tools: Bash(git *), Bash(cat *), Bash(ls *), Bash(grep *), Bash(wc *), Bash(curl *), Read, Edit, Write
---

# GOD MODE — Agent Oversight Dashboard

## Swarm Status
- Agent definitions: .claude/agents/*.md
- Skill definitions: .claude/skills/*/SKILL.md
- Active processes: trios_app, bun server, browseros-mcp

## Agent Status
```bash
cd /Users/playra/BrowserOS-full/trios
echo "=== AGENTS ==="
ls .claude/agents/*.md 2>/dev/null | xargs -n1 basename -s .md
echo "=== SKILLS ===" 
ls .claude/skills/*/SKILL.md 2>/dev/null | xargs dirname | xargs basename
echo "=== PROCESSES ==="
pgrep -la trios_app 2>/dev/null || echo "trios_app: not running"
curl -s http://127.0.0.1:9105/health | head -c 50 || echo "MCP: DOWN"
curl -s http://127.0.0.1:9200/health | head -c 50 2>/dev/null || echo "Agent: DOWN"
```

## Git Activity
```bash
cd /Users/playra/BrowserOS-full/trios
git log --oneline -10 --all --graph
git branch -a | head -10
```

## Rule Violations
- Dirty .swift files without commit → WARNING
- Build broken > 30 min → CRITICAL
- MCP server down > 5 min → CRITICAL
- No wrap-up in last session → WARNING

## Circuit Breaker State
- trios_app crash count
- Build failure streak
- Auto-recovery attempts

## Report Format
```
## GOD MODE Report

**Status: {HEALTHY|WARNING|CRITICAL}**

### Agents
- {N} definitions, {N} healthy

### Build
- Last: {PASS|FAIL} at {time}

### Violations
- {list or "None"}

### Actions
- {recommendations}
``
