---
name: tri
description: TRI status dashboard for trios — build health, git state, server status, agent memory. Compact mode by default, /tri full for full diagnostic.
argument-hint: [short] [full] [audit] [coverage] [lang:ru|en]
allowed-tools: Bash(ls *), Bash(wc *), Bash(grep *), Bash(cat *), Bash(find *), Bash(git *), Bash(date *), Bash(test *), Bash(tail *), Bash(echo *), Bash(curl *), Read, Edit, Write
---

## Mode Detection

Check arguments for mode:
- If arguments contains "full" → **MODE=FULL**
- If arguments contains "short" → **MODE=COMPACT**
- If arguments contains "audit" → **MODE=AUDIT**
- Otherwise → **MODE=COMPACT**

## Compact Mode (~15 lines)

Quick trios health check:

```bash
cd /Users/playra/BrowserOS-full/trios
echo "=== TRIOS STATUS ==="
echo "Build: $(test -f trios_app && echo '✅ binary exists' || echo '❌ no binary')"
echo "Git: $(git status --short | wc -l | tr -d ' ') dirty files"
echo "Branch: $(git branch --show-current)"
echo "Last commit: $(git log --oneline -1 | head -c 50)"
echo "Server health: $(curl -s http://127.0.0.1:9105/health | grep -o 'ok' || echo 'DOWN')"
echo "MCP tools: $(curl -s http://127.0.0.1:9105/tools/list | grep -o 'name' | wc -l | tr -d ' ') registered"
echo "Agents: $(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d ' ') definitions"
echo "Skills: $(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d ' ') loaded"
echo "Rings: $(find rings -name '*.swift' | wc -l | tr -d ' ') Swift files"
echo "BR-OUTPUT: $(find BR-OUTPUT -name '*.swift' 2>/dev/null | wc -l | tr -d ' ') UI files"
echo "A2A registry: $(curl -s http://127.0.0.1:9200/a2a/registry | grep -o 'agent' | wc -l | tr -d ' ') agents"
```

## Full Mode

Complete diagnostic:

1. **Build Check**: Verify trios_app binary, run ./build.sh if needed
2. **Git State**: Full git status, branch, recent commits
3. **Server Health**: MCP server (9105), Agent server (9200)
4. **File Inventory**: All .swift files by ring layer
5. **Agent Health**: queen-browseros, other agent definitions
6. **Skill Health**: All loaded skills, check for orphans
7. **Memory**: .trinity/experience.md last entry
8. **A2A Network**: /a2a/registry health

## Audit Mode

Deep project audit:
- Count LOC per ring layer
- Check for uncommitted changes
- Verify build.sh integrity
- Check for unused imports

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