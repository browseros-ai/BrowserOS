import { describe, expect, it } from 'bun:test'
import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type { ModelMessage, ToolResultPart } from 'ai'
import {
  computeBudget,
  createCompactionPrepareStep,
  estimateTokens,
  estimateTotalTokens,
  getCurrentTokenCount,
  type StepWithUsage,
} from '../../src/agent/compaction'
import {
  countBinaryParts,
  stripBinaryContent,
} from '../../src/agent/compaction/content'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from '../../src/agent/message-normalization'

const {
  COMPACTION_RESERVE_TOKENS,
  COMPACTION_FIXED_OVERHEAD,
  DEFAULT_CONTEXT_WINDOW,
} = AGENT_LIMITS

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function userMsg(text: string): ModelMessage {
  return { role: 'user', content: text }
}

function assistantMsg(text: string): ModelMessage {
  return { role: 'assistant', content: text }
}

function assistantToolCall(
  toolName: string,
  input: Record<string, unknown>,
): ModelMessage {
  return {
    role: 'assistant',
    content: [
      {
        type: 'tool-call',
        toolCallId: `call_${toolName}`,
        toolName,
        input,
      },
    ],
  }
}

function toolResult(
  toolName: string,
  text: string,
  toolCallId?: string,
): ModelMessage {
  return {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: toolCallId ?? `call_${toolName}`,
        toolName,
        output: { type: 'text' as const, value: text },
      },
    ],
  }
}

function toolResultJson(toolName: string, value: unknown): ModelMessage {
  return {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: `call_${toolName}`,
        toolName,
        output: { type: 'json' as const, value },
      },
    ],
  }
}

function toolResultContent(
  toolName: string,
  value: Extract<ToolResultPart['output'], { type: 'content' }>['value'],
): ModelMessage {
  return {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: `call_${toolName}`,
        toolName,
        output: { type: 'content' as const, value },
      },
    ],
  }
}

function userMsgWithImage(text: string): ModelMessage {
  return {
    role: 'user',
    content: [
      { type: 'text', text },
      { type: 'image', image: new Uint8Array([1, 2, 3]) },
    ],
  }
}

function repeat(char: string, count: number): string {
  return char.repeat(count)
}

function agentConfig(
  overrides: Partial<{
    provider: string
    model: string
    upstreamProvider: string
    supportsImages: boolean
  }> = {},
) {
  return {
    conversationId: 'test-conversation',
    provider: LLM_PROVIDERS.OPENROUTER,
    model: 'moonshotai/kimi-k2.5',
    workingDir: '/tmp/browseros-tests',
    ...overrides,
  }
}

function buildBrowserConversation(
  toolOutputSize: number,
  exchanges: number,
): ModelMessage[] {
  const messages: ModelMessage[] = [
    userMsg('Book me a flight from NYC to LAX on Kayak'),
  ]

  for (let i = 0; i < exchanges; i++) {
    messages.push(assistantToolCall(`action_${i}`, { step: i }))
    messages.push(toolResult(`action_${i}`, repeat('x', toolOutputSize)))
    messages.push(assistantMsg(`Completed step ${i}`))
  }

  return messages
}

/**
 * Asserts that no tool result survives without the assistant tool call that
 * produced it. This is the invariant that makes a compacted transcript a legal
 * request body: providers reject an orphaned tool result outright.
 */
function expectNoOrphanedToolResults(messages: ModelMessage[]): void {
  const seenCallIds = new Set<string>()

  for (const message of messages) {
    if (message.role === 'assistant' && Array.isArray(message.content)) {
      for (const part of message.content) {
        if (part.type === 'tool-call') {
          seenCallIds.add(part.toolCallId)
        }
      }
    }

    if (message.role === 'tool') {
      for (const part of message.content) {
        expect(seenCallIds.has(part.toolCallId)).toBe(true)
      }
    }
  }
}

const NO_STEPS: StepWithUsage[] = []

// ---------------------------------------------------------------------------
// computeBudget
// ---------------------------------------------------------------------------

describe('computeBudget: trigger threshold', () => {
  for (const size of [8_000, 16_000, 30_000, 40_000]) {
    it(`${size / 1000}K model → reserve is clamped to 50% of context`, () => {
      const { threshold } = computeBudget(size)
      expect(threshold).toBe(size - Math.floor(size * 0.5))
    })
  }

  for (const size of [64_000, 128_000, 200_000, 1_000_000]) {
    it(`${size / 1000}K model → reserve is fixed at COMPACTION_RESERVE_TOKENS`, () => {
      const { threshold } = computeBudget(size)
      expect(threshold).toBe(size - COMPACTION_RESERVE_TOKENS)
    })
  }
})

