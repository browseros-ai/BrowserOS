/** Re-captures the pinned TS implementation, never Rust output. Run from the monorepo:
 * bun crates/ui-message-stream/tests/capture-fixtures.ts
 * --check verifies committed bytes and every reducer snapshot without rewriting them.
 * The model is fake; SDK chunk translation, reduction, and SSE framing are real.
 */
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import type {
  LanguageModelV3FinishReason,
  LanguageModelV3StreamPart,
} from '../../../apps/server/node_modules/@ai-sdk/provider'
import {
  createUIMessageStreamResponse,
  dynamicTool,
  jsonSchema,
  stepCountIs,
  streamText,
  tool,
  type UIMessage,
  type UIMessageChunk,
} from '../../../apps/server/node_modules/ai'
import { MockLanguageModelV3 } from '../../../apps/server/node_modules/ai/dist/test/index'

const serverRequire = createRequire(
  join(import.meta.dir, '../../../apps/server/package.json'),
)
// The public reader only publishes on writes. Capture the pinned reducer directly to also
// cover no-write transitions. Load it at runtime so tsc need not compile the SDK's sources.
type ReducerState = { message: UIMessage }
const {
  createStreamingUIMessageState,
  processUIMessageStream,
}: {
  createStreamingUIMessageState(options: {
    lastMessage: undefined
    messageId: string
  }): ReducerState
  processUIMessageStream(options: {
    stream: ReadableStream<UIMessageChunk>
    runUpdateMessageJob(
      job: (options: { state: ReducerState; write(): void }) => Promise<void>,
    ): Promise<void>
    onError(error: unknown): void
  }): ReadableStream<UIMessageChunk>
} = serverRequire(
  join(
    dirname(serverRequire.resolve('ai')),
    '../src/ui/process-ui-message-stream.ts',
  ),
)
if (serverRequire('ai/package.json').version !== '7.0.114') {
  throw new Error('Fixtures require the compatibility target ai@7.0.114')
}
const fixtures = join(import.meta.dir, 'fixtures')
const check = process.argv.includes('--check')
const metadata = {
  clientName: 'ai-sdk-mcp-client',
  toolName: 'snapshot',
  annotations: { readOnlyHint: true },
}
const envelope = JSON.stringify({
  error: {
    code: 'tool_execution_failed',
    message: 'fixture error',
    retryable: false,
  },
})
const readable = <T>(chunks: T[]) =>
  new ReadableStream<T>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk)
      controller.close()
    },
  })

async function save(name: string, chunks: UIMessageChunk[]) {
  const state = createStreamingUIMessageState({
    lastMessage: undefined,
    messageId: '',
  })
  const snapshots: unknown[] = []
  const stream = processUIMessageStream({
    stream: readable(chunks),
    async runUpdateMessageJob(job) {
      await job({ state, write() {} })
      snapshots.push(JSON.parse(JSON.stringify(state.message)))
    },
    onError() {},
  })
  const response = createUIMessageStreamResponse({ stream })
  if (name === 'builtin') {
    const headers = Object.fromEntries(response.headers.entries())
    const path = join(fixtures, 'headers.json')
    const text = `${JSON.stringify(headers, null, 2)}\n`
    if (check) {
      if (readFileSync(path, 'utf8') !== text)
        throw new Error('SDK header drift')
    } else await Bun.write(path, text)
  }
  const bytes = await response.text()
  for (const [suffix, content] of [
    ['sse', bytes],
    [
      'json',
      `{\n  "snapshots": [\n${snapshots.map((snapshot) => `    ${JSON.stringify(snapshot)}`).join(',\n')}\n  ],\n  "message": ${JSON.stringify(state.message)}\n}\n`,
    ],
  ]) {
    const path = join(fixtures, `${name}.${suffix}`)
    if (check) {
      if (readFileSync(path, 'utf8') !== content)
        throw new Error(`TS fixture drift: ${path}`)
    } else await Bun.write(path, content)
  }
}

