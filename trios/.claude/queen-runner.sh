#!/bin/zsh
# Queen Runner - Trinity Style Pure Shell Brain

REPO="/Users/playra/BrowserOS-full/trios"
LOG="$REPO/.trinity/queen.log"
TS=$(date +%s)

echo "[$TS] Queen awakes" >> "$LOG"
cd "$REPO"

HEALTH=$(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o ok || echo DOWN)
BUILD=$(test -f trios_app && echo OK || echo MISSING)
AGENTS=$(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d " ")
SKILLS=$(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d " ")

echo "  Health=$HEALTH Build=$BUILD Agents=$AGENTS Skills=$SKILLS" >> "$LOG"

if [ "$BUILD" = "MISSING" ]; then
  echo "  Building..." >> "$LOG"
  ./build.sh >> "$LOG" 2>&1 || echo "  Build failed" >> "$LOG"
fi

if [ "$HEALTH" = "DOWN" ]; then
  echo "  Restarting MCP..." >> "$LOG"
  pm2 restart browseros-mcp >> "$LOG" 2>&1 || true
fi

echo "{"ts":$TS,"health":"$HEALTH","build":"$BUILD","agents":$AGENTS,"skills":$SKILLS}" > "$REPO/.trinity/state/last_wake.json"
echo "[$TS] Queen sleeps" >> "$LOG"
