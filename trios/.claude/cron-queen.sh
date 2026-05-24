#!/bin/zsh
# 👑 Queen — checks build and MCP only (avoids git lock conflicts)
LOG="/Users/playra/BrowserOS-full/trios/.trinity/cron.log"
TS=$(date +%s)

echo "[$TS] Queen waking..." >> "$LOG"
cd /Users/playra/BrowserOS-full/trios

HEALTH=$(curl -s http://127.0.0.1:9105/health | grep -o ok || echo DOWN)
BUILD=$(test -f trios_app && echo OK || echo MISSING)

echo "  Health=$HEALTH Build=$BUILD" >> "$LOG"

if [ "$BUILD" = "MISSING" ]; then
  echo "  Building..." >> "$LOG"
  ./build.sh >> "$LOG" 2>&1 || echo "  Build failed" >> "$LOG"
fi

if [ "$HEALTH" = "DOWN" ]; then
  echo "  Restarting MCP..." >> "$LOG"
  pm2 restart browseros-mcp >> "$LOG" 2>&1 || true
fi

echo '{"ts":'$TS',"health":"'$HEALTH'","build":"'$BUILD'"}' > ".trinity/state/last_wake.json"
echo "[$TS] Queen sleep" >> "$LOG"
