#!/bin/zsh
# 👑 Queen Runner — The Best of Both Worlds
# Phase 1: Zig-style shell (fast, no lock conflicts)
# Phase 2: Python brain (only when git is free)

REPO="/Users/playra/BrowserOS-full/trios"
LOG="$REPO/.trinity/queen.log"
TS=$(date +%s)

echo "
[$TS] ════════ 👑 QUEEN AWAKES ════════" >> "$LOG"
cd "$REPO"

# ════════ PHASE 1: SHELL PERCEPTION (Zig-style) ════════
HEALTH=$(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o ok || echo DOWN)
BUILD=$(test -f trios_app && echo OK || echo MISSING)

echo "[$TS] 👁  PERCEIVE: health=$HEALTH build=$BUILD" >> "$LOG"

# ════════ PHASE 2: CHECK GIT LOCK ════════
GIT_BUSY=0
if [ -f /Users/playra/BrowserOS-full/.git/index.lock ]; then
  AGE=$(( $(date +%s) - $(stat -f %m /Users/playra/BrowserOS-full/.git/index.lock 2>/dev/null || echo 0) ))
  if [ "$AGE" -gt 30 ]; then
    echo "[$TS] 🔓 Stale lock detected ($AGE sec), removing..." >> "$LOG"
    rm -f /Users/playra/BrowserOS-full/.git/index.lock 2>/dev/null
  else
    GIT_BUSY=1
    echo "[$TS] ⏳ Git lock active ($AGE sec), skipping git ops" >> "$LOG"
  fi
fi

# ════════ PHASE 3: ACT (Shell or Python) ════════

# Always-safe actions
if [ "$BUILD" = "MISSING" ]; then
  echo "[$TS] ⚡ ACT: Building..." >> "$LOG"
  ./build.sh >> "$LOG" 2>&1 || echo "[$TS] ❌ Build failed" >> "$LOG"
fi

if [ "$HEALTH" = "DOWN" ]; then
  echo "[$TS] ⚡ ACT: Restarting MCP..." >> "$LOG"
  pm2 restart browseros-mcp >> "$LOG" 2>&1 || true
fi

# Git-dependent actions (only when lock is free)
if [ "$GIT_BUSY" -eq 0 ]; then
  echo "[$TS] 🐍 Delegating to Python brain for git ops..." >> "$LOG"
  python3 "$REPO/.claude/queen-agent.py" >> "$LOG" 2>&1
else
  echo "[$TS] ⏭️  Skipping Python brain (git busy)" >> "$LOG"
fi

# ════════ PHASE 4: REFLECT ════════
echo "[$TS] 📝 Saving state..." >> "$LOG"
echo "{"ts":$TS,"health":"$HEALTH","build":"$BUILD","git_busy":$GIT_BUSY}" > "$REPO/.trinity/state/queen.json"

echo "[$TS] ════════ 👑 QUEEN SLEEPS ════════" >> "$LOG"
