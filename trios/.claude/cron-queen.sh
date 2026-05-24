#!/bin/zsh
# Queen Cron — Autonomous agent lifecycle for trios
# Runs every 15 min via launchd

LOG="/Users/playra/BrowserOS-full/trios/.trinity/cron.log"
STATE="/Users/playra/BrowserOS-full/trios/.trinity/state"
mkdir -p "$STATE"

echo "[$(date +%Y-%m-%d_%H:%M:%S)] Queen waking..." >> "$LOG"

cd /Users/playra/BrowserOS-full/trios

# === 1. HEALTH CHECK ===
HEALTH=$(curl -s http://127.0.0.1:9105/health 2>/dev/null | grep -o 'ok' || echo 'DOWN')

# === 2. BUILD CHECK ===
BUILD="MISSING"
if [ -f trios_app ]; then
  BUILD="OK"
fi

# === 3. DIRTY FILES ===
DIRTY=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')

# === 4. AGENTS COUNT ===
AGENTS=$(ls .claude/agents/*.md 2>/dev/null | wc -l | tr -d ' ')

# === 5. SKILLS COUNT ===
SKILLS=$(ls .claude/skills/*/SKILL.md 2>/dev/null | wc -l | tr -d ' ')

# === 6. LAST COMMIT ===
LAST_COMMIT=$(git log --oneline -1 2>/dev/null | head -c 40 || echo "none")

# === 7. STATE SNAPSHOT (JSON for UI) ===
cat > "$STATE/last_wake.json" <<JSONEOF
{"ts":$(date +%s),"health":"$HEALTH","build":"$BUILD","dirty":$DIRTY,"agents":$AGENTS,"skills":$SKILLS,"last_commit":"$LAST_COMMIT"}
JSONEOF

# === 8. LOG ENTRY ===
echo '{"ts":'$(date +%s)',"agent":"queen-cron","action":"wake","health":"'$HEALTH'","build":"'$BUILD'","dirty":'$DIRTY'}' >> ".trinity/event_log.jsonl"

echo "[$(date +%Y-%m-%d_%H:%M:%S)] Queen sleep. Health=$HEALTH Build=$BUILD Dirty=$DIRTY Agents=$AGENTS Skills=$SKILLS" >> "$LOG"
