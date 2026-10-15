/**
 * Captures the HTTP request the TypeScript server sends for each provider
 * family, through its real factory (`agent/provider-factory.ts`), reasoning
 * table (`agent/reasoning-config.ts`) and custom fetches. The Rust port's wire
 * test (`tests/llm_wire.rs`) compares its own requests against this output.
 *
 * Run from `packages/browseros-agent`:
 *   bun crates/browseros-agent/tests/fixtures/llm/capture-ts.ts
 * It rewrites (and biome-formats) `ts-requests.json` next to this file: per `<scenario>/<step>`, the
 * resolved config the Rust test rebuilds the model from, and the request TS sent.
 * Credentials are fake and nothing leaves the process: `fetch` is a recorder.
 */

import {
  jsonSchema,
  type ModelMessage,
  streamText,
  tool,
} from '../../../../../apps/server/node_modules/ai'
import { createLanguageModel } from '../../../../../apps/server/src/agent/provider-factory'
import { buildAgentReasoningConfig } from '../../../../../apps/server/src/agent/reasoning-config'
import type { ResolvedAgentConfig } from '../../../../../apps/server/src/agent/types'

interface Captured {
  url: string
  method: string
  headers: Record<string, string>
  body: unknown
}

/** A 48x32 crimson PNG: the screenshot every continuation returns. */
export const RED_PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAADAAAAAgCAIAAADbtmxLAAAAOElEQVR4nO3OQQ0AMAgEMEQgB/8qJmYuOB5NKqD1ek6p+EBISEgoPRASEhJKD4SEhITSAyEhoWUfDmAIarwCNQUAAAAASUVORK5CYII='

const SECRET_HEADERS = new Set([
  'authorization',
  'x-api-key',
  'api-key',
  'x-goog-api-key',
  'x-amz-date',
  'x-amz-content-sha256',
  'x-amz-security-token',
])

const captured: Captured[] = []

globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = input instanceof Request ? input.url : String(input)
  const headers: Record<string, string> = {}
  new Headers(init?.headers as HeadersInit).forEach((value, key) => {
    headers[key] = SECRET_HEADERS.has(key) ? '<redacted>' : value
  })
  const text = typeof init?.body === 'string' ? init.body : ''
  let body: unknown = text
  try {
    body = JSON.parse(text)
  } catch {}
  captured.push({ url, method: init?.method ?? 'GET', headers, body })
  return new Response(
    JSON.stringify({ error: { message: 'captured', type: 'invalid' } }),
    { status: 400, headers: { 'content-type': 'application/json' } },
  )
}) as typeof fetch

const SYSTEM = 'You are BrowserOS, a browser agent.'

const FIRST_STEP: ModelMessage[] = [
  { role: 'user', content: [{ type: 'text', text: 'What time is it?' }] },
]

const CONTINUATION: ModelMessage[] = [
  ...FIRST_STEP,
  {
    role: 'assistant',
    content: [
      { type: 'text', text: 'Checking.' },
      {
        type: 'tool-call',
        toolCallId: 'call_1',
        toolName: 'get_time',
        input: {},
      },
    ],
  },
  {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: 'call_1',
        toolName: 'get_time',
        output: {
          type: 'content',
          value: [
            { type: 'text', text: '{"utc":"2026-10-15T04:47:54Z"}' },
            {
              type: 'file',
              mediaType: 'image/png',
              data: { type: 'data', data: RED_PNG_BASE64 },
            },
          ],
        },
      },
    ],
  },
]

const TOOLS = {
  get_time: tool({
    description: 'Returns the current UTC time.',
    inputSchema: jsonSchema({
      type: 'object',
      properties: {},
      additionalProperties: false,
    }),
  }),
}

interface Scenario {
  name: string
  config: Partial<ResolvedAgentConfig> &
    Pick<ResolvedAgentConfig, 'provider' | 'model'>
  continuation?: boolean
}

const USER_HEADERS = { 'X-Trace': 'conv-{{conversationId}}' }