describe('computeBudget: overhead', () => {
  it('8K model → overhead is capped at 40% of context', () => {
    expect(computeBudget(8_000).overhead).toBe(3_200)
  })

  it('20K model → overhead is capped at 40% of context', () => {
    expect(computeBudget(20_000).overhead).toBe(8_000)
  })

  for (const size of [30_000, 64_000, 200_000]) {
    it(`${size / 1000}K model → overhead equals the constant`, () => {
      expect(computeBudget(size).overhead).toBe(COMPACTION_FIXED_OVERHEAD)
    })
  }

  // Without this, overhead alone exceeds the trigger and compaction never
  // stops firing on small models.
  for (const size of [4_000, 8_000, 16_000, 20_000, 30_000, 40_000, 200_000]) {
    it(`${size / 1000}K model → overhead stays under the threshold`, () => {
      const { threshold, overhead } = computeBudget(size)
      expect(overhead).toBeLessThan(threshold)
    })
  }
})

describe('computeBudget: invalid context windows', () => {
  // 0.5 is the interesting one: it is positive, so a `> 0` bound admits it,
  // and then flooring makes it a zero-token window.
  for (const invalid of [
    0,
    0.5,
    0.9,
    -1,
    Number.NaN,
    Number.POSITIVE_INFINITY,
  ]) {
    it(`${String(invalid)} falls back to the default context window`, () => {
      const budget = computeBudget(invalid)
      expect(budget.contextWindow).toBe(DEFAULT_CONTEXT_WINDOW)
      expect(budget.threshold).toBe(
        DEFAULT_CONTEXT_WINDOW - COMPACTION_RESERVE_TOKENS,
      )
    })
  }

  it('rounds a fractional context window down', () => {
    expect(computeBudget(200_000.9).contextWindow).toBe(200_000)
  })

  it('accepts the smallest usable window', () => {
    expect(computeBudget(1).contextWindow).toBe(1)
  })

  it('never produces a zero threshold', () => {
    for (const size of [0, 0.5, 1, 2, 100, 8_000, 200_000]) {
      expect(computeBudget(size).threshold).toBeGreaterThan(0)
    }
  })
})

// ---------------------------------------------------------------------------
// createCompactionPrepareStep
// ---------------------------------------------------------------------------

describe('createCompactionPrepareStep', () => {
  const contextWindow = 40_000
  const { threshold, overhead } = computeBudget(contextWindow)
  const prepareStep = createCompactionPrepareStep({ contextWindow })

  it('returns the messages untouched when under the threshold', async () => {
    const messages = buildBrowserConversation(200, 3)
    const result = await prepareStep({ messages, steps: NO_STEPS })
    expect(result.messages).toEqual(messages)
  })

  it('prunes older tool exchanges once over the threshold', async () => {
    const messages = buildBrowserConversation(4_000, 12)
    expect(estimateTotalTokens(messages, overhead)).toBeGreaterThan(threshold)

    const result = await prepareStep({ messages, steps: NO_STEPS })

    expect(estimateTotalTokens(result.messages, overhead)).toBeLessThanOrEqual(
      threshold,
    )
    expect(result.messages.length).toBeLessThan(messages.length)
  })

  it('keeps the original user request after pruning', async () => {
    const messages = buildBrowserConversation(4_000, 12)
    const result = await prepareStep({ messages, steps: NO_STEPS })

    expect(result.messages[0]).toEqual(messages[0])
  })

  it('never orphans a tool result', async () => {
    const messages = buildBrowserConversation(4_000, 12)
    const result = await prepareStep({ messages, steps: NO_STEPS })

    expectNoOrphanedToolResults(result.messages)
  })

  it('falls back to clearing every tool call when recent messages alone overflow', async () => {
    const messages = [
      userMsg('summarize this page'),
      assistantToolCall('snapshot', {}),
      toolResult('snapshot', repeat('x', 400_000)),
      assistantMsg('working on it'),
    ]

    const result = await prepareStep({ messages, steps: NO_STEPS })

    expect(estimateTotalTokens(result.messages, overhead)).toBeLessThanOrEqual(
      threshold,
    )
    expectNoOrphanedToolResults(result.messages)
    expect(result.messages.some((m) => m.role === 'tool')).toBe(false)
  })

  it('strips binary tool-result media before pruning', async () => {
    const messages = [
      userMsg('screenshot the page'),
      assistantToolCall('screenshot', {}),
      toolResultContent('screenshot', [
        { type: 'text', text: repeat('x', 200_000) },
        {
          type: 'image-data',
          data: repeat('A', 200_000),
          mediaType: 'image/png',
        },
      ]),
    ]

    expect(countBinaryParts(messages)).toBe(1)

    const result = await prepareStep({ messages, steps: NO_STEPS })

    expect(countBinaryParts(result.messages)).toBe(0)
  })

  it('prefers reported usage over estimation when deciding to compact', async () => {
    const messages = buildBrowserConversation(200, 3)
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: threshold + 1_000, outputTokens: 0 } },
    ]

    const result = await prepareStep({ messages, steps })

    // The transcript is tiny, so only the reported usage can have tripped it.
    expect(result.messages.length).toBeLessThan(messages.length)
  })

  it('is idempotent once the transcript is already at the floor', async () => {
    const messages = buildBrowserConversation(4_000, 12)
    const once = await prepareStep({ messages, steps: NO_STEPS })
    const twice = await prepareStep({
      messages: once.messages,
      steps: NO_STEPS,
    })

    expect(twice.messages).toEqual(once.messages)
  })

  it('uses the default context window when none is supplied', async () => {
    const defaultStep = createCompactionPrepareStep()
    const messages = buildBrowserConversation(4_000, 12)

    // 12 exchanges is far under a 200K window, so nothing should change.
    const result = await defaultStep({ messages, steps: NO_STEPS })
    expect(result.messages).toEqual(messages)
  })
})