// Recon A11.4: reasoning + text, streamed dynamic MCP tool, static nudge, unknown tool,
// then another model step. Outputs are deliberately fast and deterministic.
const usage = {
  inputTokens: {
    total: 1,
    noCache: 1,
    cacheRead: undefined,
    cacheWrite: undefined,
  },
  outputTokens: { total: 1, text: 1, reasoning: undefined },
}
const finish = (
  finishReason: LanguageModelV3FinishReason['unified'],
): LanguageModelV3StreamPart => ({
  type: 'finish',
  finishReason: { unified: finishReason, raw: finishReason },
  usage,
})
const model = new MockLanguageModelV3({
  doStream: [
    {
      stream: readable<LanguageModelV3StreamPart>([
        { type: 'stream-start', warnings: [] },
        { type: 'reasoning-start', id: 'r1' },
        { type: 'reasoning-delta', id: 'r1', delta: 'thinking' },
        { type: 'reasoning-end', id: 'r1' },
        { type: 'text-start', id: 't1' },
        { type: 'text-delta', id: 't1', delta: 'Hi' },
        { type: 'text-end', id: 't1' },
        { type: 'tool-input-start', id: 'call_1', toolName: 'snapshot' },
        { type: 'tool-input-delta', id: 'call_1', delta: '{"page":1}' },
        { type: 'tool-input-end', id: 'call_1' },
        {
          type: 'tool-call',
          toolCallId: 'call_1',
          toolName: 'snapshot',
          input: '{"page":1}',
        },
        {
          type: 'tool-call',
          toolCallId: 'call_2',
          toolName: 'suggest_schedule',
          input: '{"query":"q","suggestedName":"n","scheduleType":"daily"}',
        },
        {
          type: 'tool-call',
          toolCallId: 'call_3',
          toolName: 'nope',
          input: '{}',
        },
        finish('tool-calls'),
      ]),
    },
    {
      stream: readable<LanguageModelV3StreamPart>([
        { type: 'stream-start', warnings: [] },
        { type: 'text-start', id: 't2' },
        { type: 'text-delta', id: 't2', delta: 'Done' },
        { type: 'text-end', id: 't2' },
        finish('stop'),
      ]),
    },
  ],
})
const builtIn = streamText({
  model,
  prompt: 'hi',
  stopWhen: stepCountIs(2),
  tools: {
    snapshot: dynamicTool({
      title: 'Snapshot',
      metadata,
      inputSchema: jsonSchema({ type: 'object' }),
      execute: async () => ({
        content: [{ type: 'text', text: 'ok' }],
        isError: false,
      }),
    }),
    suggest_schedule: tool({
      inputSchema: jsonSchema({ type: 'object' }),
      execute: async () => ({
        content: [
          {
            type: 'text',
            text: '{"type":"schedule_suggestion","query":"q","suggestedName":"n","scheduleType":"daily","scheduleTime":"09:00"}',
          },
        ],
        isError: false,
      }),
    }),
  },
}).toUIMessageStream({ onError: () => envelope })
const builtInChunks: UIMessageChunk[] = []
for await (const chunk of builtIn) builtInChunks.push(chunk)
if (
  builtInChunks.filter((chunk) => chunk.type === 'start-step').length !== 2 ||
  builtInChunks.filter((chunk) => chunk.type === 'tool-output-available')
    .length !== 2
)
  throw new Error('Mock must exercise two steps and both tool results')
await save('builtin', builtInChunks)

