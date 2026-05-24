## E2E Testing Skill for trios

### Enable Accessibility (Required)
1. System Settings > Privacy & Security > Accessibility
2. Add /Users/playra/BrowserOS-full/trios/trios_app
3. Enable checkbox
4. Restart trios_app

### Test via MCP API (No UI needed)
```
curl -s http://127.0.0.1:9105/health
curl -X POST http://127.0.0.1:9105/mcp -H "Content-Type: application/json" -d JSON_RPC_PAYLOAD
```

### CGEvent Mouse Simulation (Swift)
Use CoreGraphics CGEvent for low-level mouse events

### E2E Test Scenarios
1. Launch trios > verify status bar icon
2. Click status bar > verify panel opens
3. Type message > verify ViewModel receives it
4. Send command > verify MCP health passes
5. Switch to BrowserOS tab > verify view renders