describe('estimateTokens', () => {
  it('estimates text messages as chars/3', () => {
    const msgs = [userMsg('a'.repeat(300))]
    expect(estimateTokens(msgs)).toBe(100)
  })

  it('estimates tool result text', () => {
    const msgs = [toolResult('test', 'a'.repeat(600))]
    expect(estimateTokens(msgs)).toBe(200)
  })

  it('estimates tool result JSON', () => {
    const obj = { key: 'a'.repeat(100) }
    const msgs = [toolResultJson('test', obj)]
    const serialized = JSON.stringify(obj)
    expect(estimateTokens(msgs)).toBe(Math.ceil(serialized.length / 3))
  })

  it('estimates tool result content without counting base64 payload size', () => {
    const msgs = [
      toolResultContent('snapshot', [
        { type: 'text', text: 'Screenshot taken' },
        {
          type: 'image-data',
          data: 'x'.repeat(120_000),
          mediaType: 'image/png',
        },
      ]),
    ]

    const textTokens = Math.ceil('Screenshot taken'.length / 3)
    expect(estimateTokens(msgs)).toBe(textTokens + 1000)
  })

  it('counts images as 1000 tokens each', () => {
    const msgs = [userMsgWithImage('hello')]
    const textTokens = Math.ceil('hello'.length / 3)
    expect(estimateTokens(msgs)).toBe(textTokens + 1000)
  })

  it('counts multiple images', () => {
    const msg: ModelMessage = {
      role: 'user',
      content: [
        { type: 'text', text: 'compare these' },
        { type: 'image', image: new Uint8Array([1]) },
        { type: 'image', image: new Uint8Array([2]) },
      ],
    }
    const textTokens = Math.ceil('compare these'.length / 3)
    expect(estimateTokens([msg])).toBe(textTokens + 2000)
  })

  it('handles tool call input', () => {
    const msgs = [assistantToolCall('navigate', { url: 'https://example.com' })]
    const expected = Math.ceil(
      JSON.stringify({ url: 'https://example.com' }).length / 3,
    )
    expect(estimateTokens(msgs)).toBe(expected)
  })

  it('handles empty messages', () => {
    expect(estimateTokens([])).toBe(0)
  })
})

describe('stripBinaryContent', () => {
  it('replaces content outputs with placeholder text and counts media parts', () => {
    const msgs = [
      toolResultContent('snapshot', [
        { type: 'text', text: 'Before image' },
        {
          type: 'image-data',
          data: 'abcd',
          mediaType: 'image/png',
        },
        {
          type: 'file-data',
          data: 'efgh',
          mediaType: 'application/pdf',
          filename: 'report.pdf',
        },
      ]),
    ]

    const stripped = stripBinaryContent(msgs)
    const output = (
      stripped[0].content as Array<{ output: { type: string; value: string } }>
    )[0].output

    expect(countBinaryParts(msgs)).toBe(2)
    expect(output.type).toBe('text')
    expect(output.value).toContain('Before image')
    expect(output.value).toContain('[Image]')
    expect(output.value).toContain('[File: report.pdf]')
    expect(output.value).not.toContain('abcd')
    expect(output.value).not.toContain('efgh')
  })
})

