<div align="center">
<img width="693" height="379" alt="github-banner" src="https://github.com/user-attachments/assets/1e37941c-4dbc-4662-9c8c-3bbe9971301d" />

<br></br>
<a href="https://discord.gg/YKwjt5vuKr"><img src="https://img.shields.io/badge/Discord-555?logo=discord" alt="Discord" /></a>
<a href="https://dub.sh/browserOS-slack"><img src="https://img.shields.io/badge/Slack-555?logo=slack" alt="Slack" /></a>
<a href="https://x.com/browserOS_ai"><img src="https://img.shields.io/badge/@browserOS__ai-555?logo=x" alt="X / Twitter" /></a>
<a href="https://github.com/browseros-ai/BrowserOS"><img src="https://img.shields.io/github/stars/browseros-ai/BrowserOS?style=flat&logo=github&label=stars&color=4c71f2" alt="GitHub stars" /></a>
<br></br>

</div>

<table>
<tr>
<td width="50%" align="center" valign="top">

### BrowserClaw

**The browser for AI agents**

Claude Code, Codex, Cursor, or any MCP client drives it using the accounts you're already signed into — while you watch live and replay every step.

[![Download for macOS](https://img.shields.io/badge/Download-macOS-black?style=flat&logo=apple&logoColor=white)](https://cdn.browseros.com/download/BrowserClaw.dmg)
[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?style=flat&logo=windows&logoColor=white)](https://cdn.browseros.com/download/BrowserClaw_installer.exe)

**[Website](https://www.browseros.com/agents)** · **[Docs](https://docs.browseros.com/browserclaw)**

</td>
<td width="50%" align="center" valign="top">

### BrowserOS

**The AI browser for humans**

An open-source Chromium fork with a built-in AI agent — the privacy-first alternative to ChatGPT Atlas, Perplexity Comet, and Dia.

[![Download for macOS](https://img.shields.io/badge/Download-macOS-black?style=flat&logo=apple&logoColor=white)](https://files.browseros.com/download/BrowserOS.dmg)
[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?style=flat&logo=windows&logoColor=white)](https://files.browseros.com/download/BrowserOS_installer.exe)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=flat&logo=linux&logoColor=black)](https://files.browseros.com/download/BrowserOS.AppImage)
[![Download for Debian](https://img.shields.io/badge/Download-Debian-D70A53?style=flat&logo=debian&logoColor=white)](https://cdn.browseros.com/download/BrowserOS.deb)

**[Website](https://www.browseros.com)** · **[Docs](https://docs.browseros.com)**

</td>
</tr>
</table>

<div align="center">

**Two browsers, one codebase — we're building both.** Free and open source under AGPL-3.0, and everything runs on your machine.

</div>

## BrowserClaw

Your AI is smart, but it can't press the buttons. Ask it to book a flight, download an invoice, or reply to an email — it stops at the login screen. BrowserClaw fixes that: it's a real browser you install, sign into, and use like any other, and your AI drives it with the logins you already have — not a headless spec, not a cloud rental.

### Get started

1. **Install BrowserClaw and sign in** to the sites you use every day — it works like any browser, and every account you sign into becomes something your AI can use
2. **Connect your AI in one click** — Claude Code, Codex, Cursor, VS Code, Zed, Antigravity; anything else that speaks MCP connects with a URL
3. **Give it a real task** — *"Book me the cheapest morning flight from SFO to NYC next Friday"* — then watch it live from your new tab and replay the whole run like a video

### Features

<table>
<tr>
<td width="40%" valign="middle">
<h3>Live dashboard</h3>
Your new tab shows every agent working right now — which site it's on, what it's doing, how far along. <a href="https://docs.browseros.com/browserclaw/cockpit">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--dashboard-populated.png" alt="BrowserClaw dashboard showing agent sessions and recent activity" width="100%" />
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h3>One-click MCP connect</h3>
One endpoint, every harness. Claude Code, Codex, Cursor, Antigravity, VS Code, and Zed set up with a single click. <a href="https://docs.browseros.com/browserclaw/mcp/index">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--mcp-install-board.png" alt="BrowserClaw MCP connect board with one-click install for Claude Code, Codex, Cursor, Antigravity, VS Code, and Zed" width="100%" />
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h3>Replay &amp; audit</h3>
Every session is saved as a scrubbable video on your disk, with a step-by-step action timeline — rewind and see exactly what happened. <a href="https://docs.browseros.com/browserclaw/audit-and-replay">Docs</a>
</td>
<td width="60%">
<img src="docs/images/browserclaw--replay-scrubber.png" alt="BrowserClaw replay view with video scrubber and action timeline" width="100%" />
</td>
</tr>
</table>

- **Your logins** — agents automate your real work using the sessions you already have, not a blank sandbox ([how it works](https://docs.browseros.com/browserclaw/how-it-works))
- **Isolated agent tabs** — every agent works in its own tabs and never touches yours; run several in parallel ([tabs & isolation](https://docs.browseros.com/browserclaw/tabs-and-isolation))
- **100% local** — sessions, screenshots, history, and settings never leave your machine ([privacy](https://docs.browseros.com/browserclaw/privacy))

## BrowserOS

An open-source Chromium fork that runs AI agents natively — the privacy-first alternative to ChatGPT Atlas, Perplexity Comet, and Dia. Use your own API keys or run local models with Ollama; your data never leaves your machine.

### Quick Start

1. **Download and install** BrowserOS — [macOS](https://files.browseros.com/download/BrowserOS.dmg) · [Windows](https://files.browseros.com/download/BrowserOS_installer.exe) · [Linux (AppImage)](https://files.browseros.com/download/BrowserOS.AppImage) · [Linux (Debian)](https://cdn.browseros.com/download/BrowserOS.deb)
2. **Import your Chrome data** (optional) — bookmarks, passwords, extensions all carry over
3. **Connect your AI provider** — Claude, OpenAI, Gemini, ChatGPT Pro via OAuth, or local models via Ollama/LM Studio

### Features

| Feature | Description | Docs |
|---------|-------------|------|
| **AI Agent** | 53+ browser automation tools — navigate, click, type, extract data, all with natural language | [Guide](https://docs.browseros.com/getting-started) |
| **MCP Server** | Control the browser from Claude Code, Gemini CLI, or any MCP client | [Setup](https://docs.browseros.com/features/use-with-claude-code) |
| **Cowork** | Combine browser automation with local file operations — research the web, save reports to your folder | [Docs](https://docs.browseros.com/features/cowork) |
| **Scheduled Tasks** | Run agents on autopilot — daily, hourly, or every few minutes | [Docs](https://docs.browseros.com/features/scheduled-tasks) |
| **40+ App Integrations** | Gmail, Slack, GitHub, Linear, Notion, Figma, Salesforce, and more via MCP | [Docs](https://docs.browseros.com/features/connect-mcps) |
| **Vertical Tabs** | Side-panel tab management — stay organized even with 100+ tabs open | [Docs](https://docs.browseros.com/features/vertical-tabs) |
| **Ad Blocking** | uBlock Origin + Manifest V2 support — [10x more protection](https://docs.browseros.com/features/ad-blocking) than Chrome | [Docs](https://docs.browseros.com/features/ad-blocking) |
| **Cloud Sync** | Sync browser config and agent history across devices | [Docs](https://docs.browseros.com/features/sync-to-cloud) |
| **Smart Nudges** | Contextual suggestions to connect apps and use features at the right moment | [Docs](https://docs.browseros.com/features/smart-nudges) |

### Demos

#### BrowserOS agent in action
[![BrowserOS agent in action](docs/videos/browserOS-agent-in-action.gif)](https://www.youtube.com/watch?v=SoSFev5R5dI)
<br/><br/>

#### Install [BrowserOS as MCP](https://docs.browseros.com/features/use-with-claude-code) and control it from `claude-code`

https://github.com/user-attachments/assets/c725d6df-1a0d-40eb-a125-ea009bf664dc

<br/><br/>

#### Use BrowserOS to chat

https://github.com/user-attachments/assets/726803c5-8e36-420e-8694-c63a2607beca

<br/><br/>

#### Use BrowserOS to scrape data

https://github.com/user-attachments/assets/9f038216-bc24-4555-abf1-af2adcb7ebc0

<br/><br/>

### LLM Providers

BrowserOS works with any LLM. Bring your own keys, use OAuth, or run models locally.

| Provider | Type | Auth |
|----------|------|------|
| Kimi K2.5 | Cloud (default) | Built-in |
| ChatGPT Pro/Plus | Cloud | [OAuth](https://docs.browseros.com/features/chatgpt) |
| GitHub Copilot | Cloud | [OAuth](https://docs.browseros.com/features/github-copilot) |
| Qwen Code | Cloud | [OAuth](https://docs.browseros.com/features/qwen-code) |
| Claude (Anthropic) | Cloud | API key |
| GPT-4o / o3 (OpenAI) | Cloud | API key |
| Gemini (Google) | Cloud | API key |
| Azure OpenAI | Cloud | API key |
| AWS Bedrock | Cloud | IAM credentials |
| OpenRouter | Cloud | API key |
| Ollama | Local | [Setup](https://docs.browseros.com/features/ollama) |
| LM Studio | Local | [Setup](https://docs.browseros.com/features/lm-studio) |

### How We Compare

| | BrowserOS | Chrome | Brave | Dia | Comet | Atlas |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| Open Source | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| AI Agent | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ |
| MCP Server | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cowork (files + browser) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Scheduled Tasks | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Bring Your Own Keys | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Local Models (Ollama) | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Local-first Privacy | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Ad Blocking (MV2) | ✅ | ❌ | ✅ | ❌ | ✅ | ❌ |

**Detailed comparisons:**
- [BrowserOS vs Chrome DevTools MCP](https://docs.browseros.com/comparisons/chrome-devtools-mcp) — developer-focused comparison for browser automation
- [BrowserOS vs Claude Cowork](https://docs.browseros.com/comparisons/claude-cowork) — getting real work done with AI
- [BrowserOS vs OpenClaw](https://docs.browseros.com/comparisons/openclaw) — everyday AI assistance

## FAQ

### What's the difference between BrowserClaw and BrowserOS?

BrowserClaw is a browser your AI drives; BrowserOS is a browser you drive, with an AI agent built in. They're standalone apps that live side by side — many people use BrowserOS as their daily browser and BrowserClaw as their agents' browser.

### Which AI tools work with BrowserClaw?

Any AI agent that supports MCP, which is essentially every serious AI coding tool today. Claude Code, Codex, Cursor, VS Code, Zed, and Antigravity connect with one click; anything else connects with a URL.

### Does my AI share my logins?

Yes — that's the point. Agents drive BrowserClaw using the sessions you already have, so they automate your real work instead of poking a blank sandbox. Per-agent profile isolation is on the roadmap if you want to keep them apart.

### Will my AI interrupt my browsing?

No. Every agent opens its own tabs to work in and never touches the ones you have open. It can't close the doc you're writing or steer the tab you're using — you keep working while it works.

### Can I run two AIs at the same time?

Yes. Every agent gets its own set of tabs, tracked separately. Codex and Claude Code can each work on their own tasks at the same time without stepping on each other.

### Can I replay what my AI did?

Yes. Every session is saved as a scrubbable video with a step-by-step action timeline. Rewind, spot the moment things went sideways, and give the agent a better instruction next time.

### Is my data safe? Does anything go to the cloud?

Everything runs on your machine. Session history, screenshots, replays, and settings are files on your disk, and your logins stay in your browser profile — same as any browser. There's no dashboard we own. Open source, top to bottom.

### What LLM providers does BrowserOS support?

11+ providers: Kimi, Claude, OpenAI, Gemini, ChatGPT Pro/Plus and GitHub Copilot via OAuth, OpenRouter, Azure, Bedrock — or fully local models through Ollama and LM Studio. Bring your own keys and switch anytime.

### Do my Chrome extensions and bookmarks work?

Yes. Both browsers are Chromium forks, so Chrome extensions work and your bookmarks, passwords, and settings import in one click.

### What platforms are supported?

BrowserClaw runs on macOS and Windows. BrowserOS runs on macOS, Windows, and Linux. System requirements match Google Chrome.

### Is it free?

Yes. Both products are free and open source under AGPL-3.0. You bring your own AI provider keys.

## Architecture

Both products ship from this monorepo, which has two main subsystems: the **browser** (Chromium fork) and the **agent platform** (TypeScript/Go).

```
BrowserOS/
├── packages/browseros/              # Chromium fork + build system (Python)
│   ├── chromium_patches/            # Patches applied to Chromium source
│   ├── build/                       # Build CLI and modules
│   └── resources/                   # Icons, entitlements, signing
│
├── packages/browseros-agent/        # Agent platform (TypeScript/Go)
│   ├── apps/
│   │   ├── claw-server/             # BrowserClaw backend — MCP endpoint + JSON API (Hono)
│   │   ├── claw-app/                # BrowserClaw dashboard extension (WXT + React)
│   │   ├── claw-onboard/            # BrowserClaw onboarding flow
│   │   ├── server/                  # BrowserOS MCP server + AI agent loop (Bun)
│   │   ├── app/                     # BrowserOS extension UI (WXT + React)
│   │   └── cli/                     # CLI tool (Go)
│   │
│   └── packages/
│       ├── agent-sdk/               # Node.js SDK (npm: @browseros-ai/agent-sdk)
│       ├── cdp-protocol/            # CDP type bindings
│       └── shared/                  # Shared constants
```

| Package | What it does |
|---------|-------------|
| [`packages/browseros`](packages/browseros/) | Chromium fork — patches, build system, signing |
| [`apps/claw-server`](packages/browseros-agent/apps/claw-server/) | BrowserClaw backend — the MCP endpoint agents connect to, plus the API behind the dashboard |
| [`apps/claw-app`](packages/browseros-agent/apps/claw-app/) | BrowserClaw new-tab dashboard — watch, replay, and manage agent sessions |
| [`apps/server`](packages/browseros-agent/apps/server/) | Bun server exposing 53+ MCP tools and running the BrowserOS AI agent loop |
| [`apps/app`](packages/browseros-agent/apps/app/) | BrowserOS extension — new tab, side panel chat, onboarding, settings |
| [`apps/cli`](packages/browseros-agent/apps/cli/) | Go CLI — control BrowserOS from the terminal or AI coding agents |
| [`agent-sdk`](packages/browseros-agent/packages/agent-sdk/) | Node.js SDK for browser automation with natural language |
| [`cdp-protocol`](packages/browseros-agent/packages/cdp-protocol/) | Type-safe Chrome DevTools Protocol bindings |

## Contributing

We'd love your help making BrowserOS and BrowserClaw better! See our [Contributing Guide](CONTRIBUTING.md) for details.

- [Report bugs](https://github.com/browseros-ai/BrowserOS/issues)
- [Suggest features](https://github.com/browseros-ai/BrowserOS/issues/99)
- [Join Discord](https://discord.gg/YKwjt5vuKr) · [Join Slack](https://dub.sh/browserOS-slack)
- [Follow on Twitter](https://x.com/browserOS_ai)

**Agent development** (TypeScript/Go) — see the [agent monorepo README](packages/browseros-agent/README.md) for setup instructions.

**Browser development** (C++/Python) — requires ~100GB disk space. See [`packages/browseros`](packages/browseros/) for build instructions.

## Credits

- [ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium) — we use some of its patches for enhanced privacy. Thanks to everyone behind this project!
- [The Chromium Project](https://www.chromium.org/) — at the core of both browsers, making it possible for them to exist in the first place.

## Citation

If you use BrowserOS in your research or project, please cite:

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

BrowserOS and BrowserClaw are open source under the [AGPL-3.0 license](LICENSE).

Copyright &copy; 2026 Felafax, Inc.

## Stargazers

Thank you to all our supporters!

<!-- [![Star History Chart](https://api.star-history.com/svg?repos=browseros-ai/BrowserOS&type=Date)](https://www.star-history.com/#browseros-ai/BrowserOS&Date) -->

Team — Nikhil ([@nv_sonti](https://x.com/intent/user?screen_name=nv_sonti)), Nithin ([@ThatNithin](https://x.com/intent/user?screen_name=ThatNithin)) and Dani ([@dani_akash_](https://x.com/intent/user?screen_name=dani_akash_)):

[![Twitter Follow](https://img.shields.io/twitter/follow/nv_sonti?style=social)](https://x.com/intent/user?screen_name=nv_sonti)
&emsp;&emsp;&emsp;
[![Twitter Follow](https://img.shields.io/twitter/follow/ThatNithin?style=social)](https://x.com/intent/user?screen_name=ThatNithin)
&emsp;&emsp;&emsp;
[![Twitter Follow](https://img.shields.io/twitter/follow/dani_akash_?style=social)](https://x.com/intent/user?screen_name=dani_akash_)

<p align="center">
Built with ❤️ from San Francisco
</p>
