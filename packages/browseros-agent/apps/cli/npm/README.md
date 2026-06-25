# browseros-cli

Command-line interface for controlling BrowserOS -- launch and automate the browser from the terminal or AI agents. The package installs both `browseros-cli` and `bos`.

## Installation

**Zero install (recommended):**

```bash
npx browseros-cli --help
```

**Global install:**

```bash
npm install -g browseros-cli
```

**Shell script fallback:**

```bash
curl -fsSL https://cdn.browseros.com/cli/install.sh | bash
```

## Quick Start

```bash
# Download BrowserOS from https://browseros.com

# Start BrowserOS
browseros-cli launch

# Configure MCP settings with the Server URL from BrowserOS settings
browseros-cli init http://127.0.0.1:9000/mcp

# Verify everything is working
browseros-cli health
```

## Usage

### Agent loop

```bash
page=$(browseros-cli open --json https://example.com | jq -r .page)
browseros-cli -p "$page" snapshot
browseros-cli -p "$page" read --links
browseros-cli -p "$page" find text "Search" click
browseros-cli -p "$page" press Enter
browseros-cli -p "$page" screenshot -o shot.png
browseros-cli -p "$page" close
```

`batch` can run shared-session browser steps for navigation, eval, snapshot/read/grep/find, and direct element actions like click/fill/press/type.

## Documentation

Full documentation is available at [browseros.com](https://browseros.com).

## License

MIT
