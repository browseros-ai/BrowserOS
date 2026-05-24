#!/bin/zsh
# Queen Trinity Brain Status Dashboard
echo "== 👑 QUEEN TRINITY BRAIN STATUS =="
echo ""
echo "TRIOS: $(pgrep -x trios_app 2>/dev/null && echo RUNNING || echo STOPPED)"
echo "MCP 9105: $(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o ok || echo DOWN)"
echo "AGENT 9200: $(curl -s http://127.0.0.1:9200/health 2>/dev/null | head -c 20 || echo DOWN)"
echo "CRON PID: $(launchctl list | grep queen-cron | awk '{print $1}' || echo NONE)"
echo "AGENTS: $(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d ' ')"
echo "SKILLS: $(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d ' ')"
echo "GIT: $(git branch --show-current 2>/dev/null) $(git status --porcelain 2>/dev/null | wc -l | tr -d ' ') dirty"
echo ""
echo "Run: ./queen-status.sh"