describe('getMessageNormalizationOptions', () => {
  it('marks openrouter-compatible transports as requiring normalization', () => {
    expect(
      getMessageNormalizationOptions(
        agentConfig({ provider: LLM_PROVIDERS.OPENROUTER }),
      ).supportsMediaInToolResults,
    ).toBe(false)

    expect(
      getMessageNormalizationOptions(
        agentConfig({
          provider: LLM_PROVIDERS.BROWSEROS,
          upstreamProvider: LLM_PROVIDERS.OPENAI,
        }),
      ).supportsMediaInToolResults,
    ).toBe(false)
  })

  it('keeps native anthropic and openai transports unchanged', () => {
    expect(
      getMessageNormalizationOptions(
        agentConfig({ provider: LLM_PROVIDERS.ANTHROPIC }),
      ).supportsMediaInToolResults,
    ).toBe(true)
    expect(
      getMessageNormalizationOptions(
        agentConfig({ provider: LLM_PROVIDERS.OPENAI }),
      ).supportsMediaInToolResults,
    ).toBe(true)
  })
})

describe('normalizeMessagesForModel', () => {
  it('moves screenshot media into a follow-up user message for incompatible providers', () => {
    const messages = [
      assistantToolCall('snapshot', { page: 2 }),
      toolResultContent('snapshot', [
        { type: 'text', text: 'Captured screenshot' },
        {
          type: 'image-data',
          data: 'abcd',
          mediaType: 'image/png',
        },
      ]),
    ]

    const normalized = normalizeMessagesForModel(messages, {
      supportsImages: true,
      supportsMediaInToolResults: false,
    })

    expect(normalized).toHaveLength(3)

    const toolMessage = normalized[1]
    expect(toolMessage.role).toBe('tool')
    const output = (toolMessage.content as ToolResultPart[])[0].output
    expect(output.type).toBe('text')
    if (output.type === 'text') {
      expect(output.value).toContain('Captured screenshot')
      expect(output.value).toContain('[Image]')
      expect(output.value).not.toContain('abcd')
    }

    const mediaMessage = normalized[2]
    expect(mediaMessage.role).toBe('user')
    expect(Array.isArray(mediaMessage.content)).toBe(true)
    if (Array.isArray(mediaMessage.content)) {
      expect(mediaMessage.content[0]).toEqual({
        type: 'text',
        text: 'Attached image(s) from tool result:',
      })
      expect(mediaMessage.content[1]).toEqual({
        type: 'image',
        image: 'abcd',
        mediaType: 'image/png',
      })
    }
  })

  it('keeps media out of the prompt when the model does not support image input', () => {
    const messages = [
      assistantToolCall('snapshot', { page: 2 }),
      toolResultContent('snapshot', [
        { type: 'text', text: 'Captured screenshot' },
        {
          type: 'image-data',
          data: 'abcd',
          mediaType: 'image/png',
        },
      ]),
    ]

    const normalized = normalizeMessagesForModel(messages, {
      supportsImages: false,
      supportsMediaInToolResults: false,
    })

    expect(normalized).toHaveLength(2)
    const output = (normalized[1].content as ToolResultPart[])[0].output
    expect(output.type).toBe('text')
  })

  it('converts generic file attachments into follow-up user file parts', () => {
    const messages = [
      assistantToolCall('fetch_report', { id: 'report-1' }),
      toolResultContent('fetch_report', [
        { type: 'text', text: 'Downloaded report' },
        {
          type: 'file-data',
          data: 'cGRm',
          mediaType: 'application/pdf',
          filename: 'report.pdf',
        },
      ]),
    ]

    const normalized = normalizeMessagesForModel(messages, {
      supportsImages: true,
      supportsMediaInToolResults: false,
    })

    expect(normalized).toHaveLength(3)
    expect(normalized[2].role).toBe('user')
    if (Array.isArray(normalized[2].content)) {
      expect(normalized[2].content[0]).toEqual({
        type: 'text',
        text: 'Attached file(s) from tool result:',
      })
      expect(normalized[2].content[1]).toEqual({
        type: 'file',
        data: 'cGRm',
        mediaType: 'application/pdf',
        filename: 'report.pdf',
      })
    }
  })
})

