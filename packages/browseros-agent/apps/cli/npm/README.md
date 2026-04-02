# browseros-cli

Command-line interface for controlling BrowserOS -- launch and automate the browser from the terminal.

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
# Download BrowserOS
browseros-cli install

# Start BrowserOS
browseros-cli launch

# Auto-configure MCP settings for your AI tools
browseros-cli init --auto

# Verify everything is working
browseros-cli health
```

## Usage

### Navigation

```bash
browseros-cli navigate "https://example.com"
```

### Observation

```bash
browseros-cli snapshot           # Get the accessibility tree of the current page
browseros-cli console-logs       # View browser console output
```

### Screenshots

```bash
browseros-cli screenshot         # Capture the current page
```

### Input

```bash
browseros-cli click 42           # Click an element by its node ID
browseros-cli fill 85 "query"    # Type text into an input field
```

### Agent Mode

```bash
browseros-cli agent "Search for flights to Tokyo"
```

## Documentation

Full documentation is available at [browseros.com](https://browseros.com).

## License

MIT
