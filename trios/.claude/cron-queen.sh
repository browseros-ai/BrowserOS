#!/bin/zsh
# Queen Cron — Autonomous agent lifecycle for trios
# Runs every 15 min via launchd

LOG="/Users/playra/BrowserOS-full/trios/.trinity/cron.log"
STATE="/Users/playra/BrowserOS-full/trios/.trinity/state"
mkdir -p "$STATE"

echo "[$(date +%Y-%m-%d_%H:%M:%S)] Queen waking..." >> "$LOG"

cd /Users/playra/BrowserOS-full/trios

# === 1. HEALTH CHECK (every 15min) ===
HEALTH=$(curl -s http://127.0.0.1:9105/health | grep -o 'ok' || echo 'DOWN')
if [ "$HEALTH" = "DOWN" ]; then
  echo "MCP DOWN — restarting..." >> "$LOG"
  pm2 restart browseros-mcp 2>> "$LOG" || echo "pm2 not available" >> "$LOG"
fi

# === 2. BUILD CHECK ===
if [ ! -f trios_app ]; then
  echo "No binary — building..." >> "$LOG"
  ./build.sh 2>> "$LOG" || echo "BUILD_FAILED" >> "$LOG"
fi

# === 3. DIRTY FILES CHECK ===
DIRTY=$(git status --porcelain | wc -l | tr -d ' ')
if [ "$DIRTY" -gt 5 ]; then
  echo "WARNING: $DIRTY dirty files" >> "$LOG"
  # Auto-commit if build passes
  if ./build.sh 2>/dev/null; then
    git add -A
    git commit -m "chore: auto-commit dirty files ($(date +%H:%M))" >> "$LOG" 2>&1 || true
  fi
fi

# === 4. STATE SNAPSHOT ===
echo '{"ts":'$(date +%s)',"health":"'$HEALTH'","dirty":'$DIRTY',"build":"'$(test -f trios_app && echo OK || echo MISSING)'"}' > "$STATE/last_wake.json"

# === 5. LOG ENTRY ===
echo '{"ts":'$(date +%s)',"agent":"queen-cron","action":"wake","health":"'$HEALTH'","dirty":'$DIRTY'}' >> ".trinity/event_log.jsonl"

echo "[$(date +%Y-%m-%d_%H:%M:%S)] Queen sleep. Health=$HEALTH Dirty=$DIRTY" >> "$LOG"
