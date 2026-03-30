# browseros-cli

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Command-line interface for controlling BrowserOS — launch and automate the browser from the terminal or from AI coding agents like Claude Code and Gemini CLI.

Communicates with the BrowserOS MCP server over JSON-RPC 2.0 / StreamableHTTP. All 53+ MCP tools are mapped to CLI commands.

## Install

### macOS / Linux

```bash
curl -fsSL https://cdn.browseros.com/cli/install.sh | bash
```

### Windows

```powershell
irm https://cdn.browseros.com/cli/install.ps1 | iex
```

### Build from Source

Requires Go 1.25+.

```bash
make            # Build binary
make install    # Install to $GOPATH/bin
```

## Quick Start

```bash
# If BrowserOS is not installed yet
browseros-cli install                # downloads BrowserOS for your platform

# If BrowserOS is installed but not running
browseros-cli launch                 # opens BrowserOS, waits for server

# Configure the CLI (auto-discovers running BrowserOS)
browseros-cli init --auto            # detects server URL and saves config

# Verify connection
browseros-cli health
```

### Other init modes

```bash
browseros-cli init <url>             # non-interactive — pass URL directly
browseros-cli init                   # interactive — prompts for URL
```

Config is saved to `~/.config/browseros-cli/config.yaml`. The CLI also auto-discovers the server from `~/.browseros/server.json` (written by BrowserOS on startup).

### CLI updates

The CLI checks for a newer BrowserOS CLI release in the background about once per day and will suggest an update on a later run when one is available.

```bash
browseros-cli update         # check and apply the latest CLI release
browseros-cli update --check # check only
browseros-cli update --yes   # apply without prompting
```

## Usage

```bash
# Check connection
browseros-cli health
browseros-cli status

# Pages
browseros-cli pages                 # List all tabs
browseros-cli active                # Show active tab
browseros-cli open https://example.com
browseros-cli close 42

# Navigation
browseros-cli nav https://example.com
browseros-cli back
browseros-cli forward
browseros-cli reload

# Observation
browseros-cli snap                  # Accessibility tree snapshot
browseros-cli snap -e               # Enhanced snapshot
browseros-cli text                  # Extract page as markdown
browseros-cli links                 # Extract all links
browseros-cli eval "document.title" # Run JavaScript

# Input
browseros-cli click e5              # Click element by ref
browseros-cli click-at 100 200      # Click at coordinates
browseros-cli fill e12 "hello"      # Type into input
browseros-cli key Enter             # Press key
browseros-cli hover e3
browseros-cli scroll down 500

# Screenshots & export
browseros-cli ss                    # Screenshot (saves to screenshot.png)
browseros-cli ss -o shot.png        # Screenshot to specific file
browseros-cli pdf -o page.pdf       # Export as PDF

# Resource management (grouped commands)
browseros-cli window list
browseros-cli bookmark search "github"
browseros-cli history recent
browseros-cli group list
```

## Use as MCP Server

BrowserOS exposes an MCP server that AI coding agents can connect to directly. The CLI is the easiest way to verify the connection and interact with tools from the terminal.

To connect Claude Code, Gemini CLI, or any MCP client, see the [MCP setup guide](https://docs.browseros.com/features/use-with-claude-code).

## Global Flags

| Flag | Env Var | Description |
|------|---------|-------------|
| `--server, -s` | `BROWSEROS_URL` | Server URL (default: from config) |
| `--page, -p` | `BROWSEROS_PAGE` | Target page ID (default: active page) |
| `--json` | `BOS_JSON=1` | JSON output (outputs structuredContent) |
| `--debug` | `BOS_DEBUG=1` | Debug output |
| `--timeout, -t` | | Request timeout (default: 2m) |

Priority for server URL: `--server` flag > `BROWSEROS_URL` env > `~/.browseros/server.json` > config file

If no server URL is configured, the CLI exits with setup instructions pointing to `install`, `launch`, and `init`.

## Testing

Integration tests require a running BrowserOS server with the dev build (for structured content support).

```bash
# 1. Start the dev server from the monorepo root
bun run dev:watch:new

# 2. Configure the CLI to point at the dev server
./browseros-cli init
# Enter the Server URL shown in BrowserOS settings

# 3. Run integration tests
make test

# Or with a custom server URL
BROWSEROS_URL=http://127.0.0.1:9105 go test -tags integration -v ./...
```

Tests skip gracefully if no server is reachable — they won't fail in environments without BrowserOS.

The integration tests (`integration_test.go`) cover:
- Health check and version
- Page lifecycle: open → text → snap → eval → screenshot → nav → reload → close
- Active page query
- Info command
- Error handling (invalid page ID, JS errors)

## Build

```bash
make                    # Build binary
make vet                # Run go vet
make test               # Run integration tests
make install            # Install to $GOPATH/bin
make clean              # Remove binary
VERSION=1.0 make        # Build with version
```

## Architecture

```
apps/cli/
├── main.go             # Entry point
├── Makefile            # Build targets
├── config/
│   └── config.go       # Config file (~/.config/browseros-cli/config.yaml)
├── cmd/
│   ├── root.go         # Root command, global flags
│   ├── init.go         # Server URL configuration (URL arg, --auto, interactive)
│   ├── install.go      # install (download BrowserOS for current platform)
│   ├── launch.go       # launch (find and start BrowserOS, wait for server)
│   ├── open.go         # open (new_page / new_hidden_page)
│   ├── nav.go          # nav, back, forward, reload
│   ├── pages.go        # pages, active, close
│   ├── snap.go         # snap (take_snapshot / take_enhanced_snapshot)
│   ├── text.go         # text, links
│   ├── screenshot.go   # ss (take_screenshot / save_screenshot)
│   ├── eval.go         # eval (evaluate_script)
│   ├── click.go        # click, click-at
│   ├── fill.go         # fill, clear, key
│   ├── interact.go     # hover, focus, check, uncheck, select, drag, upload
│   ├── scroll.go       # scroll
│   ├── dialog.go       # dialog (handle_dialog)
│   ├── wait.go         # wait (wait_for)
│   ├── file_actions.go # pdf, download
│   ├── dom.go          # dom, dom-search
│   ├── window.go       # window {list,create,close,activate}
│   ├── bookmark.go     # bookmark {list,create,remove,update,move,search}
│   ├── history.go      # history {search,recent,delete,delete-range}
│   ├── group.go        # group {list,create,update,ungroup,close}
│   ├── health.go       # health, status (REST endpoints)
│   └── info.go         # info (browseros_info)
├── mcp/
│   ├── client.go       # MCP JSON-RPC 2.0 client (initialize + tools/call)
│   └── types.go        # JSON-RPC and MCP type definitions
└── output/
    └── printer.go      # Human-readable and JSON output formatting
```

The CLI communicates with BrowserOS via two HTTP POST requests per command:
1. `initialize` — MCP handshake
2. `tools/call` — execute the actual tool

## Links

- [Documentation](https://docs.browseros.com)
- [MCP Setup Guide](https://docs.browseros.com/features/use-with-claude-code)
- [Changelog](./CHANGELOG.md)
