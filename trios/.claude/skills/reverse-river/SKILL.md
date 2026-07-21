## Reverse River Architecture

### Problem
Currently chatting IN BrowserOS web UI.

### Solution
Control BrowserOS FROM trios (SwiftUI native app).

### Architecture
```
trios (SwiftUI) -> ChatViewModel -> HTTP/SSE -> MCP Server (9105) -> BrowserOS Agent (Chromium)
```

### Components
1. **TriosMCPClient** - Swift actor, HTTP client
2. **ChatViewModel+BrowserOS** - MCP integration layer
3. **BrowserOSBridgeView** - Reverse control panel UI
4. **MessageSegment+BrowserOS** - Tool call rendering

### Workflow
1. User types in trios SwiftUI chat
2. Message goes to ChatViewModel
3. ViewModel decides: direct response or BrowserOS action
4. If BrowserOS needed: call MCP tool via HTTP
5. BrowserOS executes (navigate, click, extract)
6. Results stream back as SSE events
7. SwiftUI renders results as native tool cards

### Benefits
- Pure native UI (no web chat visible)
- Faster than web rendering
- Better macOS integration (hotkeys, gestures)
- Queen agent can orchestrate via native UI

### No .sh/.py Rule
All integration via Swift + HTTP. No shell scripts or Python.