// Recon B4.9 is abbreviated, so fill out block boundaries and use a fixed UUID. Keep the
// legacy spurious tool-input-error to prove the reducer handles old stored/replayed streams;
// DECISIONS 6 tells the new ACP engine to omit that chunk, covered separately below.
const acp: UIMessageChunk[] = [
  { type: 'start', messageId: '9e602f3e-19f9-4d94-a85e-0ca4044b2e9c' },
  { type: 'start-step' },
  { type: 'reasoning-start', id: 'acpx-2' },
  { type: 'reasoning-delta', id: 'acpx-2', delta: 'pondering' },
  { type: 'reasoning-end', id: 'acpx-2' },
  { type: 'text-start', id: 'acpx-3' },
  { type: 'text-delta', id: 'acpx-3', delta: 'Hello ' },
  { type: 'text-end', id: 'acpx-3' },
  {
    type: 'tool-input-start',
    toolCallId: 'tc1',
    toolName: 'browseros: navigate',
    dynamic: false,
  },
  {
    type: 'tool-input-delta',
    toolCallId: 'tc1',
    inputTextDelta: 'browseros: navigate (pending): https://x',
  },
  {
    type: 'tool-input-delta',
    toolCallId: 'tc1',
    inputTextDelta: 'browseros: navigate (completed): ok',
  },
  {
    type: 'tool-input-error',
    toolCallId: 'tc1',
    toolName: 'browseros: navigate',
    input:
      'browseros: navigate (pending): https://xbrowseros: navigate (completed): ok',
    providerExecuted: true,
    dynamic: true,
    errorText: 'The ACP agent failed to respond.',
  },
  {
    type: 'tool-output-available',
    toolCallId: 'tc1',
    output:
      'browseros: navigate (pending): https://xbrowseros: navigate (completed): ok',
    providerExecuted: true,
  },
  { type: 'reasoning-start', id: 'acpx-4' },
  { type: 'reasoning-delta', id: 'acpx-4', delta: '[Plan] 1. do thing' },
  { type: 'reasoning-end', id: 'acpx-4' },
  { type: 'text-start', id: 'acpx-5' },
  { type: 'text-delta', id: 'acpx-5', delta: 'Done' },
  { type: 'text-end', id: 'acpx-5' },
  { type: 'finish-step' },
  { type: 'finish', finishReason: 'stop' },
]
await save('acp-legacy', acp)
await save(
  'acp',
  acp.map(
    (chunk): UIMessageChunk =>
      chunk.type === 'tool-input-error'
        ? {
            type: 'tool-input-available',
            toolCallId: chunk.toolCallId,
            toolName: chunk.toolName,
            input: chunk.input,
            providerExecuted: true,
          }
        : chunk,
  ),
)

const interrupted: UIMessageChunk[] = [
  { type: 'start' },
  { type: 'start-step' },
  { type: 'text-start', id: 't' },
  { type: 'text-delta', id: 't', delta: 'line 1\n"quotes" \\ café 🙂' },
]
await save('prestream-error', [
  { type: 'start' },
  { type: 'error', errorText: envelope },
])
await save('midstream-error', [
  ...interrupted,
  { type: 'error', errorText: envelope },
  { type: 'finish-step' },
  { type: 'finish', finishReason: 'error' },
])
await save('abort', [
  ...interrupted,
  { type: 'abort', reason: 'AbortError: Conversation stopped' },
])

