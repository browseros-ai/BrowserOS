/** Capture the installed TS server's behavior once; Rust tests consume these checked-in outputs.
 * MCP tools come from the real SDK client over its in-memory transport, so its private
 * toModelOutput implementation is exercised without a browser or an external service.
 * Run explicitly with `bun tools/server-compat/golden-agent.ts`; never during Rust tests.
 */
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import type {
  JSONRPCMessage,
  MCPTransport,
} from '../../apps/server/node_modules/@ai-sdk/mcp'
import {
  computeBudget,
  createCompactionPrepareStep,
  estimateTokens,
  getCurrentTokenCount,
  type StepWithUsage,
} from '../../apps/server/src/agent/compaction'
import { stripBinaryContent } from '../../apps/server/src/agent/compaction/content'
import { formatUserMessage } from '../../apps/server/src/agent/format-message'
import { normalizeMessagesForModel } from '../../apps/server/src/agent/message-normalization'
import {
  filterValidMessages,
  sanitizeMessagesForToolset,
  stripReasoningParts,
} from '../../apps/server/src/agent/message-validation'
import { buildNudgeToolSet } from '../../apps/server/src/agent/nudge-tools'
import { buildSystemPrompt } from '../../apps/server/src/agent/prompt'
import {
  describeMcpChange,
  describeModeChange,
  describeWorkspaceChange,
} from '../../apps/server/src/api/services/chat-service.helpers'
import { toModelOutput } from '../../apps/server/src/tools/filesystem/utils'

const serverRequire = createRequire(
  resolve(import.meta.dir, '../../apps/server/package.json'),
)
const { convertToModelMessages, pruneMessages, tool, jsonSchema } =
  serverRequire('ai') as typeof import('../../apps/server/node_modules/ai')
const { createMCPClient } = serverRequire(
  '@ai-sdk/mcp',
) as typeof import('../../apps/server/node_modules/@ai-sdk/mcp')
type ModelMessage = import('../../apps/server/node_modules/ai').ModelMessage
type UIMessage = import('../../apps/server/node_modules/ai').UIMessage
type Json = Record<string, unknown>

/** Only discovery is faked: the returned tools retain the production SDK output codec. */
class GoldenTransport implements MCPTransport {
  onmessage?: (message: JSONRPCMessage) => void
  async start() {}
  async close() {}
  async send(message: JSONRPCMessage) {
    if (!('id' in message) || !('method' in message)) return
    const result =
      message.method === 'initialize'
        ? {
            protocolVersion: '2025-06-18',
            capabilities: { tools: {} },
            serverInfo: { name: 'golden', version: '1' },
          }
        : { tools: [{ name: 'snapshot', inputSchema: { type: 'object' } }] }
    this.onmessage?.({ jsonrpc: '2.0', id: message.id, result })
  }
}

