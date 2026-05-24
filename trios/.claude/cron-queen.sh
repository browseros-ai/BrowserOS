#!/bin/zsh
# 👑 Queen Cron — Pure Shell Brain (Zig/Trinity style)
# No Python! No trinity-w1! Only ~/BrowserOS-full/trios

LOG="/Users/playra/BrowserOS-full/trios/.trinity/cron.log"
REPO="/Users/playra/BrowserOS-full/trios"
TS=$(date +%s)

echo "[$TS] 👑 Queen waking..." >> "$LOG"
cd "$REPO"

# === PERCEIVE ===
HEALTH=$(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o ok || echo DOWN)
BUILD=$(test -f trios_app && echo OK || echo MISSING)
AGENTS=$(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d " ")
SKILLS=$(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d " ")

echo "  Health=$HEALTH Build=$BUILD Agents=$AGENTS Skills=$SKILLS" >> "$LOG"

# === THINK ===
ACTIONS=""
[ "$BUILD" = "MISSING" ] && ACTIONS="$ACTIONS build"
[ "$HEALTH" = "DOWN" ] && ACTIONS="$ACTIONS restart-mcp"

# === ACT ===
if echo "$ACTIONS" | grep -q "build"; then
  echo "  ⚡ Building..." >> "$LOG"
  ./build.sh >> "$LOG" 2>&1 || echo "  ❌ Build failed" >> "$LOG"
fi

if echo "$ACTIONS" | grep -q "restart-mcp"; then
  echo "  ⚡ Restarting MCP..." >> "$LOG"
  pm2 restart browseros-mcp >> "$LOG" 2>&1 || true
fi

# === REFLECT ===
echo "{"ts":$TS,"health":"$HEALTH","build":"$BUILD","agents":$AGENTS,"skills":$SKILLS}" > "$REPO/.trinity/state/last_wake.json"
echo "[$TS] 👑 Queen sleep" >> "$LOG"
