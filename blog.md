# Shimmy-Browser: A Practical Fork of BrowserOS for Building Local AI Browsing

Shimmy-Browser is a development fork of BrowserOS, the open-source Chromium-based browser that runs AI agents locally and connects them to the web through the Chrome DevTools Protocol (CDP). If you want a working, inspectable stack for building AI-assisted browsing, Shimmy-Browser gives you a complete, end-to-end system: a browser, an extension UI, and a Bun server that runs the agent loop.

This post is a technical overview for general users who want to understand what the project is, how it works, and what you can do with it.

## What Shimmy-Browser actually is

Shimmy-Browser is not just a UI demo. It is a full system made of three cooperating pieces:

- **Chromium (or BrowserOS build)**: runs the browser with remote debugging enabled.
- **Agent extension (WXT + React)**: the UI you use to chat and give tasks, delivered as a browser extension.
- **Server (Bun)**: an HTTP server that runs the AI agent loop, exposes MCP tools, and talks to the browser over CDP.

In practice, those three parts let you ask questions about pages, trigger actions like clicking and typing, and run multi-step workflows. The agent drives the browser; the browser does not call the server. If CDP is down or ports are misaligned, the system cannot operate.

## Why a fork?

Shimmy-Browser is a development fork of BrowserOS. That means you get the full upstream architecture, but you can tailor identity, prompts, and product messaging to your needs. In this fork, the agent is rebranded as **Sup agent** (with customized logo, centered sidebar placement, and custom branding) instead of the default BrowserOS theme. This is especially useful for experimentation, internal deployments, or building on top of upstream without waiting for upstream changes.

## The core architecture

At a high level, Shimmy-Browser is a pipeline:

1. The extension UI collects your prompt.
2. The server receives it and runs the agent loop.
3. The agent uses tools (CDP, MCP, filesystem, memory) to act.
4. The browser performs the actions via CDP.
5. Results stream back into the UI.

The server is the brain. The browser is the body. The extension is the interface.

## What the agent can do

Because the server connects to the browser via CDP, the agent can perform real browser actions, not just simulate them. That includes:

- Clicking, typing, and navigating pages
- Extracting text and data from the DOM
- Managing tabs and windows
- Saving screenshots or PDFs
- Running external app integrations via MCP

The system is designed to run locally by default, so your data stays on your machine unless you choose to connect external services.

## What is MCP and why it matters

MCP (Model Context Protocol) gives the agent a way to call tools beyond the browser itself. In Shimmy-Browser, MCP is used for:

- External app integrations (for example, Google services)
- Additional tool catalogs exposed by the server
- Optional workflows that combine browser actions with APIs

You can think of MCP as the extension point that moves the system from "browser automation" to "agent platform." The browser is still the main surface, but MCP lets the agent do more than just click pages.

## Repository layout you will care about

The repo is split into two major parts:

- **packages/browseros**: Chromium patches, build scripts, packaging.
- **packages/browseros-agent**: the agent platform, including:
  - `apps/app` (extension UI)
  - `apps/server` (Bun server + agent loop)
  - shared packages and tooling

There is also a vendor submodule (`packages/browseros-agent/vendor/sup-agent`) that provides upstream skills and assets.

If you are building UI or behavior changes, you will mostly live in the agent package. If you are changing the browser itself (for example, toolbar behavior or built-in side panel), you will work in `packages/browseros` and rebuild the binary.

## Running the stack locally

You can start the full stack with a single command once dependencies and env files are set up. The quick path is:

1. Install dependencies in `packages/browseros-agent`.
2. Configure your local environment variables in `packages/browseros-agent/apps/app/.env.development` (making sure your browser path `BROWSEROS_BINARY` is correctly set).
3. Keep `BROWSEROS_CDP_PORT` and `BROWSEROS_SERVER_PORT` aligned.
4. Run the dev stack from the repo root:

```
bun run dev
```

This starts the agent (extension + browser) first, waits for the CDP port to be ready, and then starts the server. If you prefer manual control, you can start the agent and server in two terminals.

## What makes Shimmy-Browser useful

Here is the practical value of this fork if you are experimenting with AI browsing:

- **Local-first**: the agent loop runs on your machine.
- **Transparent**: you can inspect every prompt, tool call, and response.
- **Composable**: the architecture cleanly separates UI, server, and browser.
- **Customizable**: identity and product messaging can be tailored without waiting on upstream.

It is a strong foundation if you want to prototype agent behavior, test prompts in real UI, or develop new MCP tools.

## Latest Platform Enhancements

The Shimmy-Browser platform has recently received several critical updates to make development smoother and the user experience more seamless:

- **Rebranded to Sup Agent**: The visual elements have been completely tailored to the "Sup agent" identity. This includes center-aligned logos in the sidebar, updated branding headers, text placeholders, and custom search selectors.
- **Per-Window Session Persistence**: The extension now tracks conversation panels on a per-window basis. Switching tabs no longer resets the sidebar state, ensuring chat history is preserved throughout your sessions.
- **Active Tab Focus**: When the agent performs actions or navigates to a new webpage, the browser automatically switches tab focus to show the active work page.
- **Resilient Dev Connection Fallback**: Improved local connection reliability. The extension now falls back automatically to the local dev server port (`9111`) if the custom browser APIs (`chrome.browserOS`) are unavailable or slow to initialize.

## How to contribute

Shimmy-Browser is open source under AGPL-3.0. If you plan to contribute:

- Use the repo root README to understand layout and commands.
- Keep the submodule updated if you pull upstream.
- Follow linting and commit conventions.

The project is intentionally structured for contributors who want a single map of the codebase and a simple way to run the full stack.

## Where this goes next

Shimmy-Browser inherits a fast-moving upstream and adds a customization layer. That makes it a good place to explore:

- custom agent identities and system prompts
- new onboarding and default actions
- local-first privacy UX
- alternative LLM providers and routing

If you want to build a bespoke AI browser experience without rebuilding all of Chromium from scratch, Shimmy-Browser gives you a pragmatic starting point.

---

If you want to try the project or contribute, start with the repo README and the run guide. You can also star the GitHub repo and track updates there.
