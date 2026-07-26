# browseros-cli

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Command-line interface for controlling BrowserOS — launch and automate the browser from the terminal or from AI coding agents like Claude Code and Gemini CLI. The installed `bos` command is a short alias for `browseros-cli`.

Communicates with the BrowserOS MCP server over JSON-RPC 2.0 / StreamableHTTP and maps the core BrowserOS automation tools to CLI commands.

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
# If BrowserOS is not installed yet, download it from https://browseros.com

# If BrowserOS is installed but not running
browseros-cli launch                 # opens BrowserOS, waits for server

# Configure the CLI with the Server URL from BrowserOS settings
browseros-cli init http://127.0.0.1:9000/mcp

# Verify connection
browseros-cli health
```

## Agent workflow

Run `browseros-cli --llm-txt` for a concise, copy-pasteable agent guide to the whole CLI (printed
from the binary, so it always matches the installed version).

Agents should capture a page id from `open` or `tabs`, then pass it explicitly with `-p`.

```bash
page=$(browseros-cli open --json https://example.com | jq -r .page)
browseros-cli -p "$page" snapshot
browseros-cli -p "$page" read --links
browseros-cli -p "$page" find text "Search" click
browseros-cli -p "$page" press Enter
browseros-cli -p "$page" snapshot
browseros-cli -p "$page" close
```

### Other init modes

```bash
browseros-cli init <url>             # non-interactive — pass URL directly
browseros-cli init                   # interactive — prompts for URL
```

Config is saved to `~/.config/browseros-cli/config.yaml`. If `browseros-cli health` cannot connect, copy the current Server URL from BrowserOS Settings > BrowserOS MCP and run `browseros-cli init <Server URL>` again.

### CLI updates

The CLI checks for a newer BrowserOS CLI release in the background about once per day and will suggest an update on a later run when one is available.

```bash
browseros-cli update         # check and apply the latest CLI release
browseros-cli update --check # check only
browseros-cli update --yes   # apply without prompting
```

### Release flow

CLI releases are cut from annotated git tags. The tag is the source of truth for the release version; do not commit a version bump or add a checked-in version file.

```bash
git tag -a cli/v0.2.3 -m "browseros-cli v0.2.3"
git push origin cli/v0.2.3
```

Pushing `cli/vX.Y.Z` starts the CLI release workflow. The workflow rejects tags that are not newer than production latest or whose target commit is not reachable from the repository default branch.

The `NPM_TOKEN` release secret must authenticate as an npm owner of `browseros-cli`; the workflow checks this before uploading CDN assets or creating the GitHub release.

Inspect versions with:

```bash
browseros-cli --version
curl -fsSL https://cdn.browseros.com/cli/latest/version.txt
curl -fsSL https://cdn.browseros.com/cli/latest/manifest.json
git tag -l 'cli/v*' --sort=-v:refname
git tag -l 'browseros-cli-v*' --sort=-v:refname
```

## Usage

```bash
# Check connection
browseros-cli health
browseros-cli status

# Tabs
browseros-cli tabs                  # List all tabs
browseros-cli active                # Show active tab
browseros-cli open --json https://example.com
browseros-cli -p 42 close

# Navigation
browseros-cli -p 42 nav https://example.com
browseros-cli -p 42 back
browseros-cli -p 42 forward
browseros-cli -p 42 reload

# Observation
browseros-cli -p 42 snapshot        # Accessibility tree snapshot
browseros-cli -p 42 read            # Extract page as markdown
browseros-cli -p 42 read --links    # Extract all links
browseros-cli -p 42 grep "Submit"   # Search snapshot lines
browseros-cli -p 42 eval "document.title" # Run JavaScript

# Input
browseros-cli -p 42 click @e5       # Click element by ref
browseros-cli -p 42 click-at 100 200
browseros-cli -p 42 fill @e12 "hello"
browseros-cli -p 42 press Enter
browseros-cli -p 42 type "hello"
browseros-cli -p 42 find role button --name "Submit" click
browseros-cli -p 42 hover @e3
browseros-cli -p 42 scroll down 500

# Screenshots & export
browseros-cli -p 42 screenshot
browseros-cli -p 42 screenshot -o shot.png
browseros-cli -p 42 pdf page.pdf

# Batch multiple steps through one MCP session
browseros-cli -p 42 batch --bail "find role searchbox fill query" "press Enter"

# Resource management (grouped commands)
browseros-cli window list
browseros-cli bookmark search "github"
browseros-cli history recent
browseros-cli group list
```

`batch` supports page-scoped browser steps that map directly to MCP calls: `nav`, `back`, `forward`, `reload`, `eval`, `snapshot`, `read`, `grep`, `find`, `click`, `fill`, `press`, `type`, `hover`, `check`, `uncheck`, `focus`, and `select`.

## Use as MCP Server

BrowserOS exposes an MCP server that AI coding agents can connect to directly. The CLI is the easiest way to verify the connection and interact with tools from the terminal.

To connect Claude Code, Gemini CLI, or any MCP client, see the [MCP setup guide](https://docs.browseros.com/features/use-with-claude-code).

## Global Flags

| Flag | Env Var | Description |
|------|---------|-------------|
| `--server, -s` | `BROWSEROS_URL` | Server URL (default: from config) |
| `--page, -p` | | Required page ID for page-scoped commands |
| `--json` | `BOS_JSON=1` | JSON output (outputs structuredContent) |
| `--debug` | `BOS_DEBUG=1` | Debug output |
| `--timeout, -t` | | Request timeout (default: 2m) |

Priority for server URL: `--server` flag > `BROWSEROS_URL` env > config file

If no server URL is configured, the CLI exits with setup instructions pointing to `launch` and `init <Server URL>`.

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
- Page lifecycle: open → read → snapshot → eval → screenshot → nav → reload → close
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
│   ├── init.go         # Server URL configuration (URL arg or interactive)
│   ├── launch.go       # launch (find and start BrowserOS, wait for server)
│   ├── open.go         # open (new_page / new_hidden_page)
│   ├── nav.go          # nav, back, forward, reload
│   ├── tabs.go         # tabs/pages alias, active, close
│   ├── snap.go         # snapshot/snap
│   ├── text.go         # read, text, links, grep
│   ├── find.go         # find (grep + act)
│   ├── batch.go        # batch command runner
│   ├── screenshot.go   # screenshot/ss
│   ├── eval.go         # eval (evaluate_script)
│   ├── click.go        # click, click-at
│   ├── fill.go         # fill, clear, key
│   ├── interact.go     # hover, focus, check, uncheck, select, drag, upload
│   ├── scroll.go       # scroll
│   ├── wait.go         # wait (wait_for)
│   ├── file_actions.go # pdf, download
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

Normal CLI commands initialize an MCP session, call the requested tool, and close the session. `batch` keeps one MCP session open for all subcommands in that invocation.

## Links

- [Documentation](https://docs.browseros.com)
- [MCP Setup Guide](https://docs.browseros.com/features/use-with-claude-code)
- [Changelog](./CHANGELOG.md)