const client = await createMCPClient({
  transport: new GoldenTransport(),
})
const tools = {
  ...(await client.tools()),
  ...buildNudgeToolSet(),
  filesystem_read: tool({
    inputSchema: jsonSchema({ type: 'object' }),
    toModelOutput,
  }),
}
const catalog = Object.keys(tools).map((name) => ({
  name,
  dynamic: name === 'snapshot',
}))
const text = (text: string) => ({ type: 'text', text })
const message = (role: string, parts: Json[], id = 'fixture') => ({
  id,
  role,
  parts,
})
const call = (toolName: string, state: string, extra: Json = {}) => ({
  type: toolName === 'snapshot' ? 'dynamic-tool' : `tool-${toolName}`,
  ...(toolName === 'snapshot' ? { toolName } : {}),
  toolCallId: 'call',
  state,
  input: { url: 'https://example.com' },
  ...extra,
})
const conversionInputs: Json[] = [
  {
    name: 'system-and-user-metadata',
    history: [
      message('system', [
        { ...text('one'), providerMetadata: { a: { x: 1 } } },
        { ...text('two'), providerMetadata: { b: { x: 2 } } },
      ]),
      message('user', [
        text('hello 😀'),
        {
          type: 'file',
          mediaType: 'image/png',
          filename: 'screen.png',
          url: 'data:image/png;base64,AA==',
        },
      ]),
      message('user', [
        {
          type: 'file',
          mediaType: 'application/pdf',
          url: 'https://example.com/file',
          providerReference: { vendor: 'id' },
          providerMetadata: { x: { y: true } },
        },
      ]),
    ],
  },
  {
    name: 'step-splitting-and-mcp-output',
    history: [
      message('assistant', [
        { type: 'step-start' },
        {
          type: 'reasoning',
          text: 'thinking',
          providerMetadata: { x: { sig: 'abc' } },
        },
        call('snapshot', 'output-available', {
          output: {
            content: [
              text('page'),
              { type: 'image', data: 'AA==', mimeType: 'image/png' },
              { type: 'resource', resource: { uri: 'a', text: 'b' } },
            ],
          },
          callProviderMetadata: { x: { id: 'call' } },
        }),
        { type: 'step-start' },
        text('done'),
        {
          type: 'custom',
          kind: 'vendor-block',
          providerMetadata: { vendor: { opaque: 'x' } },
        },
        {
          type: 'reasoning-file',
          mediaType: 'image/png',
          url: 'https://example.com/thought',
        },
        { type: 'source-url', sourceId: 'source', url: 'https://example.com' },
        { type: 'data-notice', data: 'ignored' },
      ]),
    ],
  },
]
for (const state of [
  'input-streaming',
  'input-available',
  'approval-requested',
  'approval-responded',
  'output-available',
  'output-error',
  'output-denied',
]) {
  for (const providerExecuted of [false, true]) {
    conversionInputs.push({
      name: `${state}-provider-${providerExecuted}`,
      history: [
        message('assistant', [
          call('snapshot', state, {
            providerExecuted,
            output: { content: [text('result')] },
            errorText: 'failure',
            approval: {
              id: 'approve',
              approved: state !== 'output-denied',
              reason: 'because',
              requestReason: 'confirm',
              isAutomatic: false,
              inputSchemaInput: { a: 1 },
              signature: 'sig',
            },
            callProviderMetadata: { vendor: { phase: 'call' } },
            resultProviderMetadata: { vendor: { phase: 'result' } },
          }),
        ]),
      ],
    })
  }
}
conversionInputs.push(
  {
    name: 'approval-denied',
    history: [
      message('assistant', [
        call('snapshot', 'approval-responded', {
          approval: { id: 'deny', approved: false, reason: 'no' },
        }),
      ]),
    ],
  },
  {
    name: 'preliminary-result-is-incomplete',
    history: [
      message('assistant', [
        call('snapshot', 'output-available', { preliminary: true, output: {} }),
        text('still useful'),
      ]),
    ],
  },
  {
    name: 'error-metadata-and-raw-input',
    history: [
      message('assistant', [
        call('snapshot', 'output-error', {
          input: null,
          rawInput: '{bad',
          errorText: 'bad json',
          resultProviderMetadata: { vendor: { error: true } },
        }),
      ]),
    ],
  },
  {
    name: 'local-output-mappings',
    history: [
      message('assistant', [
        call('suggest_schedule', 'output-available', {
          output: { content: [text('{"type":"schedule_suggestion"}')] },
        }),
        call('suggest_app_connection', 'output-available', {
          output: { content: [] },
        }),
        call('filesystem_read', 'output-available', {
          output: {
            text: 'file',
            images: [{ data: 'AA==', mimeType: 'image/png' }],
          },
        }),
      ]),
    ],
  },
  {
    name: 'local-error-and-success',
    history: [
      message('assistant', [
        call('filesystem_read', 'output-available', {
          output: { text: '', isError: false },
        }),
        call('filesystem_read', 'output-available', {
          toolCallId: 'error',
          output: { text: 'denied', isError: true },
        }),
      ]),
    ],
  },
  {
    name: 'unknown-dynamic-json',
    history: [
      message('assistant', [
        {
          ...call('snapshot', 'output-available', { output: { z: 1, a: 2 } }),
          toolName: 'missing',
        },
      ]),
    ],
  },
  {
    name: 'unknown-static-json',
    history: [
      message('assistant', [
        call('missing', 'output-available', { output: { answer: 42 } }),
      ]),
    ],
  },
  {
    name: 'mcp-no-content',
    history: [
      message('assistant', [
        call('snapshot', 'output-available', {
          output: { structuredContent: { answer: 42 } },
        }),
      ]),
    ],
  },
  {
    name: 'assistant-file',
    history: [
      message('assistant', [
        {
          type: 'file',
          mediaType: 'audio/wav',
          url: 'https://example.com/audio',
          filename: 'voice.wav',
        },
      ]),
    ],
  },
  {
    name: 'ignored-parts-retain-empty-user',
    history: [
      message('user', [{ type: 'data-x', data: 'ignored' }]),
      message('assistant', [{ type: 'data-x', data: 'ignored' }]),
    ],
  },
)
conversionInputs.push({
  name: 'parse-error-without-input',
  history: [
    message('assistant', [
      call('snapshot', 'output-error', {
        input: undefined,
        errorText: 'parse failed',
      }),
    ]),
  ],
})
const conversion = []
for (const input of conversionInputs) {
  conversion.push({
    ...input,
    catalog,
    expected: await convertToModelMessages(input.history as UIMessage[], {
      tools,
      ignoreIncompleteToolCalls: true,
    }),
  })
}

