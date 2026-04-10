import { z } from 'zod'
import { defineToolWithCategory } from './framework'

const defineAssistantTool = defineToolWithCategory('assistant')

const BROWSEROS_INFO = `# BrowserOS — The Open-Source AI Browser

BrowserOS is an AI-native browser built on Chromium that turns plain English into browser actions. It runs AI agents locally on your machine, keeping your data private. Open source under AGPL-3.0.

**Docs:** https://docs.browseros.com/

---

## Modes

- **Chat Mode** — Ask questions about any webpage: summarize articles, extract data, translate content. Activate with Option+K. Works with any LLM, including local models.
- **Agent Mode** — Describe a task and the agent executes it: clicking, typing, navigating, filling forms, extracting data, and multi-step browser tasks. Best with Claude Opus 4.5 or Kimi K2.5.

---

## Core Features

### Bring Your Own LLM
Connect your preferred AI provider or run models locally. Supported providers: Gemini (free tier), Claude/Anthropic, OpenAI, OpenRouter (500+ models). Local options: Ollama, LM Studio. Configure at chrome://browseros/settings.
Learn more: https://docs.browseros.com/features/bring-your-own-llm

### Scheduled Tasks
Automate tasks on a schedule — daily, hourly, or every few minutes. Runs in a background window without interrupting your work. Use cases: morning briefings, LinkedIn automation, price monitoring. Requires BrowserOS to be open.
Learn more: https://docs.browseros.com/features/scheduled-tasks

### Filesystem Access
Grant the agent controlled access to a local folder to read files, write reports, and run shell commands. Sandboxed — cannot access parent directories. Combine web research with local file creation in a single task.
Learn more: https://docs.browseros.com/features/cowork

### Connect Apps (MCPs)
Link external apps so the agent can access them conversationally. Built-in integrations: Gmail, Google Calendar, Google Docs, Google Sheets, Google Drive, Slack, Notion, LinkedIn. Custom MCP servers supported via SSE endpoints. Credentials stored locally.
Learn more: https://docs.browseros.com/features/connect-mcps

### MCP Server for Developer Tools
Built-in MCP server exposes 31 browser automation tools to Claude Code, Gemini CLI, OpenAI Codex CLI, and Claude Desktop. Enables agentic coding (test web apps, read console errors, fix code), data extraction from authenticated pages, and programmatic browser control.
Learn more: https://docs.browseros.com/features/use-with-claude-code

### Chat & LLM Hub
Chat provides quick AI access across any webpage via the side panel. LLM Hub enables side-by-side comparison of up to 3 models simultaneously. Switch providers instantly with Option+L.
Learn more: https://docs.browseros.com/features/llm-chat-hub

### Ad Blocking
Built-in ad blocking powered by uBlock Origin with full Manifest V2 support. Blocks ~10x more ads than Chrome out of the box (68% vs 7% effectiveness). Faster page loads, less bandwidth, reduced tracking.
Learn more: https://docs.browseros.com/features/ad-blocking`

const VALID_TOPICS = [
  'overview',
  'bring-your-own-llm',
  'scheduled-tasks',
  'filesystem-access',
  'connect-apps',
  'mcp-server',
  'chat-hub',
  'ad-blocking',
] as const

const TOPIC_SECTIONS: Record<string, { start: string; end?: string }> = {
  overview: { start: '# BrowserOS', end: '## Core Features' },
  'bring-your-own-llm': {
    start: '### Bring Your Own LLM',
    end: '### Scheduled Tasks',
  },
  'scheduled-tasks': {
    start: '### Scheduled Tasks',
    end: '### Filesystem Access',
  },
  'filesystem-access': {
    start: '### Filesystem Access',
    end: '### Connect Apps',
  },
  'connect-apps': {
    start: '### Connect Apps',
    end: '### MCP Server for Developer Tools',
  },
  'mcp-server': {
    start: '### MCP Server for Developer Tools',
    end: '### Chat & LLM Hub',
  },
  'chat-hub': { start: '### Chat & LLM Hub', end: '### Ad Blocking' },
  'ad-blocking': { start: '### Ad Blocking' },
}

function getTopicContent(topic: string): string {
  const section = TOPIC_SECTIONS[topic]
  if (!section) return BROWSEROS_INFO

  const startIdx = BROWSEROS_INFO.indexOf(section.start)
  if (startIdx === -1) return BROWSEROS_INFO

  const endIdx = section.end ? BROWSEROS_INFO.indexOf(section.end) : undefined

  return endIdx !== undefined && endIdx !== -1
    ? BROWSEROS_INFO.slice(startIdx, endIdx).trim()
    : BROWSEROS_INFO.slice(startIdx).trim()
}

export const browseros_info = defineAssistantTool({
  name: 'browseros_info',
  description:
    'Get information about BrowserOS features, capabilities, and documentation links. Use when users ask "What is BrowserOS?", "What can BrowserOS do?", or about specific features.',
  input: z.object({
    topic: z
      .enum(VALID_TOPICS)
      .optional()
      .default('overview')
      .describe(
        'Specific topic to get info about. Use "overview" for general questions.',
      ),
  }),
  output: z.object({
    topic: z.enum(VALID_TOPICS),
    content: z.string(),
  }),
  handler: async (args, _ctx, response) => {
    const content = args.topic ? getTopicContent(args.topic) : BROWSEROS_INFO
    response.text(content)
    response.data({ topic: args.topic, content })
  },
})
