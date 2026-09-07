<div align="center">
<img width="693" height="415" alt="github-banner" src="https://github.com/user-attachments/assets/8129f9c8-e8f4-4afe-834a-91397121d833" />

<br></br>
<a href="https://discord.gg/YKwjt5vuKr"><img src="https://img.shields.io/badge/Discord-555?logo=discord" alt="Discord" /></a>
<a href="https://dub.sh/browserOS-slack"><img src="https://img.shields.io/badge/Slack-555?logo=slack" alt="Slack" /></a>
<a href="https://x.com/browserOS_ai"><img src="https://img.shields.io/badge/@browserOS__ai-555?logo=x" alt="X / Twitter" /></a>
<a href="https://github.com/browseros-ai/BrowserOS"><img src="https://img.shields.io/github/stars/browseros-ai/BrowserOS?style=flat&logo=github&label=stars&color=4c71f2" alt="GitHub stars" /></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-555" alt="AGPL-3.0" /></a>
<br></br>

<a href="https://www.producthunt.com/products/browseros_ai?embed=true&amp;utm_source=badge-featured&amp;utm_medium=badge&amp;utm_campaign=badge-browseros-neo" target="_blank" rel="noopener noreferrer"><picture><source media="(prefers-color-scheme: dark)" srcset="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1031913&amp;theme=dark&amp;t=1786088428884" /><img alt="BrowserOS neo - The Missing Browser for Claude, Cowork &amp; Codex | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1031913&amp;theme=light&amp;t=1786088428884" /></picture></a>
<a href="https://trendshift.io/repositories/16468?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-16468" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/16468/daily?language=TypeScript" alt="browseros-ai%2FBrowserOS | Trendshift" width="250" height="55"/></a>
<br></br>

<b>Enterprise?</b> Rolling BrowserOS neo out across a team: <a href="mailto:enterprise@browseros.com?subject=BrowserOS%20neo%20enterprise%20deployment">enterprise@browseros.com</a>

<br></br>

<h3>Give your agents their own browser.</h3>

Free · Open source · Everything runs on your machine

</div>

# <img src="packages/browseros/resources/browserclaw/icons/product_logo_192.png" alt="" width="28" /> BrowserOS neo

A second browser, just for your AI agents. Import your logins from Chrome in one click, connect Claude Code, Codex, or any MCP agent, and hand off your web tasks. Agents run in parallel in their own tabs. You watch live, or replay any session like a video.

