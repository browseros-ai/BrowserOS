# Contributing to BrowserOS

Hey there! Thanks for your interest in BrowserOS. Whether you're fixing bugs, adding features, improving docs, or just poking around the code, we're glad you're here.

BrowserOS is a monorepo with two main parts:

- **Agent** — The agent platform: extension UI (TypeScript/React), server, MCP tools (Rust), and a CLI (Go)
- **Browser** — The custom Chromium build (C++/Python)

Most folks start with the agent since it's way easier to set up and iterate on.

## Pick Your Path

<table>
<tr>
<td width="50%">

### 🤖 Agent Development

**What you'll work on:**
- AI agent features & tools
- UI/UX improvements
- Browser automation
- Testing & docs

**What you need:**
- [Bun](https://bun.sh) — the version pinned in `packages/browseros-agent/package.json`
- [Go](https://go.dev/dl/) — required by the `dev:*` scripts
- [Rust](https://rustup.rs/) — only if you touch `crates/`

**Skills:** TypeScript, React, Rust, Go, Chrome APIs

**[→ Agent Setup](#agent-development)**

</td>
<td width="50%">

### 🌐 Browser Development

**What you'll work on:**
- Chromium patches
- Build system
- Platform features
- Core browser stuff

**What you need:**
- ~100GB disk space
- 16GB+ RAM (recommended)
- 3+ hours for first build

**Skills:** C++, Python, Chromium internals

**[→ Browser Setup](#browser-development)**

</td>
</tr>
</table>

## Agent Development

The agent platform lives in `packages/browseros-agent`, a Bun-workspaces monorepo. It holds the extension UI, the server, the Rust MCP server and browser core, and the Go CLI.

**Use Bun, not npm/yarn/pnpm.** The package manifest pins Bun and marks the others unsupported in `engines`.

### Quick Setup

```bash
# 1. Navigate to the agent directory
cd packages/browseros-agent

# 2. Set up the development environment file
cp .env.development.example .env.development
# Fill in only the secrets the workflow you're touching needs

# 3. Install deps and generate agent code (requires Go on PATH)
bun run dev:setup

# 4. Start the server and the app UI together
bun run dev:watch
```

Working on an older checkout with per-app env files? Run `bun run env:migrate` to merge them into the root `.env.development`.

If you only want to build the extension and skip the dev harness, `bun install && bun run codegen:agent && bun run build:agent:dev` gets you a loadable build without Go. The codegen step is required — the build fails without the generated GraphQL types.

### Load in BrowserOS

1. Open `chrome://extensions/`
2. Enable **Developer mode** (top right toggle)
3. Click **Load unpacked**
4. Select `packages/browseros-agent/apps/app/dist/chrome-mv3-dev` (a production build lands in `dist/chrome-mv3`)
5. Press the Agent icon from the extensions toolbar to open the agent panel

### Before You Push

```bash
bun run check   # lint, typecheck, and Fallow
bun run test    # full test suite
```

If you changed anything under `crates/`, also run the Rust gates:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**For architecture, env vars, ports, and the full script list, see [packages/browseros-agent/README.md](packages/browseros-agent/README.md).**

## Browser Development

Building the custom Chromium browser requires significant disk space and time. Only go down this path if you're working on browser-level features like patches to Chromium itself.

### Prerequisites

- **~100GB disk space** for Chromium source
- **16GB+ RAM** (recommended)
- **Python 3.12+** and [uv](https://docs.astral.sh/uv/)
- **Platform tools:**
  - macOS: Xcode + Command Line Tools
  - Linux: build-essential and dependencies
  - Windows: Visual Studio Build Tools

### Quick Setup

**1. Get a Chromium checkout**

Follow the official Chromium guide for your platform:
- **[Chromium: Get the Code](https://www.chromium.org/developers/how-tos/get-the-code/)**

This sets up `depot_tools` and fetches the ~100GB Chromium source tree, typically 2-3 hours depending on your internet speed. The build system can also provision a checkout for you via `--provision` — see the build README below.

**2. Set up the build system**

```bash
cd packages/browseros

uv sync                 # once
cp .env.example .env    # once, then fill in what you need
uv run browseros --help
```

**3. Build BrowserOS**

```bash
# Debug build (for development) — same command on macOS/Linux/Windows
uv run browseros build --preset debug --chromium-src /path/to/chromium/src

# Release build (for production; add --no-sign/--no-upload as needed)
uv run browseros build --preset release --chromium-src /path/to/chromium/src

# See every pipeline step and preset switch
uv run browseros build --list
uv run browseros build --help

# Print the composed plan without touching a checkout
uv run browseros build --preset debug --show-plan
```

The build typically takes 1-3 hours on modern hardware (M4 Max, Ryzen 9, etc.).

**For the build system architecture and more invocations, see [packages/browseros/bos_build/README.md](packages/browseros/bos_build/README.md).**

## Making Your First Contribution

Open a PR on GitHub with:
- **Clear title** in conventional commit format
- **Description** explaining what changed and why
- **Screenshots/videos** for UI changes
- **Link to related issues** (e.g., "Fixes #123")

### Sign the CLA

On your first PR, our bot will ask you to sign the Contributor License Agreement:

1. Read the [CLA document](CLA.md)
2. Comment on your PR: `I have read the CLA Document and I hereby sign the CLA`
3. The bot will record your signature (one-time thing)

## Code Standards

The ground rules live next to the code, so they stay current as it changes:

| Scope | Read |
|-------|------|
| Agent monorepo (all of it) | [packages/browseros-agent/CLAUDE.md](packages/browseros-agent/CLAUDE.md) |
| Extension / app UI | `packages/browseros-agent/apps/app/CLAUDE.md` |
| Server | `packages/browseros-agent/apps/server/CLAUDE.md` |
| CLI (Go idioms, not the TS rules) | `packages/browseros-agent/apps/cli/CLAUDE.md` |
| Browser | Follow the Chromium style guide |

The highlights, so you know what you're walking into:

### TypeScript

- **Strict typing** — avoid `any`
- **Extensionless imports** — `./utils`, not `./utils.js`
- **Bun everywhere** — `bun test`, `bun install`, `bun run <script>`; Bun loads env files, so don't reach for dotenv
- **kebab-case** for folders and multi-word non-component files
- **Shared constants** belong in `@browseros/shared` (ports, timeouts, limits, urls, paths) rather than scattered magic values
- **Naming:** Classes `PascalCase`, functions/variables `camelCase`, constants `UPPERCASE`

### React (Agent UI)

- **Styling:** Tailwind CSS only (no SCSS or CSS modules)
- **Hooks:** Only at top level
- **Testing:** `bun test`

### General

- Keep functions short (<20 lines ideally)
- Write tests for new features
- Keep comments minimal — document hidden constraints, subtle invariants, and surprising behavior; don't restate the code
- Use descriptive variable names
- Handle errors gracefully

## Project Structure

```
BrowserOS/
├── packages/
│   ├── browseros/                 # Chromium build & release system (Python)
│   │   ├── bos_build/             # The `browseros` CLI: build steps, presets, release
│   │   ├── chromium_patches/      # Patches applied to the Chromium source
│   │   ├── series_patches/        # Ordered patch series
│   │   ├── chromium_files/        # Files copied into the Chromium tree
│   │   └── resources/             # Icons, configs, branding
│   │
│   └── browseros-agent/           # Agent platform (Bun workspaces)
│       ├── apps/
│       │   ├── app/               # Agent UI — Chrome extension (WXT + React)
│       │   ├── server/            # Bun server — agent loop and HTTP endpoints
│       │   ├── cli/               # browseros-cli (Go)
│       │   ├── claw-app/          # BrowserClaw UI
│       │   └── claw-server-rust/  # BrowserClaw server (Rust)
│       ├── crates/                # Rust — CDP client, browser core, MCP server
│       ├── packages/              # Shared TS packages (shared, cdp-protocol, ...)
│       └── contracts/             # Contract test suites
│
├── docs/                          # Public documentation (Mintlify)
└── CONTRIBUTING.md                # This file
```

## Ways to Contribute

You don't need to write code to help out! Here are other ways:

### 🐛 Report Bugs

Found a bug? [Open an issue](https://github.com/browseros-ai/BrowserOS/issues/new) with:
- Clear description
- Steps to reproduce
- Expected vs actual behavior
- Screenshots/videos
- Environment details (OS, browser version, BrowserOS version)

### 💡 Suggest Features

Have an idea? [Share it here](https://github.com/browseros-ai/BrowserOS/issues/99) or chat with us on [Discord](https://discord.gg/YKwjt5vuKr).

### 📚 Improve Documentation

- Docs live in `docs/` and use Mintlify — edit pages and update `docs/docs.json` for navigation
- Write blog posts or guides

### 🧪 Test & Provide Feedback

- Try new features
- Test on different platforms
- Report edge cases
- Share your use cases

### 🎨 Design & UX

- Suggest UI improvements
- Create mockups
- Improve accessibility
- Enhance user experience

## Getting Help

Stuck? Need clarification? We're here to help.

- **[Discord](https://discord.gg/YKwjt5vuKr)** - Real-time chat and support
- **[GitHub Issues](https://github.com/browseros-ai/BrowserOS/issues)** - Bug reports and features
- **[GitHub Discussions](https://github.com/browseros-ai/BrowserOS/discussions)** - General questions

## Recognition

We appreciate all contributors! You'll get:
- Credits in release notes
- Name in README

## License

By contributing, you agree that your contributions will be licensed under AGPL-3.0.

---

Built with ❤️ from San Francisco

Questions? Hit us up on [Discord](https://discord.gg/YKwjt5vuKr) or [Twitter](https://twitter.com/browseros_ai).