const hygieneHistory = [
  message('assistant', []),
  message('user', [text(' \t\n')]),
  message('user', [text('\uFEFF')]),
  message('user', [text('\u0085')]),
  message('assistant', [{ type: 'reasoning', text: 'secret' }]),
  message('assistant', [{ type: 'reasoning', text: 'secret' }, text('answer')]),
  message('assistant', [call('missing', 'output-error', { errorText: 'bad' })]),
  message('assistant', [call('snapshot', 'output-available', { output: {} })]),
  message('assistant', [{ type: 'step-start' }]),
  message('assistant', [
    {
      type: 'reasoning-file',
      mediaType: 'image/png',
      url: 'https://example.com/thought',
    },
  ]),
] as UIMessage[]
const hygiene = [
  {
    history: hygieneHistory,
    toolNames: ['filesystem_read'],
    filtered: filterValidMessages(hygieneHistory),
    stripped: stripReasoningParts(hygieneHistory),
    sanitized: sanitizeMessagesForToolset(
      hygieneHistory,
      new Set(['filesystem_read']),
    ),
  },
]
const contentItems = [
  text('caption'),
  text(''),
  {
    type: 'file',
    mediaType: 'image/png',
    data: { type: 'data', data: 'AA==' },
  },
  {
    type: 'file',
    mediaType: 'application/pdf',
    filename: 'report.pdf',
    data: { type: 'data', data: 'BB==' },
  },
  {
    type: 'file',
    mediaType: 'image/png',
    data: { type: 'url', url: 'https://example.com/image' },
  },
  { type: 'image-data', mediaType: 'image/jpeg', data: 'CC==' },
  { type: 'file-data', mediaType: 'image/png', data: 'DD==' },
  {
    type: 'file-data',
    mediaType: 'audio/wav',
    filename: 'voice.wav',
    data: 'EE==',
  },
  { type: 'image-data', mediaType: 'audio/wav', data: 'FF==' },
  { type: 'image-url', url: 'https://example.com/image' },
  { type: 'file-url', url: 'https://example.com/doc' },
  { type: 'file-id', fileId: { vendor: 'id' } },
  { type: 'image-file-id', fileId: 'image-id' },
  { type: 'custom', kind: 'vendor', providerOptions: {} },
]
const modelResult = (value: Json[], id = 'call') => ({
  type: 'tool-result',
  toolCallId: id,
  toolName: 'snapshot',
  output: { type: 'content', value },
})
const normalization = []
for (const value of [contentItems, [contentItems[2]], [contentItems[3]], []]) {
  for (const supportsMediaInToolResults of [false, true])
    for (const supportsImages of [false, true]) {
      const messages = [
        { role: 'tool', content: [modelResult(value)] },
      ] as ModelMessage[]
      const options = { supportsMediaInToolResults, supportsImages }
      normalization.push({
        messages,
        options,
        expected: normalizeMessagesForModel(messages, options),
      })
    }
}
const budgets = [
  undefined,
  0,
  -1,
  0.5,
  1,
  2,
  3,
  5,
  20,
  1000,
  40000,
  128000,
  200000,
  200000.9,
  NaN,
  Infinity,
].map((input) => ({
  input:
    input === undefined
      ? null
      : Number.isNaN(input)
        ? 'NaN'
        : input === Infinity
          ? 'Infinity'
          : input,
  expected: computeBudget(input ?? 200000),
}))
const mixed = [
  { role: 'system', content: 'system 😀' },
  {
    role: 'user',
    content: [
      text('😀漢字'),
      { type: 'image', image: 'AA==' },
      { type: 'file', data: 'BB==', mediaType: 'application/pdf' },
    ],
  },
  {
    role: 'assistant',
    content: [
      text('answer'),
      { type: 'reasoning', text: 'think' },
      { type: 'reasoning-file', mediaType: 'image/png', data: 'ZZ==' },
      { type: 'file', mediaType: 'audio/wav', data: 'AA==' },
      {
        type: 'tool-call',
        toolCallId: 'c',
        toolName: 'snapshot',
        input: { n: 1.0, tiny: 1e-7, huge: 1e21, unicode: '😀' },
      },
      { type: 'tool-approval-request', approvalId: 'a', toolCallId: 'c' },
      modelResult(contentItems),
    ],
  },
  {
    role: 'tool',
    content: [
      modelResult(contentItems),
      {
        type: 'tool-approval-response',
        approvalId: 'a',
        approved: false,
        reason: 'no',
      },
      {
        type: 'tool-result',
        toolCallId: 'err',
        toolName: 'x',
        output: { type: 'execution-denied', reason: 'because' },
      },
    ],
  },
] as ModelMessage[]
const estimates = [
  [],
  mixed,
  ...mixed.map((m) => [m]),
  [
    {
      role: 'tool',
      content: [
        ...['text', 'error-text', 'json', 'error-json'].map((type) => ({
          type: 'tool-result',
          toolCallId: type,
          toolName: 'x',
          output: {
            type,
            value: type.includes('json') ? { z: 2, a: '😀' } : '😀',
          },
        })),
        {
          type: 'tool-result',
          toolCallId: 'denied',
          toolName: 'x',
          output: { type: 'execution-denied' },
        },
      ],
    },
  ],
].map((messages) => ({
  messages,
  expected: estimateTokens(messages as ModelMessage[]),
}))
const prepareInputs: Json[] = [
  { name: 'untouched', window: 200000, messages: mixed },
  { name: 'strip-binary-and-reasoning', window: 1000, messages: mixed },
  {
    name: 'clear-recent-tools',
    window: 1000,
    messages: [
      { role: 'user', content: [text('hello')] },
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 'x',
            toolName: 'snapshot',
            input: { large: 'x'.repeat(2000) },
          },
        ],
      },
      { role: 'tool', content: [modelResult([text('x'.repeat(2000))], 'x')] },
    ],
  },
  {
    name: 'drop-oldest',
    window: 1000,
    messages: Array.from({ length: 12 }, (_, i) => ({
      role: i % 2 ? 'assistant' : 'user',
      content: [text(`${i}:${'x'.repeat(170)}`)],
    })),
  },
  {
    name: 'floor-still-over',
    window: 1,
    messages: [
      { role: 'user', content: [text('x'.repeat(30))] },
      { role: 'assistant', content: [text('y'.repeat(30))] },
      { role: 'user', content: [text('z'.repeat(30))] },
    ],
  },
  {
    name: 'boundary-tool-pair',
    window: 1000,
    messages: [
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 'keep',
            toolName: 'snapshot',
            input: {},
          },
          {
            type: 'tool-call',
            toolCallId: 'drop',
            toolName: 'snapshot',
            input: {},
          },
        ],
      },
      { role: 'tool', content: [{ ...modelResult([text('old')], 'drop') }] },
      ...Array.from({ length: 5 }, () => ({
        role: 'user',
        content: [text('short')],
      })),
      { role: 'tool', content: [modelResult([text('new')], 'keep')] },
    ],
    steps: [{ usage: { inputTokens: 10000 } }],
  },
  {
    name: 'approval-keeps-originating-call',
    window: 1000,
    messages: [
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 'keep',
            toolName: 'snapshot',
            input: {},
          },
          {
            type: 'tool-approval-request',
            approvalId: 'approve',
            toolCallId: 'keep',
          },
        ],
      },
      ...Array.from({ length: 6 }, () => ({
        role: 'user',
        content: [text('short')],
      })),
      {
        role: 'tool',
        content: [
          {
            type: 'tool-approval-response',
            approvalId: 'approve',
            approved: true,
          },
        ],
      },
    ],
    steps: [{ usage: { inputTokens: 10000 } }],
  },
  {
    name: 'usage-can-skip-compaction',
    window: 1000,
    messages: mixed,
    steps: [{ usage: { inputTokens: 1, outputTokens: 1 } }],
    withoutTrailing: true,
  },
  {
    name: 'no-assistant-reported-usage',
    window: 1000,
    messages: [{ role: 'user', content: [text('x'.repeat(2000))] }],
    steps: [{ usage: { inputTokens: 1 } }],
  },
  {
    name: 'empty-content-removal',
    window: 1000,
    messages: [
      { role: 'system', content: '' },
      {
        role: 'assistant',
        content: [{ type: 'reasoning', text: 'x'.repeat(2000) }],
      },
      { role: 'user', content: [] },
    ],
  },
]
const prepare = []
for (const input of prepareInputs) {
  if (input.withoutTrailing)
    input.messages = (input.messages as ModelMessage[]).slice(0, 3)
  const messages = input.messages as ModelMessage[]
  const steps = (input.steps ?? []) as StepWithUsage[]
  const window = input.window as number
  const budget = computeBudget(window)
  const current = getCurrentTokenCount(steps, messages, budget.overhead)
  const c1 = pruneMessages({
    messages: stripBinaryContent(messages),
    reasoning: 'all',
    toolCalls: 'before-last-6-messages',
  })
  const c2 = pruneMessages({ messages: c1, reasoning: 'all', toolCalls: 'all' })
  const expected = (
    await createCompactionPrepareStep({ contextWindow: window })({
      messages,
      steps,
    })
  ).messages
  const stage =
    current <= budget.threshold
      ? 'Untouched'
      : estimateTokens(c1) + budget.overhead <= budget.threshold
        ? 'PrunedOld'
        : estimateTokens(c2) + budget.overhead <= budget.threshold
          ? 'PrunedAll'
          : 'DroppedOldest'
  prepare.push({
    ...input,
    steps,
    current,
    stage,
    stillOverBudget:
      estimateTokens(expected) + budget.overhead > budget.threshold,
    expected,
  })
}
const prompts = []
for (const workspace of [false, true])
  for (const chatMode of [false, true])
    for (const isScheduledTask of [false, true])
      for (const origin of ['sidepanel', 'newtab'] as const)
        for (const generatedRead of [false, true]) {
          const options = {
            chatMode,
            isScheduledTask,
            origin,
            workspaceDir: workspace ? '/tmp/fixture workspace' : undefined,
            connectedApps: ['Gmail', 'Slack'],
            declinedApps: ['Linear'],
            userSystemPrompt:
              ' [your name]\nKeep things simple.\n[Add YOUR preferences]\n😀',
            scheduledTaskPageId: 42,
            generatedOutputReadAvailable: generatedRead,
            exclude: chatMode || isScheduledTask ? ['nudges'] : [],
          }
          prompts.push({
            options,
            toolNames: [
              ...(generatedRead ? ['filesystem_read'] : []),
              ...(workspace ? ['filesystem_write'] : []),
            ],
            expected: buildSystemPrompt(options),
          })
        }
