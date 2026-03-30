# @browseros-ai/agent-sdk

[![npm version](https://img.shields.io/npm/v/@browseros-ai/agent-sdk)](https://www.npmjs.com/package/@browseros-ai/agent-sdk)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Browser automation SDK for BrowserOS — navigate, interact, extract data, and verify page state using natural language.

Build automations that describe *what* to do, not *how* to do it. The SDK connects to a running BrowserOS instance and translates natural language instructions into browser actions using your choice of LLM provider.

## Prerequisites

- A running [BrowserOS](https://browseros.com) instance
- An API key for at least one [supported LLM provider](#llm-providers)

## Installation

```bash
npm install @browseros-ai/agent-sdk
# or
bun add @browseros-ai/agent-sdk
```

## Quick Start

```typescript
import { Agent } from '@browseros-ai/agent-sdk'
import { z } from 'zod'

const agent = new Agent({
  url: 'http://localhost:9100',
  llm: {
    provider: 'openai',
    apiKey: process.env.OPENAI_API_KEY,
  },
})

// Navigate to a page
await agent.nav('https://example.com')

// Perform actions with natural language
await agent.act('click the login button')

// Extract structured data
const { data } = await agent.extract('get all product names and prices', {
  schema: z.array(z.object({
    name: z.string(),
    price: z.number(),
  })),
})

// Verify page state
const { success, reason } = await agent.verify('user is logged in')
```

## Multi-Step Example

Combine navigation, actions, extraction, and verification for end-to-end automation:

```typescript
import { Agent } from '@browseros-ai/agent-sdk'
import { z } from 'zod'

const agent = new Agent({
  url: 'http://localhost:9100',
  llm: { provider: 'anthropic', apiKey: process.env.ANTHROPIC_API_KEY },
})

// 1. Navigate
await agent.nav('https://news.ycombinator.com')

// 2. Extract data
const { data: stories } = await agent.extract('get the top 5 stories with title, points, and link', {
  schema: z.array(z.object({
    title: z.string(),
    points: z.number(),
    link: z.string(),
  })),
})

// 3. Act on extracted data
await agent.act(`click on the story titled "${stories[0].title}"`)

// 4. Verify the result
const { success } = await agent.verify('the story page or external link has loaded')

console.log({ stories, navigationSuccess: success })
```

## API Reference

### `new Agent(options)`

Create a new agent instance.

```typescript
const agent = new Agent({
  url: string,           // BrowserOS server URL
  llm?: LLMConfig,       // Optional LLM configuration
  onProgress?: (event) => void,  // Progress callback
})
```

### `agent.nav(url, options?)`

Navigate to a URL.

```typescript
const { success } = await agent.nav('https://google.com')
```

### `agent.act(instruction, options?)`

Perform browser actions using natural language.

```typescript
// Simple action
await agent.act('click the submit button')

// With context interpolation
await agent.act('search for {{query}}', {
  context: { query: 'browseros' },
})

// Multi-step with limit
await agent.act('fill out the form and submit', {
  maxSteps: 15,
})
```

### `agent.extract(instruction, options)`

Extract structured data from the page.

```typescript
import { z } from 'zod'

const { data } = await agent.extract('get the page title', {
  schema: z.object({ title: z.string() }),
})
```

### `agent.verify(expectation, options?)`

Verify the current page state.

```typescript
const { success, reason } = await agent.verify('the form was submitted successfully')
```

## LLM Providers

| Provider | Config |
|----------|--------|
| OpenAI | `{ provider: 'openai', apiKey: '...' }` |
| Anthropic | `{ provider: 'anthropic', apiKey: '...' }` |
| Google | `{ provider: 'google', apiKey: '...' }` |
| Azure | `{ provider: 'azure', apiKey: '...', resourceName: '...' }` |
| OpenRouter | `{ provider: 'openrouter', apiKey: '...' }` |
| Ollama | `{ provider: 'ollama', baseUrl: 'http://localhost:11434' }` |
| LM Studio | `{ provider: 'lmstudio', baseUrl: 'http://localhost:1234' }` |
| AWS Bedrock | `{ provider: 'bedrock', region: '...', accessKeyId: '...' }` |
| OpenAI Compatible | `{ provider: 'openai-compatible', baseUrl: '...', apiKey: '...' }` |

## Progress Events

Track agent operations in real time:

```typescript
const agent = new Agent({
  url: 'http://localhost:9100',
  onProgress: (event) => {
    console.log(`[${event.type}] ${event.message}`)
  },
})
```

Event types: `nav`, `act`, `extract`, `verify`, `error`, `done`

## Error Handling

```typescript
import {
  NavigationError,
  ActionError,
  ExtractionError,
  VerificationError,
  ConnectionError
} from '@browseros-ai/agent-sdk'

try {
  await agent.act('click non-existent button')
} catch (error) {
  if (error instanceof ActionError) {
    console.error('Action failed:', error.message)
  }
}
```

## Links

- [Documentation](https://docs.browseros.com)
- [GitHub](https://github.com/browseros-ai/BrowserOS)
- [Changelog](./CHANGELOG.md)
- [Discord](https://discord.gg/YKwjt5vuKr)

## License

[AGPL-3.0-or-later](../../../../LICENSE)
