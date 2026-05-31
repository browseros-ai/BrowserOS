#!/bin/bash
# TRIOS E2E Self-Test Flow
# Run: bash e2e/trios_e2e_flow.sh
# This script checks server health, app status, takes screenshots, and scans logs.

set -e
LOG_DIR="/tmp/trios_e2e"
mkdir -p "$LOG_DIR"
TIMESTAMP=$(date +%s)
REPORT="$LOG_DIR/report_${TIMESTAMP}.md"

echo "# TRIOS E2E Report $(date)" > "$REPORT"

# --- 1. Server Health ---
HEALTH=$(curl -s http://127.0.0.1:9105/health 2>&1 || echo "FAIL")
if echo "$HEALTH" | grep -q '"status":"ok"'; then
    echo "- ✅ BrowserOS Server: OK ($HEALTH)" >> "$REPORT"
else
    echo "- ❌ BrowserOS Server: DOWN ($HEALTH)" >> "$REPORT"
fi

# --- 2. App Running ---
PID=$(pgrep -f "trios.app/Contents/MacOS/trios" || true)
if [ -n "$PID" ]; then
    echo "- ✅ Trios App: PID $PID" >> "$REPORT"
else
    echo "- ❌ Trios App: NOT RUNNING — restarting..." >> "$REPORT"
    pkill -f trios_app 2>/dev/null || true
    sleep 1
    open /Users/playra/BrowserOS-full/trios/trios.app
    sleep 3
fi

# --- 3. Screenshot ---
screencapture -x "$LOG_DIR/screenshot_${TIMESTAMP}.png"
echo "- 📸 Screenshot: $LOG_DIR/screenshot_${TIMESTAMP}.png" >> "$REPORT"

# --- 4. Log Errors (last 5 min) ---
ERRORS=$(log show --predicate 'process == "trios"' --last 5m --style compact 2>/dev/null | grep -iE "timed out|TransportError|crash|fatal|error" | tail -5 || true)
if [ -n "$ERRORS" ]; then
    echo "- ⚠️ Recent Errors:" >> "$REPORT"
    echo "\`\`\`" >> "$REPORT"
    echo "$ERRORS" >> "$REPORT"
    echo "\`\`\`" >> "$REPORT"
else
    echo "- ✅ No critical errors in last 5m" >> "$REPORT"
fi

# --- 5. UI Anomaly Checklist ---
echo "" >> "$REPORT"
echo "## UI Anomaly Checklist (verify against screenshot)" >> "$REPORT"
echo "- [ ] Title bar shows correct status (Online green dot, A2A blue dot)" >> "$REPORT"
echo "- [ ] Tab bar icons visible and not duplicated (Chat/Git/Terminal/Queen/Settings)" >> "$REPORT"
echo "- [ ] Chat input field visible at bottom with placeholder 'Ask anything...'" >> "$REPORT"
echo "- [ ] No overlapping views, no black rectangles, no glitched rendering" >> "$REPORT"
echo "- [ ] Glassmorphism blur visible behind panel content" >> "$REPORT"
echo "- [ ] Messages scroll correctly without cutting off bubbles" >> "$REPORT"
echo "- [ ] No duplicate headers or buttons outside tab bar" >> "$REPORT"

echo "$REPORT"
