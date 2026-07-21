# Session Longevity + Chat UI/UX Optimization
## Problem
Temporary sessions = lost context + wasted tokens. Agent dies every session end.

## Solution: SessionGuard Actor
Swift actor keeps session alive via proactive health pings.

## Chat UI/UX Best Practices
1. Message bubbles: User right/accent, Agent left/glass, System center/monospace
2. Streaming: Character reveal 16ms, typing indicator before first token
3. Tool cards: Expandable, icon+status badge, duration timer
4. Session UI: Active timer, token counter, auto-save indicator

## Reverse River Architecture
trios SwiftUI sends commands TO BrowserOS via MCP. BrowserOS executes. Results stream back as native components. Zero web UI visible.

## Agent Vitality Ritual (every 2 min)
1. Ping MCP health endpoint
2. Update activity timestamp
3. Check token budget
4. Auto-save session state
5. If idle > 4 min: send heartbeat

## Session Resurrection
On new session: query NotebookLM context, load last_wake.json, present summary, ask continue or fresh.

## Rules
- MUST invoke before complex multi-step tasks
- No .sh/.py - Swift actors + MCP tools only
- Auto-trigger wrap-up before session end