// Every remaining v7 type and lifecycle rule: metadata deep merge, data replacement/transience,
// approvals, cross-step output lookup, repeated ids, reset, and overlapping text/reasoning.
await save('extensions', [
  {
    type: 'start',
    messageMetadata: { z: { second: 2, first: 1 }, a: [1, 2] },
    messageId: 'extensions',
  },
  { type: 'start-step' },
  { type: 'text-start', id: 'shared', providerMetadata: { z: { b: 2, a: 1 } } },
  { type: 'reasoning-start', id: 'shared' },
  { type: 'finish-step' },
  { type: 'start-step' },
  {
    type: 'text-delta',
    id: 'shared',
    delta: 'overlap',
    providerMetadata: { a: { c: true } },
  },
  { type: 'text-end', id: 'shared' },
  {
    type: 'reasoning-delta',
    id: 'shared',
    delta: 'thought',
    providerMetadata: { p: { sig: 's' } },
  },
  { type: 'reasoning-end', id: 'shared' },
  {
    type: 'source-url',
    sourceId: 's',
    url: 'https://example.com',
    title: 'Source',
    providerMetadata: { p: { z: 1 } },
  },
  {
    type: 'source-document',
    sourceId: 'doc',
    mediaType: 'application/pdf',
    title: 'Doc',
    filename: 'doc.pdf',
  },
  { type: 'file', mediaType: 'image/png', url: 'data:image/png;base64,AA==' },
  {
    type: 'reasoning-file',
    mediaType: 'image/png',
    url: 'data:image/png;base64,AA==',
    providerMetadata: { google: { thoughtSignature: 's' } },
  },
  {
    type: 'custom',
    kind: 'google.search',
    providerMetadata: { google: { q: 'hi' } },
  },
  { type: 'data-progress', id: 'n', data: { z: 1, a: null }, transient: false },
  { type: 'data-progress', id: 'n', data: null },
  { type: 'data-progress', id: 'n', data: 'discard', transient: true },
  { type: 'data-progress', data: [false, 'anonymous'] },
  {
    type: 'tool-input-available',
    toolCallId: 'c',
    toolName: 'run',
    input: { z: 1, a: 2 },
    providerMetadata: { p: { call: true } },
    title: 'Run',
  },
  {
    type: 'tool-approval-request',
    approvalId: 'ap',
    toolCallId: 'c',
    approvalDescriptor: { action: 'write' },
    inputSchemaInput: null,
    reason: 'Confirm',
    isAutomatic: true,
    signature: 'sig',
  },
  {
    type: 'tool-approval-response',
    approvalId: 'ap',
    approved: false,
    reason: 'Denied',
    providerExecuted: false,
    providerMetadata: { p: { approved: false } },
  },
  { type: 'tool-output-denied', toolCallId: 'c' },
  { type: 'finish-step' },
  { type: 'start-step' },
  {
    type: 'tool-output-available',
    toolCallId: 'c',
    output: null,
    providerExecuted: true,
    providerMetadata: { p: { result: true } },
    toolMetadata: { ignored: true },
    preliminary: true,
    dynamic: true,
  },
  {
    type: 'tool-output-error',
    toolCallId: 'c',
    errorText: 'failed',
    providerExecuted: false,
    providerMetadata: { p: { result: false } },
    dynamic: true,
  },
  {
    type: 'tool-input-available',
    toolCallId: 'c',
    toolName: 'new',
    input: null,
  },
  { type: 'tool-output-available', toolCallId: 'c', output: 0 },
  {
    type: 'tool-input-start',
    toolCallId: 'reset',
    toolName: 't',
    dynamic: true,
  },
  { type: 'text-start', id: 'reset' },
  { type: 'text-delta', id: 'reset', delta: 'discard' },
  { type: 'reset-step' },
  { type: 'tool-output-available', toolCallId: 'c', output: 'earlier-step' },
  { type: 'text-start', id: 'reset' },
  { type: 'text-delta', id: 'reset', delta: 'kept' },
  { type: 'text-end', id: 'reset' },
  { type: 'message-metadata', messageMetadata: { z: { third: 3 }, a: [3] } },
  { type: 'message-metadata', messageMetadata: null },
  { type: 'finish-step' },
  {
    type: 'finish',
    finishReason: 'stop',
    messageMetadata: { z: { second: null } },
  },
])

// Compare every prefix, not just the completed input: /state and aborted runs persist these
// intermediate values. Include malformed/non-JSON ACP text and incomplete unicode escapes.
const partialInputs = [
  '',
  '{"q":"hello 🙂","values":[true,false,null,-1.2e3],"o":{"k":1}}',
  '[1,2,',
  '{"q":"a\\u263a"}',
  'true',
  'false',
  'null',
  '1e+',
  '{"a":-',
  'plain ACP status',
  '{"__proto__":{"x":1}}',
  '{"nested":{"constructor":{"prototype":{}}}}',
  '{"constructor":{"ordinary":true}}',
  '9007199254740993',
]
const partialChunks: UIMessageChunk[] = [
  { type: 'start' },
  { type: 'start-step' },
]
for (const [index, input] of partialInputs.entries()) {
  const id = `p${index}`
  partialChunks.push({
    type: 'tool-input-start',
    toolCallId: id,
    toolName: 'partial',
    dynamic: index % 2 === 0,
  })
  for (const char of input)
    partialChunks.push({
      type: 'tool-input-delta',
      toolCallId: id,
      inputTextDelta: char,
    })
}
await save('partial-inputs', partialChunks)

// JSON.stringify's exponent thresholds and integer-key ordering differ from serde_json's
// defaults. Values are still opaque tool output: framing must not change their bytes.
await save('json-values', [
  { type: 'start' },
  {
    type: 'tool-input-available',
    toolCallId: 'json',
    toolName: 'json',
    input: {},
  },
  {
    type: 'tool-output-available',
    toolCallId: 'json',
    output: {
      z: 1,
      a: 2,
      '10': 'ten',
      '2': 'two',
      '01': 'ordinary',
      '4294967295': 'ordinary too',
      nested: {
        b: [0, -0, 1.0, 1e-7, 1e-6, 1e20, 1e21, 1.2345678901234567],
        a: 'line\n\t"\\ café 🙂',
      },
    },
  },
  { type: 'finish', finishReason: 'stop' },
])