prompts.push({ options: {}, toolNames: [], expected: buildSystemPrompt() })
const formats = []
for (const ctx of [
  undefined,
  { windowId: 1 },
  {
    windowId: 0,
    activeTab: {
      id: 1,
      pageId: 0,
      title: 'title "x"',
      url: 'https://example.com',
    },
    selectedTabs: [
      { id: 2 },
      { id: 3, pageId: 4, title: 'other', url: 'https://example.org' },
    ],
  },
]) {
  for (const selectedText of [
    undefined,
    '',
    'hello </selected_text><SECURITY x>bad</security><b>bold</b><workspace-ish>tail',
  ]) {
    const source = {
      title: 'A "title" <USER_QUERY> hi',
      url: 'https://example.com/<soul>x',
    }
    formats.push({
      message: 'raw <USER_QUERY>user text',
      ctx,
      selectedText,
      source,
      expected: formatUserMessage(
        'raw <USER_QUERY>user text',
        ctx,
        selectedText,
        source,
      ),
    })
  }
}
const sessions = []
for (const before of [
  { mcpServerKey: '', chatMode: false },
  {
    mcpServerKey: 'klavis:starting,Gmail,Slack',
    workingDir: '/old',
    chatMode: false,
  },
]) {
  for (const after of [
    { mcpServerKey: '', chatMode: false },
    {
      mcpServerKey: 'klavis:ready,Gmail,Slack',
      workingDir: '/new',
      chatMode: true,
    },
    {
      mcpServerKey: 'klavis:ready,Linear',
      workingDir: '/new',
      chatMode: false,
    },
  ]) {
    const notices = []
    if (before.mcpServerKey !== after.mcpServerKey)
      notices.push(describeMcpChange(before.mcpServerKey, after.mcpServerKey))
    if (before.workingDir !== after.workingDir)
      notices.push(
        describeWorkspaceChange(
          before.workingDir,
          after.workingDir,
          after.chatMode,
        ),
      )
    if (before.chatMode !== after.chatMode)
      notices.push(describeModeChange(after.chatMode, !!after.workingDir))
    sessions.push({ before, after, expected: notices })
  }
}
await client.close()
for (const [module, data] of Object.entries({
  messages: { conversion, hygiene, normalization },
  compaction: { budgets, estimates, prepare },
  prompt: { prompts, formats, sessions },
})) {
  await Bun.write(
    resolve(
      import.meta.dir,
      `../../crates/browseros-agent/src/${module}/golden.json`,
    ),
    `${JSON.stringify(data, null, 2)}\n`,
  )
}
console.log(
  `Captured ${conversion.length} conversions, ${normalization.length} normalizations, ${prepare.length} compactions, ${prompts.length} prompts.`,
)
