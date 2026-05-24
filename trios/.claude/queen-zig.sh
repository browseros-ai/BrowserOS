#!/bin/zsh
# 👑 Queen Zig Brain — Trinity-style autonomous agent
# Pure shell, no Python — follows tri/doctor skill patterns

LOG="/Users/playra/BrowserOS-full/trios/.trinity/queen-zig.log"
REPO="/Users/playra/BrowserOS-full/trios"
TS=$(date +%s)

echo "[$TS] 👑 Queen Zig awakes" >> "$LOG"
cd "$REPO"

# === WAIT FOR GIT PROCESSES ===
for i in {1..10}; do
  GIT_PIDS=$(pgrep -f "git.*diff.*shortstat" | wc -l | tr -d ' ')
  if [ "$GIT_PIDS" -eq 0 ]; then break; fi
  echo "  Waiting for git diff ($GIT_PIDS procs)..." >> "$LOG"
  sleep 1
done

# === PERCEIVE ===
HEALTH=$(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o ok || echo DOWN)
BUILD=$(test -f trios_app && echo OK || echo MISSING)
DIRTY=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
AGENTS=$(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d ' ')
SKILLS=$(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d ' ')

echo "  Health=$HEALTH Build=$BUILD Dirty=$DIRTY Agents=$AGENTS Skills=$SKILLS" >> "$LOG"

# === THINK ===
ACTIONS=""
[ "$BUILD" = "MISSING" ] && ACTIONS="$ACTIONS build"
[ "$DIRTY" -gt 0 ] && ACTIONS="$ACTIONS commit"
[ "$HEALTH" = "DOWN" ] && ACTIONS="$ACTIONS restart-mcp"

echo "  Actions:$ACTIONS" >> "$LOG"

# === ACT ===

if echo "$ACTIONS" | grep -q "build"; then
  echo "  ⚡ Building..." >> "$LOG"
  ./build.sh >> "$LOG" 2>&1 || echo "    Build failed" >> "$LOG"
fi

if echo "$ACTIONS" | grep -q "commit"; then
  echo "  ⚡ Committing..." >> "$LOG"
  # Kill stale lock and wait
  rm -f /Users/playra/BrowserOS-full/.git/index.lock 2>/dev/null
  sleep 1
  git add -A 2>> "$LOG"
  git commit -m "chore(queen): auto-commit $(date +%H:%M)" >> "$LOG" 2>&1 || echo "    Commit blocked" >> "$LOG"
fi

if echo "$ACTIONS" | grep -q "restart-mcp"; then
  echo "  ⚡ Restarting MCP..." >> "$LOG"
  pm2 restart browseros-mcp >> "$LOG" 2>&1 || echo "    pm2 not available" >> "$LOG"
fi

# === REFLECT ===
echo '{"ts":'$TS',"health":"'$HEALTH'","build":"'$BUILD'","dirty":'$DIRTY',"actions":"'$ACTIONS'"}' > ".trinity/state/queen_zig.json"

echo "[$TS] 👑 Queen Zig sleeps" >> "$LOG"
echo "" >> "$LOG"