const SCENARIOS: Scenario[] = [
  {
    name: 'anthropic-default',
    config: {
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      apiKey: 'k',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'anthropic-opus-max',
    config: {
      provider: 'anthropic',
      model: 'claude-opus-4-8',
      apiKey: 'k',
      reasoningEffort: 'max',
    },
  },
  {
    name: 'anthropic-sonnet-4-6-low',
    config: {
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKey: 'k',
      reasoningEffort: 'low',
    },
  },
  {
    name: 'anthropic-haiku-high',
    config: {
      provider: 'anthropic',
      model: 'claude-haiku-4-5',
      apiKey: 'k',
      reasoningEffort: 'high',
    },
  },
  {
    name: 'anthropic-unknown-model',
    config: {
      provider: 'anthropic',
      model: 'my-proxy-model',
      apiKey: 'k',
      baseUrl: 'https://proxy.example/v1',
    },
  },
  {
    name: 'anthropic-no-reasoning',
    config: {
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      apiKey: 'k',
      supportsReasoning: false,
    },
  },
  {
    name: 'openai-default',
    config: {
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'k',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'openai-high',
    config: {
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'k',
      reasoningEffort: 'high',
    },
  },
  {
    name: 'azure-resource',
    config: {
      provider: 'azure',
      model: 'o3',
      apiKey: 'k',
      resourceName: 'myres',
      supportsReasoning: true,
    },
  },
  {
    name: 'azure-base-url',
    config: {
      provider: 'azure',
      model: 'my-deployment',
      apiKey: 'k',
      baseUrl: 'https://gateway.example/openai/v1',
    },
  },
  {
    name: 'google-2.5-high',
    config: {
      provider: 'google',
      model: 'gemini-2.5-pro',
      apiKey: 'k',
      reasoningEffort: 'high',
    },
    continuation: true,
  },
  {
    name: 'google-3-low',
    config: {
      provider: 'google',
      model: 'gemini-3.5-flash',
      apiKey: 'k',
      reasoningEffort: 'low',
    },
  },
  {
    name: 'google-2.0',
    config: { provider: 'google', model: 'gemini-2.0-flash', apiKey: 'k' },
  },
  {
    name: 'openrouter',
    config: {
      provider: 'openrouter',
      model: 'anthropic/claude-sonnet-4.5',
      apiKey: 'k',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'ollama',
    config: {
      provider: 'ollama',
      model: 'gemma4',
      baseUrl: 'http://127.0.0.1:11434/v1',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'lmstudio',
    config: {
      provider: 'lmstudio',
      model: 'qwen3',
      baseUrl: 'http://127.0.0.1:1234/v1',
      apiKey: 'k',
    },
  },
  {
    name: 'openai-compatible',
    config: {
      provider: 'openai-compatible',
      model: 'llama-4',
      baseUrl: 'https://compat.example/v1',
    },
  },
  {
    name: 'moonshot',
    config: {
      provider: 'moonshot',
      model: 'kimi-k2',
      baseUrl: 'https://api.moonshot.ai/v1',
      apiKey: 'k',
    },
  },
  {
    name: 'chatgpt-pro',
    config: {
      provider: 'chatgpt-pro',
      model: 'gpt-6-astra',
      apiKey: 'k',
      accountId: 'acct_123',
      reasoningEffort: 'high',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'github-copilot',
    config: {
      provider: 'github-copilot',
      model: 'gpt-5-mini',
      apiKey: 'k',
      headers: USER_HEADERS,
    },
    continuation: true,
  },
  {
    name: 'qwen-code',
    config: { provider: 'qwen-code', model: 'coder-model', apiKey: 'k' },
  },
  {
    name: 'bedrock-anthropic',
    config: {
      provider: 'bedrock',
      model: 'us.anthropic.claude-sonnet-4-5-20250929-v1:0',
      region: 'us-east-1',
      accessKeyId: 'AKIDEXAMPLE',
      secretAccessKey: 'secret',
      reasoningEffort: 'medium',
    },
    continuation: true,
  },
  {
    name: 'bedrock-nova',
    config: {
      provider: 'bedrock',
      model: 'amazon.nova-pro-v1:0',
      region: 'us-west-2',
      accessKeyId: 'AKIDEXAMPLE',
      secretAccessKey: 'secret',
    },
  },
]

async function capture(
  config: ResolvedAgentConfig,
  messages: ModelMessage[],
): Promise<Captured> {
  const start = captured.length
  const { model } = await createLanguageModel(config)
  const result = streamText({
    model,
    system: SYSTEM,
    messages,
    tools: TOOLS,
    maxRetries: 0,
    ...buildAgentReasoningConfig(config),
    onError: () => {},
  })
  for await (const _ of result.fullStream) {
    // Drain: the recorder answers 400, which ends the stream.
  }
  const request = captured[start]
  if (!request) throw new Error('no request captured')
  return request
}

const out: Record<string, { config: ResolvedAgentConfig; request: Captured }> =
  {}
for (const scenario of SCENARIOS) {
  // The chat request schema defaults `supportsReasoning` to true.
  const config = {
    conversationId: 'conv-1',
    supportsReasoning: true,
    ...scenario.config,
  } as ResolvedAgentConfig
  out[`${scenario.name}/first-step`] = {
    config,
    request: await capture(config, FIRST_STEP),
  }
  if (scenario.continuation) {
    out[`${scenario.name}/continuation`] = {
      config,
      request: await capture(config, CONTINUATION),
    }
  }
}

const target = new URL('./ts-requests.json', import.meta.url).pathname
await Bun.write(target, `${JSON.stringify(out, null, 2)}\n`)
// Keep the fixture in the repo's formatting so `bun run lint` stays clean.
Bun.spawnSync(['bunx', '@biomejs/biome', 'format', '--write', target])