// ---------------------------------------------------------------------------
// getCurrentTokenCount
// ---------------------------------------------------------------------------

describe('getCurrentTokenCount: additive trailing accounting', () => {
  const { overhead } = computeBudget(200_000)

  it('returns the estimate plus overhead when no steps exist', () => {
    const msgs = [userMsg('a'.repeat(400))]
    const result = getCurrentTokenCount([], msgs, overhead)
    expect(result).toBe(estimateTokens(msgs) + overhead)
  })

  it('returns the estimate when the last step has no usage', () => {
    const steps: StepWithUsage[] = [{ usage: undefined }]
    const msgs = [userMsg('hello')]
    const result = getCurrentTokenCount(steps, msgs, overhead)
    expect(result).toBe(estimateTokens(msgs) + overhead)
  })

  it('adds outputTokens to base when no trailing post-step messages remain', () => {
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 50_000, outputTokens: 2_000 } },
    ]
    const msgs = [userMsg('hello'), assistantMsg('response')]
    const result = getCurrentTokenCount(steps, msgs, overhead)
    expect(result).toBe(52_000)
  })

  it('adds trailing tool result tokens to base + output', () => {
    const toolOutput = 'x'.repeat(40_000) // ~10K tokens
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 100_000, outputTokens: 1_000 } },
    ]
    const msgs = [
      userMsg('hello'),
      assistantToolCall('snapshot', {}),
      toolResult('snapshot', toolOutput),
    ]

    const result = getCurrentTokenCount(steps, msgs, overhead)
    const expectedTrailing = estimateTokens([
      toolResult('snapshot', toolOutput),
    ])
    expect(result).toBe(100_000 + 1_000 + expectedTrailing)
  })

  it('catches large DOM snapshot that would bypass threshold', () => {
    // Simulates the original bug: last step saw 150K tokens,
    // then a 100K-char tool result (~25K tokens) is added
    const largeSnapshot = 'x'.repeat(100_000)
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 150_000, outputTokens: 500 } },
    ]
    const msgs = [
      userMsg('navigate to site'),
      assistantToolCall('snapshot', {}),
      toolResult('snapshot', largeSnapshot),
    ]

    const result = getCurrentTokenCount(steps, msgs, overhead)
    // Must be significantly above 150K. The old code returned a stale 150K.
    expect(result).toBeGreaterThan(170_000)
  })

  it('counts multiple trailing tool results', () => {
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 80_000, outputTokens: 1_000 } },
    ]
    const msgs = [
      userMsg('do things'),
      assistantToolCall('click', { selector: '#btn' }),
      toolResult('click', 'x'.repeat(4_000)),
      toolResult('snapshot', 'y'.repeat(8_000)),
    ]

    const result = getCurrentTokenCount(steps, msgs, overhead)
    const trailing1 = estimateTokens([toolResult('click', 'x'.repeat(4_000))])
    const trailing2 = estimateTokens([
      toolResult('snapshot', 'y'.repeat(8_000)),
    ])
    expect(result).toBe(80_000 + 1_000 + trailing1 + trailing2)
  })

  it('counts the synthetic follow-up user media message too', () => {
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 50_000, outputTokens: 500 } },
    ]
    const msgs = normalizeMessagesForModel(
      [
        userMsg('hello'),
        assistantToolCall('snapshot', {}),
        toolResultContent('snapshot', [
          { type: 'text', text: 'Captured screenshot' },
          {
            type: 'image-data',
            data: 'abcd',
            mediaType: 'image/png',
          },
        ]),
      ],
      {
        supportsImages: true,
        supportsMediaInToolResults: false,
      },
    )

    const result = getCurrentTokenCount(steps, msgs, overhead)
    const trailing = estimateTokens(msgs.slice(-2))

    expect(result).toBe(50_000 + 500 + trailing)
  })

  it('stops counting trailing at the most recent assistant message', () => {
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 50_000, outputTokens: 500 } },
    ]
    const msgs = [
      userMsg('hello'),
      assistantToolCall('click', {}),
      toolResult('click', 'x'.repeat(4_000)),
      assistantMsg('done'),
    ]

    const result = getCurrentTokenCount(steps, msgs, overhead)
    expect(result).toBe(50_500)
  })

  it('handles zero outputTokens gracefully', () => {
    const steps: StepWithUsage[] = [{ usage: { inputTokens: 50_000 } }]
    const msgs = [userMsg('hello')]
    const result = getCurrentTokenCount(steps, msgs, overhead)
    expect(result).toBe(50_000)
  })
})
