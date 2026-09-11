# <img src="packages/browseros/resources/browseros/icons/product_logo_192.png" alt="" width="28" /> BrowserOS: the AI browser for humans

> Looking for the browser your agents drive? That is [BrowserOS neo](README.md), and it ships from this same repo.

BrowserOS is a free, open-source Chromium fork with an AI agent built into every new tab. Ask it to summarise a page, click through a flow, extract data, or run a scheduled task, and it uses 20+ built-in tools plus 40+ app integrations to get the work done. Bring your own AI keys or run everything locally with Ollama.

Every AI browser today asks you to sign into their cloud and hand over your data. BrowserOS is the one that doesn't. Same daily browser you already use, with a helpful agent one keystroke away.

[![Download for macOS](https://img.shields.io/badge/Download-macOS-black?style=flat&logo=apple&logoColor=white)](https://files.browseros.com/download/BrowserOS.dmg)
[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?style=flat&logo=windows&logoColor=white)](https://files.browseros.com/download/BrowserOS_installer.exe)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?style=flat&logo=linux&logoColor=black)](https://files.browseros.com/download/BrowserOS.AppImage)
[![Download for Debian](https://img.shields.io/badge/Download-Debian-D70A53?style=flat&logo=debian&logoColor=white)](https://cdn.browseros.com/download/BrowserOS.deb)
&nbsp; **[Website](https://www.browseros.com)** · **[Docs](https://docs.browseros.com)**

## Get started

1. **Install BrowserOS.** On macOS, `brew install --cask browseros`. Otherwise download it: [macOS](https://files.browseros.com/download/BrowserOS.dmg) · [Windows](https://files.browseros.com/download/BrowserOS_installer.exe) · [Linux (AppImage)](https://files.browseros.com/download/BrowserOS.AppImage) · [Linux (Debian)](https://cdn.browseros.com/download/BrowserOS.deb).
2. **Import from Chrome** in one click. Bookmarks, passwords, extensions all carry over.
3. **Connect your AI provider.** Claude, OpenAI, Gemini, ChatGPT Pro via OAuth, or local models via Ollama or LM Studio.

## Key features

<table>
<tr>
<td width="40%" valign="middle">
<h4>BrowserOS agent in action</h4>
Ask it in plain English. 20+ built-in tools plus 40+ app integrations (Gmail, Slack, GitHub, Linear, Notion, and more). <a href="https://docs.browseros.com/getting-started">Docs</a>
</td>
<td width="60%">
<a href="https://www.youtube.com/watch?v=SoSFev5R5dI"><img src="docs/videos/browserOS-agent-in-action.gif" alt="BrowserOS agent completing a browser task with natural language" width="100%" /></a>
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h4>Install as MCP and control from claude-code</h4>
Turn BrowserOS into an MCP server and drive it from Claude Code, Cursor, or any MCP client. <a href="https://docs.browseros.com/features/use-with-claude-code">Docs</a>
</td>
<td width="60%">
<video src="https://github.com/user-attachments/assets/c725d6df-1a0d-40eb-a125-ea009bf664dc" controls width="100%"></video>
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h4>Use BrowserOS to chat</h4>
Chat about the current page from the side panel. Summarise, ask questions, transform what you're reading. <a href="https://docs.browseros.com/getting-started">Docs</a>
</td>
<td width="60%">
<video src="https://github.com/user-attachments/assets/726803c5-8e36-420e-8694-c63a2607beca" controls width="100%"></video>
</td>
</tr>
<tr>
<td width="40%" valign="middle">
<h4>Use BrowserOS to scrape data</h4>
Point the agent at a page, tell it what to pull, and get structured data back. <a href="https://docs.browseros.com/getting-started">Docs</a>
</td>
<td width="60%">
<video src="https://github.com/user-attachments/assets/9f038216-bc24-4555-abf1-af2adcb7ebc0" controls width="100%"></video>
</td>
</tr>
</table>

- **Cowork with files.** Combine browser automation with local file operations in one session. [Docs](https://docs.browseros.com/features/cowork)
- **Scheduled tasks.** Run agents on autopilot: daily, hourly, or every few minutes. [Docs](https://docs.browseros.com/features/scheduled-tasks)
- **Bring your own AI.** 11+ providers, or fully local with Ollama and LM Studio. [Provider list](https://docs.browseros.com/features/bring-your-own-llm)
- **Real ad blocking.** uBlock Origin with full Manifest V2 support. [Docs](https://docs.browseros.com/features/ad-blocking)

## Why BrowserOS over the alternatives?

- **Not Chrome with an AI extension.** Extensions can't touch the browser chrome, can't run scheduled background tasks, can't ship the 20+ built-in tools that the agent uses natively. BrowserOS builds the agent into Chromium itself.
- **Not Comet, Atlas, or Dia.** Those AI browsers route your prompts through their cloud with their model. BrowserOS runs on your machine with your AI keys. Your data stays yours.

## How BrowserOS compares

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

## Platforms

BrowserOS runs on macOS, Windows, and Linux. System requirements match Google Chrome.

## Get help

- [Discord](https://discord.gg/YKwjt5vuKr) · [Slack](https://dub.sh/browserOS-slack)
- [Report a bug](https://github.com/browseros-ai/BrowserOS/issues)
- [BrowserOS docs](https://docs.browseros.com)
- Enterprise deployment: [founders@browseros.com](mailto:founders@browseros.com?subject=BrowserOS%20enterprise%20deployment)

## Contributing and license

BrowserOS is built in this repo alongside BrowserOS neo. See the [Contributing Guide](CONTRIBUTING.md) and the [architecture overview](README.md#architecture).

Open source under the [AGPL-3.0 license](LICENSE). Copyright &copy; 2026 Felafax, Inc.
