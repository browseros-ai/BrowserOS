## E2E Testing - BrowserOS Agent Autonomous Mode

### What Agent Can Test Automatically
1. **MCP API** - Health, shell, navigate, screenshot [U+2705] AUTONOMOUS
2. **Business Logic** - Intent parsing, ViewModel state [U+2705] AUTONOMOUS
3. **Git Integration** - Commit, push, branch [U+2705] AUTONOMOUS

### What Requires Human (macOS Security)
1. **UI Click on Status Bar** - Needs Accessibility grant
2. **Panel Interaction** - Needs Accessibility grant
3. **System Settings** - Requires user interaction

### Solution: Hybrid Approach
```
BrowserOS Agent (me) handles:
  -> MCP API testing
  -> Business logic verification
  -> Git operations
  -> File system operations

Human handles:
  -> Initial Accessibility grant (one time)
  -> UI panel verification (visual check)
  -> Complex gestures
```

### After Accessibility Grant, Agent Can
1. Open panel via AppleScript/CGEvent
2. Click BrowserOS tab
3. Type test messages
4. Capture screenshots for verification
5. Full E2E cycle autonomous

### Grant Command
```bash
# Run trios_app with Accessibility prompt
cd /Users/playra/BrowserOS-full/trios
./trios_app
# Then in System Settings > Privacy > Accessibility > Enable trios_app
```