[![Download for macOS](https://img.shields.io/badge/Download-macOS-black?style=flat&logo=apple&logoColor=white)](https://cdn.browseros.com/download/BrowserOS_neo.dmg)
[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?style=flat&logo=windows&logoColor=white)](https://cdn.browseros.com/download/BrowserOS_neo_installer.exe)
&nbsp; **[Website](https://www.browseros.com/agents)** · **[Docs](https://docs.browseros.com/neo)**

BrowserOS neo is not a Chrome replacement. It is a secondary browser that sits next to Chrome, made friendly to agents.

## Get started

1. **Install BrowserOS neo and import from Chrome** in one click: logins, bookmarks, extensions.
2. **It finds your agents.** Claude Code, Codex, Cursor, VS Code, OpenClaw, Hermes; connect with one click.
3. **Give it a task from your agent.** *"Book me the cheapest flight to London."* Watch it live from your new tab, replay it later.

## What can your agents do?

Anything that needs a logged-in browser:

- Post content to your social media (LinkedIn, Twitter/X), queue posts, pull engagement numbers
- Clear your inbox, unsubscribe from junk email
- Update your CRM, file expenses, pull reports from internal tools

## Key features

<table>
<tr>
<td width="40%" valign="middle">
<h4>Live dashboard</h4>
Your new tab shows every agent working right now: which site it's on, what it's doing, how far along. <a href="https://docs.browseros.com/neo/cockpit">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--dashboard-populated.png" alt="BrowserOS neo dashboard showing agent sessions and recent activity" width="100%" />
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h4>One-click connect</h4>
Automatically connects to every harness. We built tools optimized for web use! <a href="https://docs.browseros.com/neo/mcp">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--mcp-install-board.png" alt="BrowserOS neo MCP connect board with one-click install for supported AI tools" width="100%" />
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h4>Replay every session</h4>
Every session is saved as a scrubbable video on your disk with a step-by-step action timeline. Rewind and see exactly what happened. <a href="https://docs.browseros.com/neo/audit-and-replay">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--replay-scrubber.png" alt="BrowserOS neo replay view with video scrubber and action timeline" width="100%" />
</td>
</tr>
</table>

- **Your logins.** Agents automate your real work using your logged-in accounts, not a blank sandbox. [How it works](https://docs.browseros.com/neo/how-it-works)
- **Parallel agents.** Fire off several tasks at once. Each agent works in its own tab while you keep browsing.
- **Fewer tokens.** For the same task, BrowserOS neo uses fewer tokens than the alternatives, such as Claude's Chrome extension or the Codex browser.
- **Local-only, privacy-first.** Sessions, screenshots, and history live under `~/.browserclaw/` and never leave your machine. [Privacy](https://docs.browseros.com/neo/privacy)

## Why BrowserOS neo over the alternatives?

- **Not a headless driver.** Playwright and agent-browser spin up a fresh Chrome subprocess with no logins. Great for CI, useless for real work which requires your logged-in state like "read my inbox." BrowserOS neo imports your logins with one click and persists them across sessions.
- **Not a cloud browser.** Cloud browsers (like browser-use, browserbase) run in a datacenter, so logging into your accounts is a pain, and sites like Twitter and LinkedIn block you because you are on a datacenter IP. BrowserOS neo runs on your machine, on `127.0.0.1`.
- **Not a locked-in AI browser.** Atlas, Comet, and Dia only work with their own AI. BrowserOS neo works with the agents you already use and pay for: Claude Code, Cowork, Codex, Cursor, and others.

## Also in this repo: BrowserOS

<img src="packages/browseros/resources/browseros/icons/product_logo_192.png" alt="" width="20" /> **BrowserOS** is our other browser: a Chromium fork with an AI agent built into every new tab, for when you are the one browsing. Bring your own AI keys or run everything locally with Ollama.

**[Read about BrowserOS](README.BrowserOS.md)** · [Website](https://www.browseros.com) · [Docs](https://docs.browseros.com)

## FAQ

**What's the difference between BrowserOS neo and BrowserOS?**
BrowserOS neo is a browser your AI drives. BrowserOS is a browser you drive, with an AI agent built in. Both ship from this repo and run side by side. Keep your daily browser, and let agents work in neo.

**Which AI tools work with BrowserOS neo?**
Any AI that speaks MCP. Claude Code, Codex, Cursor, VS Code, Zed, OpenCode, Hermes, OpenClaw and Antigravity connect with one click.

**Does anything leave my machine?**
Your sessions, screenshots, history, and settings live under `~/.browserclaw/` and never upload. BrowserOS neo sends anonymous product-usage events (agent connect/disconnect, version, OS) to help us improve the app; it never sends URLs, page content, prompts, tool results, or screenshots. Off with one toggle in Settings. [Full policy](https://docs.browseros.com/neo/privacy).

**Do my Chrome extensions and bookmarks work?**
Yes. Both browsers are Chromium forks, so Chrome extensions work and your bookmarks, passwords, and settings import in one click.

**What platforms are supported?**
BrowserOS neo runs on macOS and Windows. BrowserOS runs on macOS, Windows, and Linux. System requirements match Google Chrome.

## Get help

- [Discord](https://discord.gg/YKwjt5vuKr) · [Slack](https://dub.sh/browserOS-slack)
- [Report a bug](https://github.com/browseros-ai/BrowserOS/issues)
- [BrowserOS neo docs](https://docs.browseros.com/neo) · [BrowserOS docs](https://docs.browseros.com)
- Enterprise deployment: [enterprise@browseros.com](mailto:enterprise@browseros.com?subject=BrowserOS%20neo%20enterprise%20deployment)

# For developers

Both browsers ship from this monorepo. Two main subsystems: the **browser** (Chromium fork, C++ and Python) and the **agent platform** (TypeScript, Rust and Go).

## Architecture

```
BrowserOS/
├── packages/browseros/              # Chromium fork + build system (Python)
│   ├── chromium_patches/            # Patches applied to Chromium source
│   ├── build/                       # Build CLI and modules
│   └── resources/                   # Icons, entitlements, signing
│
└── packages/browseros-agent/        # Agent platform (TypeScript / Rust / Go)
    ├── apps/
    │   ├── claw-server-rust/        # BrowserOS neo backend: MCP endpoint + JSON API (Rust)
    │   ├── claw-app/                # BrowserOS neo dashboard extension (WXT + React)
    │   ├── claw-onboard/            # BrowserOS neo onboarding flow (Vite)
    │   ├── server/                  # BrowserOS MCP server + AI agent loop (Bun)
    │   ├── app/                     # BrowserOS extension UI (WXT + React)
    │   ├── app-onboard/             # BrowserOS onboarding flow (Vite)
    │   └── cli/                     # CLI tool (Go)
    │
    ├── packages/                    # Shared TypeScript packages
    │   ├── acpx-ai-provider/        # AI SDK provider over the acpx ACP runtime
    │   ├── agent-mcp-manager/       # Add, link and unlink MCP servers across coding agents
    │   ├── browser-core/            # Core browser control primitives
    │   ├── browser-mcp/             # Browser MCP tool surface
    │   ├── build-server-tools/      # Shared build tooling for server binaries and assets
    │   ├── cdp-protocol/            # CDP type bindings
    │   ├── claw-api/                # Generated BrowserOS neo wire types
    │   ├── claw-api-client/         # Contract-typed BrowserOS neo HTTP client
    │   ├── onboarding-video/        # Remotion compositions for the first-run demo
    │   └── shared/                  # Shared constants
    │
    └── crates/                      # Shared Rust crates
        ├── browseros-cdp/           # CDP bindings
        ├── browseros-core/          # Core primitives
        ├── browseros-mcp/           # MCP server implementation
        ├── claw-api/                # Wire types, shared with the TypeScript package
        └── harness-integrations/    # Managed integrations for AI coding harnesses
```

| Package | What it does |
|---------|-------------|
| [`packages/browseros`](packages/browseros/) | Chromium fork: patches, build system, signing |
| [`apps/claw-server-rust`](packages/browseros-agent/apps/claw-server-rust/) | BrowserOS neo backend: MCP endpoint agents connect to, plus the API behind the dashboard |
| [`apps/claw-app`](packages/browseros-agent/apps/claw-app/) | BrowserOS neo new-tab dashboard: watch, replay, and manage agent sessions |
| [`apps/claw-onboard`](packages/browseros-agent/apps/claw-onboard/) | BrowserOS neo first-run onboarding |
| [`apps/server`](packages/browseros-agent/apps/server/) | Bun server exposing the browser MCP tools and running the BrowserOS AI agent loop |
| [`apps/app`](packages/browseros-agent/apps/app/) | BrowserOS extension: new tab, side panel chat, onboarding, settings |
| [`apps/app-onboard`](packages/browseros-agent/apps/app-onboard/) | BrowserOS first-run onboarding |
| [`apps/cli`](packages/browseros-agent/apps/cli/) | Go CLI: control BrowserOS from the terminal or AI coding agents |

## Contributing

We'd love your help making BrowserOS neo and BrowserOS better. See the [Contributing Guide](CONTRIBUTING.md) for details.

- **Agent platform** (TypeScript/Rust/Go): see the [agent monorepo README](packages/browseros-agent/README.md) for setup.
- **Browser** (C++/Python): requires ~100GB disk space. See [`packages/browseros`](packages/browseros/) for build instructions.

## Credits

- [ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium): we use some of its patches for enhanced privacy. Thanks to everyone behind this project.
- [The Chromium Project](https://www.chromium.org/): at the core of both browsers, making it possible for them to exist in the first place.

## Citation

If you use BrowserOS or BrowserOS neo in your research or project, please cite:

```bibtex
@software{browseros2025,
  author = {Nithin Sonti and Nikhil Sonti and {BrowserOS-team}},
  title = {BrowserOS: The open-source Agentic browser},
  url = {https://github.com/browseros-ai/BrowserOS},
  year = {2025},
  publisher = {GitHub},
  license = {AGPL-3.0},
}
```

## License

BrowserOS neo and BrowserOS are open source under the [AGPL-3.0 license](LICENSE).

Copyright &copy; 2026 Felafax, Inc.

## Stargazers

Thank you to all our supporters.

<table>
<tr>
<td align="center">Nikhil</td>
<td align="center">Nithin</td>
<td align="center">Dani</td>
</tr>
<tr>
<td align="center"><a href="https://x.com/intent/user?screen_name=nv_sonti"><img src="https://img.shields.io/twitter/follow/nv_sonti?style=social" alt="Follow Nikhil on X" /></a></td>
<td align="center"><a href="https://x.com/intent/user?screen_name=ThatNithin"><img src="https://img.shields.io/twitter/follow/ThatNithin?style=social" alt="Follow Nithin on X" /></a></td>
<td align="center"><a href="https://x.com/intent/user?screen_name=dani_akash_"><img src="https://img.shields.io/twitter/follow/dani_akash_?style=social" alt="Follow Dani on X" /></a></td>
</tr>
</table>

<p align="center">
Built with ❤️ from San Francisco
</p>
