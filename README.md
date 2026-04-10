<div align="center">
<img width="693" height="379" alt="github-banner" src="https://github.com/user-attachments/assets/1e37941c-4dbc-4662-9c8c-3bbe9971301d" />

<br></br>
[![Discord](https://img.shields.io/badge/Discord-Join%20us-blue)](https://discord.gg/YKwjt5vuKr)
[![Slack](https://img.shields.io/badge/Slack-Join%20us-4A154B?logo=slack&logoColor=white)](https://dub.sh/browserOS-slack)
[![Twitter](https://img.shields.io/twitter/follow/browserOS_ai?style=social)](https://twitter.com/browseros_ai)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/Docs-docs.browseros.com-blue)](https://docs.browseros.com)
<br></br>
<a href="https://files.browseros.com/download/BrowserOS.dmg">
  <img src="https://img.shields.io/badge/Download-macOS-black?style=flat&logo=apple&logoColor=white" alt="Download for macOS (beta)" />
</a>
<a href="https://files.browseros.com/download/BrowserOS_installer.exe">
  <img src="https://img.shields.io/badge/Download-Windows-0078D4?style=flat&logo=windows&logoColor=white" alt="Download for Windows (beta)" />
</a>
<a href="https://files.browseros.com/download/BrowserOS.AppImage">
  <img src="https://img.shields.io/badge/Download-Linux-FCC624?style=flat&logo=linux&logoColor=black" alt="Download for Linux (beta)" />
</a>
<a href="https://cdn.browseros.com/download/BrowserOS.deb">
  <img src="https://img.shields.io/badge/Download-Debian-D70A53?style=flat&logo=debian&logoColor=white" alt="Download Debian package" />
</a>
<br />
</div>

BrowserOS is an open-source Chromium fork that runs AI agents natively. **The privacy-first alternative to ChatGPT Atlas, Perplexity Comet, and Dia.**

Use your own API keys or run local models with Ollama. Your data never leaves your machine.

> **[Documentation](https://docs.browseros.com)** · **[Discord](https://discord.gg/YKwjt5vuKr)** · **[Slack](https://dub.sh/browserOS-slack)** · **[Twitter](https://x.com/browserOS_ai)** · **[Feature Requests](https://github.com/browseros-ai/BrowserOS/issues/99)**

## Quick Start

1. **Download and install** BrowserOS — [macOS](https://files.browseros.com/download/BrowserOS.dmg) · [Windows](https://files.browseros.com/download/BrowserOS_installer.exe) · [Linux (AppImage)](https://files.browseros.com/download/BrowserOS.AppImage) · [Linux (Debian)](https://cdn.browseros.com/download/BrowserOS.deb)
2. **Import your Chrome data** (optional) — bookmarks, passwords, extensions all carry over
3. **Connect your AI provider** — Claude, OpenAI, Gemini, ChatGPT Pro via OAuth, or local models via Ollama/LM Studio

## Features

| Feature | Description | Docs |
|---------|-------------|------|
| **AI Agent** | 53+ browser automation tools — navigate, click, type, extract data, all with natural language | [Guide](https://docs.browseros.com/getting-started) |
| **MCP Server** | Control the browser from Claude Code, Gemini CLI, or any MCP client | [Setup](https://docs.browseros.com/features/use-with-claude-code) |
| **Workflows** | Build repeatable browser automations with a visual graph builder | [Docs](https://docs.browseros.com/features/workflows) |
| **Cowork** | Combine browser automation with local file operations — research the web, save reports to your folder | [Docs](https://docs.browseros.com/features/cowork) |
| **Scheduled Tasks** | Run agents on autopilot — daily, hourly, or every few minutes | [Docs](https://docs.browseros.com/features/scheduled-tasks) |
| **Memory** | Persistent memory across conversations — your assistant remembers context over time | [Docs](https://docs.browseros.com/features/memory) |
| **SOUL.md** | Define your AI's personality and instructions in a single markdown file | [Docs](https://docs.browseros.com/features/soul-md) |
| **LLM Hub** | Compare Claude, ChatGPT, and Gemini responses side-by-side on any page | [Docs](https://docs.browseros.com/features/llm-chat-hub) |
| **40+ App Integrations** | Gmail, Slack, GitHub, Linear, Notion, Figma, Salesforce, and more via MCP | [Docs](https://docs.browseros.com/features/connect-apps) |
| **Vertical Tabs** | Side-panel tab management — stay organized even with 100+ tabs open | [Docs](https://docs.browseros.com/features/vertical-tabs) |
| **Ad Blocking** | uBlock Origin + Manifest V2 support — [10x more protection](https://docs.browseros.com/features/ad-blocking) than Chrome | [Docs](https://docs.browseros.com/features/ad-blocking) |
| **Cloud Sync** | Sync browser config and agent history across devices | [Docs](https://docs.browseros.com/features/sync) |
| **Skills** | Custom instruction sets that shape how your AI assistant behaves | [Docs](https://docs.browseros.com/features/skills) |
| **Smart Nudges** | Contextual suggestions to connect apps and use features at the right moment | [Docs](https://docs.browseros.com/features/smart-nudges) |

## Demos

### BrowserOS agent in action
[![BrowserOS agent in action](docs/videos/browserOS-agent-in-action.gif)](https://www.youtube.com/watch?v=SoSFev5R5dI)
<br/><br/>

### Install [BrowserOS as MCP](https://docs.browseros.com/features/use-with-claude-code) and control it from `claude-code`

https://github.com/user-attachments/assets/c725d6df-1a0d-40eb-a125-ea009bf664dc

<br/><br/>

### Use BrowserOS to chat

https://github.com/user-attachments/assets/726803c5-8e36-420e-8694-c63a2607beca

<br/><br/>

### Use BrowserOS to scrape data

https://github.com/user-attachments/assets/9f038216-bc24-4555-abf1-af2adcb7ebc0

<br/><br/>

## Install `browseros-cli`

Use `browseros-cli` to launch and control BrowserOS from the terminal or from AI coding agents like Claude Code.

**macOS / Linux:**

```bash
curl -fsSL https://cdn.browseros.com/cli/install.sh | bash
```

**Windows:**

```powershell
irm https://cdn.browseros.com/cli/install.ps1 | iex
```

After install, run `browseros-cli init` to connect the CLI to your running BrowserOS instance.

## LLM Providers

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

## How We Compare

| | BrowserOS | Chrome | Brave | Dia | Comet | Atlas |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| Open Source | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| AI Agent | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ |
| MCP Server | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Visual Workflows | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
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

## Architecture

BrowserOS is a monorepo with two main subsystems: the **browser** (Chromium fork) and the **agent platform** (TypeScript/Go).

```
BrowserOS/
├── packages/browseros/              # Chromium fork + build system (Python)
│   ├── chromium_patches/            # Patches applied to Chromium source
│   ├── build/                       # Build CLI and modules
│   └── resources/                   # Icons, entitlements, signing
│
├── packages/browseros-agent/        # Agent platform (TypeScript/Go)
│   ├── apps/
│   │   ├── server/                  # MCP server + AI agent loop (Bun)
│   │   ├── agent/                   # Browser extension UI (WXT + React)
│   │   ├── cli/                     # CLI tool (Go)
│   │   ├── eval/                    # Benchmark framework
│   │   └── controller-ext/          # Chrome API bridge extension
│   │
│   └── packages/
│       ├── agent-sdk/               # Node.js SDK (npm: @browseros-ai/agent-sdk)
│       ├── cdp-protocol/            # CDP type bindings
│       └── shared/                  # Shared constants
```

| Package | What it does |
|---------|-------------|
| [`packages/browseros`](packages/browseros/) | Chromium fork — patches, build system, signing |
| [`apps/server`](packages/browseros-agent/apps/server/) | Bun server exposing 53+ MCP tools and running the AI agent loop |
| [`apps/agent`](packages/browseros-agent/apps/agent/) | Browser extension — new tab, side panel chat, onboarding, settings |
| [`apps/cli`](packages/browseros-agent/apps/cli/) | Go CLI — control BrowserOS from the terminal or AI coding agents |
| [`apps/eval`](packages/browseros-agent/apps/eval/) | Benchmark framework — WebVoyager, Mind2Web evaluation |
| [`agent-sdk`](packages/browseros-agent/packages/agent-sdk/) | Node.js SDK for browser automation with natural language |
| [`cdp-protocol`](packages/browseros-agent/packages/cdp-protocol/) | Type-safe Chrome DevTools Protocol bindings |

## Contributing

We'd love your help making BrowserOS better! See our [Contributing Guide](CONTRIBUTING.md) for details.

- [Report bugs](https://github.com/browseros-ai/BrowserOS/issues)
- [Suggest features](https://github.com/browseros-ai/BrowserOS/issues/99)
- [Join Discord](https://discord.gg/YKwjt5vuKr) · [Join Slack](https://dub.sh/browserOS-slack)
- [Follow on Twitter](https://x.com/browserOS_ai)

**Agent development** (TypeScript/Go) — see the [agent monorepo README](packages/browseros-agent/README.md) for setup instructions.

**Browser development** (C++/Python) — requires ~100GB disk space. See [`packages/browseros`](packages/browseros/) for build instructions.

## Credits

- [ungoogled-chromium](https://github.com/ungoogled-software/ungoogled-chromium) — BrowserOS uses some patches for enhanced privacy. Thanks to everyone behind this project!
- [The Chromium Project](https://www.chromium.org/) — at the core of BrowserOS, making it possible to exist in the first place.

## License

BrowserOS is open source under the [AGPL-3.0 license](LICENSE).

Copyright &copy; 2026 Felafax, Inc.

## Stargazers

Thank you to all our supporters!

[![Star History Chart](https://api.star-history.com/svg?repos=browseros-ai/BrowserOS&type=Date)](https://www.star-history.com/#browseros-ai/BrowserOS&Date)

<p align="center">
Built with ❤️ from San Francisco
</p>
