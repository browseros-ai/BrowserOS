---
name: bridge
description: BrowserOS Bridge - MCP tool bridge for AI agents. Connects trios to BrowserOS MCP server, provides fs_read/shell_execute access.
argument-hint: [status|test|mcp]
allowed-tools: Bash(curl *), Bash(cat *), Bash(echo *), Bash(date *), Read
---

# BROWSEROS BRIDGE - MCP Access for AI Agents

## Architecture
```
AI Agent (Queen BrowserOS)
    |
    | POST 127.0.0.1:9105/mcp
    v
BrowserOS MCP Server (port 9105)
    |
    | fs_read, fs_write, shell_execute, fs_list
    v
Your Mac (Dmitrii's MacBook)
```

## MCP Endpoints
- **Health**: GET http://127.0.0.1:9105/health
- **Tools list**: POST http://127.0.0.1:9105/mcp (method: tools/list)
- **Tool call**: POST http://127.0.0.1:9105/mcp (method: tools/call)

## Key Tools
| Tool | Purpose |
|------|---------|
| fs_read | Read any file on Mac |
| fs_write | Write files |
| fs_edit | Targeted edits |
| fs_list | Directory listing |
| shell_execute | Run shell commands |

## Usage
```bash
# Test MCP
curl -s http://127.0.0.1:9105/health

# List tools
curl -X POST http://127.0.0.1:9105/mcp   -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# Read file
curl -X POST http://127.0.0.1:9105/mcp   -d '{"method":"tools/call","params":{"name":"fs_read","arguments":{"path":"/Users/playra/BrowserOS-full/trios/main.swift"}}}'
```
