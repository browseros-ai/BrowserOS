# TRIOS MCP Bridge — BrowserOS Integration Guide

## Step 1: Start the bridge

```bash
cd packages/browseros-agent
bun run start:trios-bridge
# Bridge starts at http://127.0.0.1:9200
```

## Step 2: Register in BrowserOS

Open BrowserOS Settings → MCP Servers → Add Custom Server:

- **Name**: TRIOS MCP Bridge
- **URL**: `http://127.0.0.1:9200/mcp`
- **Transport**: Streamable HTTP

Or use the registration config:
```bash
# The bridge's MCP registration is at:
# apps/trios-mcp-bridge/mcp-registration.json
```

## Step 3: Verify

In BrowserOS chat, ask:
> "Check the TRIOS bridge health"

The agent should call `gitbutler_bridge_health` and report:
- ✅ BrowserOS MCP: Connected
- ✅ GitButler CLI: Available
- ✅ GitButler MCP: Connected

## Step 4: Demo

> "Create branch 'feature-login' and commit what's visible in GitButler"

Agent flow:
1. `gitbutler_analyze_ui` → sees current state
2. `gitbutler_create_branch` → creates `feature-login`
3. `gitbutler_commit_visible` → commits changes
4. GitButler UI updates automatically

## 11 Available Tools

| Tool | Description |
|------|-------------|
| `gitbutler_analyze_ui` | Screenshot + analyze GitButler UI state |
| `gitbutler_screenshot` | Raw screenshot of GitButler tab |
| `gitbutler_workspace_status` | Detailed file/branch status |
| `gitbutler_bridge_health` | Health check for connections |
| `gitbutler_commit_visible` | Commit changed files |
| `gitbutler_create_branch` | Create virtual branch |
| `gitbutler_push_stack` | Push stack to remote |
| `gitbutler_stage` | Stage specific files |
| `gitbutler_absorb` | Smart absorb into commits |
| `gitbutler_pull` | Pull latest changes |
| `gitbutler_undo_last_commit` | Undo last commit |
