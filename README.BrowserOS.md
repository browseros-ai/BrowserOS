<div align="center">

<img src="packages/browseros/resources/browseros/icons/product_logo_192.png" alt="BrowserOS" width="96" />

<h1>BrowserOS</h1>

<h3>The AI browser for humans.</h3>

Free · Open source · Everything runs on your machine

<a href="https://discord.gg/YKwjt5vuKr"><img src="https://img.shields.io/badge/Discord-555?logo=discord" alt="Discord" /></a>
<a href="https://dub.sh/browserOS-slack"><img src="https://img.shields.io/badge/Slack-555?logo=slack" alt="Slack" /></a>
<a href="https://x.com/browserOS_ai"><img src="https://img.shields.io/badge/@browserOS__ai-555?logo=x" alt="X / Twitter" /></a>
<a href="https://github.com/browseros-ai/BrowserOS"><img src="https://img.shields.io/github/stars/browseros-ai/BrowserOS?style=flat&logo=github&label=stars&color=4c71f2" alt="GitHub stars" /></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-555" alt="AGPL-3.0" /></a>

<a href="https://files.browseros.com/download/BrowserOS.dmg"><img src="https://img.shields.io/badge/Download-macOS-black?style=for-the-badge&logo=apple&logoColor=white" alt="Download for macOS" /></a>
<a href="https://files.browseros.com/download/BrowserOS_installer.exe"><img src="https://img.shields.io/badge/Download-Windows-0078D4?style=for-the-badge&logo=windows&logoColor=white" alt="Download for Windows" /></a>
<a href="https://files.browseros.com/download/BrowserOS.AppImage"><img src="https://img.shields.io/badge/Download-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Download for Linux" /></a>
<a href="https://cdn.browseros.com/download/BrowserOS.deb"><img src="https://img.shields.io/badge/Download-Debian-D70A53?style=for-the-badge&logo=debian&logoColor=white" alt="Download for Debian" /></a>

**[Website](https://www.browseros.com/browseros/)** · **[Docs](https://docs.browseros.com/browseros)** · **[Enterprise](mailto:founders@browseros.com?subject=Enterprise%3A%20BrowserOS&body=Hi%2C%0A%0AWe%27re%20looking%20at%20BrowserOS%20for%20our%20team.%0A%0ACompany%3A%0ATeam%20size%3A%0AWhat%20we%20want%20to%20automate%3A)**

</div>

> Looking for the browser your agents drive? That is **[BrowserOS neo](README.md)**, and it ships from this same repo.

BrowserOS is a free, open-source Chromium fork with an AI agent built into every new tab. Ask it to summarise a page, click through a flow, extract data, or run a scheduled task, and it uses 20+ built-in tools plus 40+ app integrations to get the work done. Bring your own AI keys or run everything locally with Ollama.

Every AI browser today asks you to sign into their cloud and hand over your data. BrowserOS is the one that doesn't. Same daily browser you already use, with a helpful agent one keystroke away.

## Get started

### 1. Install BrowserOS

```sh
brew install --cask browseros
```

Prefer a direct download? [macOS](https://files.browseros.com/download/BrowserOS.dmg) · [Windows](https://files.browseros.com/download/BrowserOS_installer.exe) · [Linux (AppImage)](https://files.browseros.com/download/BrowserOS.AppImage) · [Linux (Debian)](https://cdn.browseros.com/download/BrowserOS.deb)

### 2. Import from Chrome

One click brings over your bookmarks, passwords and extensions.

### 3. Connect your AI provider

Claude, OpenAI, Gemini, ChatGPT Pro via OAuth, or local models through Ollama or LM Studio.

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
- [BrowserOS docs](https://docs.browseros.com/browseros)
- Enterprise deployment: [founders@browseros.com](mailto:founders@browseros.com?subject=Enterprise%3A%20BrowserOS&body=Hi%2C%0A%0AWe%27re%20looking%20at%20BrowserOS%20for%20our%20team.%0A%0ACompany%3A%0ATeam%20size%3A%0AWhat%20we%20want%20to%20automate%3A)

## Contributing and license

BrowserOS is built in this repo alongside BrowserOS neo. See the [Contributing Guide](CONTRIBUTING.md) and the [architecture overview](README.md#architecture).

Open source under the [AGPL-3.0 license](LICENSE). Copyright &copy; 2026 Felafax, Inc.
