import { describe, expect, it } from 'bun:test'
import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type { ModelMessage, ToolResultPart } from 'ai'
import {
  computeConfig,
  estimateTokens,
  findSafeSplitPoint,
  getCurrentTokenCount,
  reduceToolOutputs,
  type StepWithUsage,
  slidingWindow,
} from '../../src/agent/compaction'
import {
  countBinaryParts,
  stripBinaryContent,
} from '../../src/agent/compaction/content'
import {
  buildSummarizationPrompt,
  buildTurnPrefixPrompt,
  messagesToTranscript,
} from '../../src/agent/compaction/prompt'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from '../../src/agent/message-normalization'

const {
  COMPACTION_RESERVE_TOKENS,
  COMPACTION_SMALL_CONTEXT_WINDOW,
  COMPACTION_KEEP_RECENT_FRACTION,
  COMPACTION_MAX_KEEP_RECENT,
  COMPACTION_MIN_SUMMARIZABLE_INPUT,
  COMPACTION_MIN_SUMMARIZABLE_INPUT_SMALL,
  COMPACTION_MAX_SUMMARIZATION_INPUT,
  COMPACTION_MIN_TOKEN_FLOOR,
  COMPACTION_SUMMARIZER_OUTPUT_RATIO,
} = AGENT_LIMITS

function expectedReserve(contextWindow: number): number {
  return contextWindow <= COMPACTION_SMALL_CONTEXT_WINDOW
    ? Math.floor(contextWindow * 0.5)
    : COMPACTION_RESERVE_TOKENS
}

function expectedTrigger(contextWindow: number): number {
  return Math.max(0, contextWindow - expectedReserve(contextWindow))
}

function expectedKeepRecent(contextWindow: number): number {
  return Math.max(
    0,
    Math.min(
      COMPACTION_MAX_KEEP_RECENT,
      Math.floor(
        expectedTrigger(contextWindow) * COMPACTION_KEEP_RECENT_FRACTION,
      ),
    ),
  )
}

function expectedAvailableToSummarize(contextWindow: number): number {
  return Math.max(
    0,
    expectedTrigger(contextWindow) - expectedKeepRecent(contextWindow),
  )
}

function expectedMinSummarizable(contextWindow: number): number {
  const base =
    contextWindow <= COMPACTION_SMALL_CONTEXT_WINDOW
      ? COMPACTION_MIN_SUMMARIZABLE_INPUT_SMALL
      : COMPACTION_MIN_SUMMARIZABLE_INPUT
  return Math.max(
    COMPACTION_MIN_TOKEN_FLOOR,
    Math.min(base, expectedAvailableToSummarize(contextWindow)),
  )
}

function expectedMaxSummarizationInput(contextWindow: number): number {
  return Math.min(
    COMPACTION_MAX_SUMMARIZATION_INPUT,
    Math.max(
      expectedMinSummarizable(contextWindow),
      expectedAvailableToSummarize(contextWindow),
    ),
  )
}

function expectedSummarizerMaxOutput(contextWindow: number): number {
  return Math.max(
    COMPACTION_MIN_TOKEN_FLOOR,
    Math.floor(
      expectedReserve(contextWindow) * COMPACTION_SUMMARIZER_OUTPUT_RATIO,
    ),
  )
}

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
        toolCallId: `call_${toolName}_${Date.now()}`,
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

// Build a realistic browser automation conversation
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

// ---------------------------------------------------------------------------
// computeConfig — Pi-style reserve trigger
// ---------------------------------------------------------------------------

describe('computeConfig — reserve trigger', () => {
  it('8K model → reserve is clamped to 50% of context', () => {
    const config = computeConfig(8_000)
    expect(config.reserveTokens).toBe(expectedReserve(8_000))
    expect(config.triggerThreshold).toBe(expectedTrigger(8_000))
    expect(config.triggerRatio).toBe(0.5)
  })

  it('16K model → reserve is clamped to 50% of context', () => {
    const config = computeConfig(16_000)
    expect(config.reserveTokens).toBe(expectedReserve(16_000))
    expect(config.triggerThreshold).toBe(expectedTrigger(16_000))
    expect(config.triggerRatio).toBe(0.5)
  })

  it('30K model → reserve is clamped to 50% of context', () => {
    const config = computeConfig(30_000)
    expect(config.reserveTokens).toBe(expectedReserve(30_000))
    expect(config.triggerThreshold).toBe(expectedTrigger(30_000))
    expect(config.triggerRatio).toBe(0.5)
  })

  it('32K model → reserve is clamped to 50% of context', () => {
    const config = computeConfig(32_000)
    expect(config.reserveTokens).toBe(expectedReserve(32_000))
    expect(config.triggerThreshold).toBe(expectedTrigger(32_000))
    expect(config.triggerRatio).toBe(0.5)
  })

  for (const size of [64_000, 200_000, 1_000_000]) {
    it(`${(size / 1000).toFixed(0)}K model → reserve is fixed at COMPACTION_RESERVE_TOKENS`, () => {
      const config = computeConfig(size)
      expect(config.reserveTokens).toBe(COMPACTION_RESERVE_TOKENS)
      expect(config.triggerThreshold).toBe(expectedTrigger(size))
      expect(config.triggerRatio).toBeCloseTo(expectedTrigger(size) / size, 3)
    })
  }
})

// ---------------------------------------------------------------------------
// computeConfig — keep-recent fraction with max cap
// ---------------------------------------------------------------------------

describe('computeConfig — keep-recent', () => {
  for (const size of [8_000, 16_000, 32_000, 64_000]) {
    it(`${(size / 1000).toFixed(0)}K model → keeps ${COMPACTION_KEEP_RECENT_FRACTION * 100}% of trigger budget`, () => {
      const config = computeConfig(size)
      expect(config.keepRecentTokens).toBe(expectedKeepRecent(size))
      expect(config.minSummarizableTokens).toBe(expectedMinSummarizable(size))
    })
  }

  for (const size of [200_000, 1_000_000]) {
    it(`${(size / 1000).toFixed(0)}K model → capped at COMPACTION_MAX_KEEP_RECENT`, () => {
      const config = computeConfig(size)
      expect(config.keepRecentTokens).toBe(COMPACTION_MAX_KEEP_RECENT)
    })
  }
})

// ---------------------------------------------------------------------------
// computeConfig — Pi-style summarization budgets
// ---------------------------------------------------------------------------

describe('computeConfig — summarization budgets', () => {
  for (const size of [16_000, 32_000]) {
    it(`${(size / 1000).toFixed(0)}K model → summarize budget is trigger minus keep-recent`, () => {
      const config = computeConfig(size)
      expect(config.maxSummarizationInput).toBe(
        expectedMaxSummarizationInput(size),
      )
      expect(config.summarizerMaxOutputTokens).toBe(
        expectedSummarizerMaxOutput(size),
      )
    })
  }

  it('20K model → min summarizable is clamped to available summarize budget', () => {
    const config = computeConfig(20_000)
    expect(config.minSummarizableTokens).toBe(expectedMinSummarizable(20_000))
    expect(config.maxSummarizationInput).toBe(
      expectedMaxSummarizationInput(20_000),
    )
  })

  for (const size of [200_000, 1_000_000]) {
    it(`${(size / 1000).toFixed(0)}K model → max summarization input capped at COMPACTION_MAX_SUMMARIZATION_INPUT`, () => {
      const config = computeConfig(size)
      expect(config.maxSummarizationInput).toBe(
        COMPACTION_MAX_SUMMARIZATION_INPUT,
      )
      expect(config.summarizerMaxOutputTokens).toBe(
        expectedSummarizerMaxOutput(size),
      )
    })
  }
})

// ---------------------------------------------------------------------------
// computeConfig — fixedOverhead scaling
// ---------------------------------------------------------------------------

describe('computeConfig — fixedOverhead scaling', () => {
  it('8K model → fixedOverhead capped at 40% of context', () => {
    const config = computeConfig(8_000)
    expect(config.fixedOverhead).toBe(Math.floor(8_000 * 0.4))
    expect(config.fixedOverhead).toBeLessThan(
      AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD,
    )
  })

  it('20K model → fixedOverhead capped at 40% of context', () => {
    const config = computeConfig(20_000)
    expect(config.fixedOverhead).toBe(Math.floor(20_000 * 0.4))
    expect(config.fixedOverhead).toBeLessThan(
      AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD,
    )
  })

  it('30K model → fixedOverhead equals constant (40% of 30K = 12K = constant)', () => {
    const config = computeConfig(30_000)
    expect(config.fixedOverhead).toBe(AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD)
  })

  for (const size of [64_000, 200_000, 1_000_000]) {
    it(`${(size / 1000).toFixed(0)}K model → fixedOverhead equals constant`, () => {
      const config = computeConfig(size)
      expect(config.fixedOverhead).toBe(AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD)
    })
  }

  it('30K model → fixedOverhead does not exceed trigger threshold', () => {
    const config = computeConfig(30_000)
    expect(config.fixedOverhead).toBeLessThanOrEqual(config.triggerThreshold)
  })

  it('20K model → fixedOverhead does not exceed trigger threshold', () => {
    const config = computeConfig(20_000)
    expect(config.fixedOverhead).toBeLessThanOrEqual(config.triggerThreshold)
  })
})

// ---------------------------------------------------------------------------
// estimateTokens
// ---------------------------------------------------------------------------

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
// findSafeSplitPoint
// ---------------------------------------------------------------------------

describe('findSafeSplitPoint', () => {
  it('returns splitIndex -1 for too few messages', () => {
    const msgs = [userMsg('hello'), assistantMsg('hi')]
    const result = findSafeSplitPoint(msgs, 1)
    expect(result.splitIndex).toBe(-1)
    expect(result.isSplitTurn).toBe(false)
  })

  it('returns splitIndex -1 when conversation is smaller than keepRecent', () => {
    const msgs = [userMsg('hello'), assistantMsg('hi'), userMsg('what')]
    // Total estimated ~3-4 tokens, keepRecent = 1000
    const result = findSafeSplitPoint(msgs, 1000)
    expect(result.splitIndex).toBe(-1)
    expect(result.isSplitTurn).toBe(false)
  })

  it('never cuts before a tool message', () => {
    // Build: user, assistant(tool_call), tool, assistant(text), user, assistant
    const msgs: ModelMessage[] = [
      userMsg('do something'),
      assistantToolCall('navigate', { url: 'https://example.com' }),
      toolResult('navigate', repeat('x', 2000)),
      assistantMsg('done navigating'),
      userMsg(repeat('y', 8000)),
      assistantMsg(repeat('z', 8000)),
    ]

    const result = findSafeSplitPoint(msgs, 2100)
    expect(result.splitIndex).toBeGreaterThan(0)
    expect(msgs[result.splitIndex].role).not.toBe('tool')
  })

  it('walks backward past tool messages to find safe cut', () => {
    const msgs: ModelMessage[] = [
      userMsg('start'),
      assistantMsg('ok'),
      assistantToolCall('click', { selector: '#btn' }),
      toolResult('click', repeat('x', 4000)), // walking back lands here — unsafe
      assistantToolCall('snapshot', {}),
      toolResult('snapshot', repeat('y', 4000)),
      assistantMsg(repeat('z', 8000)), // ~2000 tokens, keepRecent = 2500
    ]

    const result = findSafeSplitPoint(msgs, 2500)
    if (result.splitIndex !== -1) {
      expect(msgs[result.splitIndex].role).not.toBe('tool')
    }
  })

  it('splits correctly in a realistic browser automation flow', () => {
    // 10 exchanges, each tool output ~4000 chars (~1000 tokens)
    const msgs = buildBrowserConversation(4000, 10)
    const result = findSafeSplitPoint(msgs, 3000)

    expect(result.splitIndex).toBeGreaterThan(0)
    expect(result.splitIndex).toBeLessThan(msgs.length)
    expect(msgs[result.splitIndex].role).not.toBe('tool')

    const keptTokens = estimateTokens(msgs.slice(result.splitIndex))
    expect(keptTokens).toBeGreaterThanOrEqual(3000)
  })

  it('handles assistant tool_call followed by tool result pairs', () => {
    const msgs: ModelMessage[] = [
      userMsg('start'),
      assistantToolCall('a', {}),
      toolResult('a', 'result a'),
      assistantToolCall('b', {}),
      toolResult('b', 'result b'),
      assistantToolCall('c', {}),
      toolResult('c', repeat('z', 4000)),
      assistantMsg('final answer'),
    ]

    const result = findSafeSplitPoint(msgs, 500)
    if (result.splitIndex !== -1) {
      const kept = msgs.slice(result.splitIndex)
      for (let i = 0; i < kept.length; i++) {
        if (kept[i].role === 'tool') {
          expect(i).toBeGreaterThan(0)
          expect(kept[i - 1].role).toBe('assistant')
        }
      }
    }
  })
})

// ---------------------------------------------------------------------------
// findSafeSplitPoint — split turn detection
// ---------------------------------------------------------------------------

describe('findSafeSplitPoint — split turn detection', () => {
  it('detects split turn when cut lands mid-turn (user+assistant+tool+assistant+tool)', () => {
    const msgs: ModelMessage[] = [
      userMsg('first request'),
      assistantMsg('done with first'),
      userMsg('order MacBook on Amazon'), // index 2 — turn start
      assistantToolCall('navigate', { url: 'https://amazon.com' }), // index 3
      toolResult('navigate', repeat('x', 4000)), // index 4
      assistantToolCall('click', { selector: '#buy' }), // index 5 — cut here
      toolResult('click', repeat('y', 4000)), // index 6
      assistantMsg(repeat('z', 8000)), // index 7
    ]

    // keepRecent should land the cut around index 5 (mid-turn)
    const result = findSafeSplitPoint(msgs, 2500)
    if (result.splitIndex !== -1 && result.splitIndex > 2) {
      expect(result.isSplitTurn).toBe(true)
      expect(result.turnStartIndex).toBe(2)
    }
  })

  it('does not flag split turn when cut is at user message', () => {
    const msgs: ModelMessage[] = [
      userMsg('first request'),
      assistantMsg('done'),
      userMsg(repeat('x', 8000)), // index 2 — this is where cut lands
      assistantMsg(repeat('y', 8000)),
    ]

    const result = findSafeSplitPoint(msgs, 2100)
    if (result.splitIndex !== -1 && msgs[result.splitIndex].role === 'user') {
      expect(result.isSplitTurn).toBe(false)
      expect(result.turnStartIndex).toBe(-1)
    }
  })

  it('does not flag split turn when user message is at index 0 (single turn)', () => {
    // One user message followed by many tool exchanges
    const msgs: ModelMessage[] = [
      userMsg('do everything'), // index 0
    ]
    for (let i = 0; i < 10; i++) {
      msgs.push(assistantToolCall(`action_${i}`, { step: i }))
      msgs.push(toolResult(`action_${i}`, repeat('x', 4000)))
    }
    msgs.push(assistantMsg(repeat('z', 8000)))

    const result = findSafeSplitPoint(msgs, 3000)
    if (result.splitIndex !== -1) {
      // When the only user message is at index 0, it's NOT a split turn
      // Regular summarization is better for this case
      expect(result.isSplitTurn).toBe(false)
      expect(result.turnStartIndex).toBe(-1)
    }
  })
})

// ---------------------------------------------------------------------------
// Splitting mechanics at different model sizes
// ---------------------------------------------------------------------------

describe('splitting at different context windows', () => {
  it('32K model — splits with realistic browser automation', () => {
    const config = computeConfig(32_000)
    const msgs = buildBrowserConversation(5000, 12)
    const totalTokens = estimateTokens(msgs)
    expect(totalTokens).toBeGreaterThan(12_800)

    const result = findSafeSplitPoint(msgs, config.keepRecentTokens)
    expect(result.splitIndex).toBeGreaterThan(0)
    expect(msgs[result.splitIndex].role).not.toBe('tool')

    const kept = msgs.slice(result.splitIndex)
    const keptTokens = estimateTokens(kept)
    expect(keptTokens).toBeGreaterThanOrEqual(config.keepRecentTokens)

    const toSummarize = msgs.slice(0, result.splitIndex)
    expect(toSummarize.length).toBeGreaterThan(0)
  })

  it('200K model — splits with long conversation', () => {
    const config = computeConfig(200_000)
    const msgs = buildBrowserConversation(10000, 50)
    const totalTokens = estimateTokens(msgs)
    expect(totalTokens).toBeGreaterThan(100_000)

    const result = findSafeSplitPoint(msgs, config.keepRecentTokens)
    expect(result.splitIndex).toBeGreaterThan(0)

    const kept = msgs.slice(result.splitIndex)
    const keptTokens = estimateTokens(kept)
    expect(keptTokens).toBeGreaterThanOrEqual(config.keepRecentTokens)
  })

  it('16K model — handles tight context', () => {
    const config = computeConfig(16_000)
    const msgs = buildBrowserConversation(2000, 5)
    const totalTokens = estimateTokens(msgs)

    if (totalTokens > 16_000 * config.triggerRatio) {
      const result = findSafeSplitPoint(msgs, config.keepRecentTokens)
      if (result.splitIndex !== -1) {
        expect(msgs[result.splitIndex].role).not.toBe('tool')
        const toSummarize = msgs.slice(0, result.splitIndex)
        expect(estimateTokens(toSummarize)).toBeGreaterThan(0)
      }
    }
  })

  it('keeps tool call + result pairs together after split', () => {
    for (const contextWindow of [16_000, 32_000, 64_000, 200_000, 1_000_000]) {
      const config = computeConfig(contextWindow)
      const msgs = buildBrowserConversation(4000, 8)
      const result = findSafeSplitPoint(msgs, config.keepRecentTokens)

      if (result.splitIndex === -1) continue

      const kept = msgs.slice(result.splitIndex)
      for (let i = 0; i < kept.length; i++) {
        if (kept[i].role === 'tool' && i === 0) {
          throw new Error(
            `Orphaned tool result at start of kept messages for ${contextWindow} context window`,
          )
        }
      }
    }
  })
})

// ---------------------------------------------------------------------------
// reduceToolOutputs
// ---------------------------------------------------------------------------

describe('reduceToolOutputs', () => {
  it('truncates protected recent outputs exceeding maxChars', () => {
    const msgs = [toolResult('test', 'a'.repeat(20_000))]
    const reduced = reduceToolOutputs(msgs, {
      maxChars: 15_000,
      keepRecentCount: 1,
    })

    const output = (
      reduced[0].content as Array<{ output: { value: string } }>
    )[0].output.value
    expect(output.length).toBeLessThan(20_000)
    expect(output).toContain('[... truncated')
  })

  it('clears older verbose outputs but protects the last two', () => {
    const msgs = [
      toolResult('old', 'x'.repeat(500)),
      toolResult('recent_0', 'y'.repeat(500)),
      toolResult('recent_1', 'z'.repeat(500)),
    ]
    const reduced = reduceToolOutputs(msgs, {
      maxChars: 200,
      keepRecentCount: 2,
      clearThreshold: 100,
    })

    const part = (
      reduced[0].content as Array<{ output: { type: string; value: string } }>
    )[0].output.value
    const protected0 = (
      reduced[1].content as Array<{ output: { value: string } }>
    )[0].output.value
    const protected1 = (
      reduced[2].content as Array<{ output: { value: string } }>
    )[0].output.value

    expect(part).toBe('[Cleared — 500 chars]')
    expect(protected0).toContain('[... truncated')
    expect(protected1).toContain('[... truncated')
  })

  it('does not modify non-tool messages', () => {
    const msgs = [userMsg('hello'), assistantMsg('world')]
    expect(
      reduceToolOutputs(msgs, { maxChars: 100, keepRecentCount: 2 }),
    ).toEqual(msgs)
  })

  it('normalizes content output before reduction', () => {
    const msgs = [
      toolResultContent('snapshot', [
        { type: 'text', text: 'Captured screenshot' },
        {
          type: 'image-data',
          data: 'x'.repeat(20_000),
          mediaType: 'image/png',
        },
      ]),
    ]
    const reduced = reduceToolOutputs(msgs, {
      maxChars: 100,
      keepRecentCount: 1,
      clearThreshold: 0,
    })

    const output = (
      reduced[0].content as Array<{ output: { type: string; value: string } }>
    )[0].output

    expect(output.type).toBe('text')
    expect(output.value).toContain('Captured screenshot')
    expect(output.value).toContain('[Image]')
    expect(output.value).not.toContain('x'.repeat(100))
  })
})

// ---------------------------------------------------------------------------
// slidingWindow
// ---------------------------------------------------------------------------

describe('slidingWindow', () => {
  it('keeps tool+assistant pairs together', () => {
    const msgs: ModelMessage[] = [
      assistantToolCall('a', {}),
      toolResult('a', repeat('x', 4000)),
      assistantToolCall('b', {}),
      toolResult('b', repeat('y', 4000)),
      userMsg('continue'),
    ]

    // maxTokens small enough to force dropping
    const windowed = slidingWindow(msgs, 1500)

    // Should not start with a tool result (that would be orphaned)
    if (windowed.length > 0 && windowed[0].role === 'tool') {
      // If it starts with tool, the next should be assistant
      expect(windowed.length).toBeGreaterThan(1)
    }
  })

  it('preserves at least 2 messages', () => {
    const msgs = [userMsg(repeat('x', 10000)), assistantMsg(repeat('y', 10000))]
    const windowed = slidingWindow(msgs, 100)
    expect(windowed.length).toBeGreaterThanOrEqual(2)
  })

  it('returns original when under threshold', () => {
    const msgs = [userMsg('hello'), assistantMsg('hi')]
    const windowed = slidingWindow(msgs, 100_000)
    expect(windowed).toEqual(msgs)
  })
})

// ---------------------------------------------------------------------------
// compaction-prompt: buildSummarizationPrompt
// ---------------------------------------------------------------------------

describe('buildSummarizationPrompt', () => {
  it('returns initial prompt when no existing summary', () => {
    const prompt = buildSummarizationPrompt(null)
    expect(prompt).toContain('Summarize the following')
    expect(prompt).toContain('## Goal')
    expect(prompt).toContain('## Active State')
    expect(prompt).not.toContain('<previous_summary>')
  })

  it('returns update prompt with previous summary', () => {
    const prompt = buildSummarizationPrompt('## Goal\nold stuff')
    expect(prompt).toContain('Update the existing summary')
    expect(prompt).toContain('PRESERVE all existing information')
    expect(prompt).toContain('<previous_summary>')
    expect(prompt).toContain('old stuff')
  })
})

// ---------------------------------------------------------------------------
// compaction-prompt: buildTurnPrefixPrompt
// ---------------------------------------------------------------------------

describe('buildTurnPrefixPrompt', () => {
  it('returns turn prefix prompt with expected sections', () => {
    const prompt = buildTurnPrefixPrompt()
    expect(prompt).toContain('PREFIX of a turn')
    expect(prompt).toContain('## Original Request')
    expect(prompt).toContain('## Early Progress')
    expect(prompt).toContain('## Context for Suffix')
  })
})

// ---------------------------------------------------------------------------
// compaction-prompt: messagesToTranscript
// ---------------------------------------------------------------------------

describe('messagesToTranscript', () => {
  it('serializes user messages', () => {
    const transcript = messagesToTranscript([userMsg('hello world')])
    expect(transcript).toBe('[User]: hello world')
  })

  it('serializes assistant text', () => {
    const transcript = messagesToTranscript([assistantMsg('I will help')])
    expect(transcript).toBe('[Assistant]: I will help')
  })

  it('serializes tool calls', () => {
    const transcript = messagesToTranscript([
      assistantToolCall('navigate_to', { url: 'https://example.com' }),
    ])
    expect(transcript).toContain('[Tool Call]: navigate_to(')
    expect(transcript).toContain('https://example.com')
  })

  it('serializes tool results', () => {
    const transcript = messagesToTranscript([
      toolResult('navigate_to', 'Navigated to Example'),
    ])
    expect(transcript).toContain(
      '[Tool Result] navigate_to: Navigated to Example',
    )
  })

  it('truncates large tool results to 2K', () => {
    const transcript = messagesToTranscript([
      toolResult('snapshot', repeat('x', 5000)),
    ])
    expect(transcript).toContain('[... truncated')
    // The tool output should be capped
    expect(transcript.length).toBeLessThan(5000)
  })

  it('serializes content tool results without leaking base64', () => {
    const transcript = messagesToTranscript([
      toolResultContent('snapshot', [
        { type: 'text', text: 'Captured screenshot' },
        {
          type: 'image-data',
          data: 'x'.repeat(10_000),
          mediaType: 'image/png',
        },
      ]),
    ])

    expect(transcript).toContain('[Tool Result] snapshot: Captured screenshot')
    expect(transcript).toContain('[Image]')
    expect(transcript).not.toContain('x'.repeat(100))
  })

  it('replaces images with [Image]', () => {
    const transcript = messagesToTranscript([userMsgWithImage('look at this')])
    expect(transcript).toContain('[Image]')
    expect(transcript).toContain('look at this')
  })

  it('handles a full conversation', () => {
    const msgs: ModelMessage[] = [
      userMsg('Open google.com'),
      assistantMsg("I'll navigate to Google."),
      assistantToolCall('navigate_to', { url: 'https://google.com' }),
      toolResult('navigate_to', 'Navigated to Google'),
      assistantMsg('I opened Google. What next?'),
      userMsg('Search for flights'),
    ]

    const transcript = messagesToTranscript(msgs)
    expect(transcript).toContain('[User]: Open google.com')
    expect(transcript).toContain("[Assistant]: I'll navigate to Google.")
    expect(transcript).toContain('[Tool Call]: navigate_to(')
    expect(transcript).toContain(
      '[Tool Result] navigate_to: Navigated to Google',
    )
    expect(transcript).toContain('[User]: Search for flights')
  })
})

// ---------------------------------------------------------------------------
// End-to-end: config + split coherence at all model sizes
// ---------------------------------------------------------------------------

describe('end-to-end config coherence', () => {
  const modelSizes = [
    8_000, 16_000, 32_000, 64_000, 128_000, 200_000, 1_000_000,
  ]

  for (const size of modelSizes) {
    it(`${(size / 1000).toFixed(0)}K model — trigger budget is partitioned into keep + summarize`, () => {
      const config = computeConfig(size)
      const triggerTokens = config.triggerThreshold

      // Trigger budget is partitioned into kept + summarizable portions.
      // For large windows the cap means leftover budget exists, so use >=.
      expect(triggerTokens).toBeGreaterThanOrEqual(
        config.keepRecentTokens + config.maxSummarizationInput,
      )
      expect(config.maxSummarizationInput).toBeGreaterThanOrEqual(
        config.minSummarizableTokens,
      )

      // keepRecent should never exceed context window
      expect(config.keepRecentTokens).toBeLessThan(size)

      // maxSummarizationInput should never exceed context window
      expect(config.maxSummarizationInput).toBeLessThanOrEqual(size)
    })
  }

  it('reserve is either half-context (tiny models) or COMPACTION_RESERVE_TOKENS (larger models)', () => {
    for (const size of [
      8_000, 16_000, 32_000, 64_000, 128_000, 200_000, 1_000_000,
    ]) {
      const config = computeConfig(size)
      expect(config.reserveTokens).toBe(expectedReserve(size))
    }
  })
})

// ---------------------------------------------------------------------------
// getCurrentTokenCount — Pi-style additive counting
// ---------------------------------------------------------------------------

describe('getCurrentTokenCount — Pi-style additive', () => {
  const config = computeConfig(200_000)

  it('returns estimated with safety margin when no steps exist', () => {
    const msgs = [userMsg('a'.repeat(400))]
    const result = getCurrentTokenCount([], msgs, config)
    const rawEstimate = estimateTokens(msgs, config.imageTokenEstimate)
    const expected =
      Math.ceil(rawEstimate * config.safetyMultiplier) + config.fixedOverhead
    expect(result).toBe(expected)
  })

  it('returns estimated when last step has no usage', () => {
    const steps: StepWithUsage[] = [{ usage: undefined }]
    const msgs = [userMsg('hello')]
    const result = getCurrentTokenCount(steps, msgs, config)
    const rawEstimate = estimateTokens(msgs, config.imageTokenEstimate)
    const expected =
      Math.ceil(rawEstimate * config.safetyMultiplier) + config.fixedOverhead
    expect(result).toBe(expected)
  })

  it('adds outputTokens to base when no trailing post-step messages remain', () => {
    const steps: StepWithUsage[] = [
      { usage: { inputTokens: 50_000, outputTokens: 2_000 } },
    ]
    const msgs = [userMsg('hello'), assistantMsg('response')]
    const result = getCurrentTokenCount(steps, msgs, config)
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

    const result = getCurrentTokenCount(steps, msgs, config)
    const expectedTrailing = estimateTokens(
      [toolResult('snapshot', toolOutput)],
      config.imageTokenEstimate,
    )
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

    const result = getCurrentTokenCount(steps, msgs, config)
    // Must be significantly above 150K — the old code returned 150K (stale)
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

    const result = getCurrentTokenCount(steps, msgs, config)
    const trailing1 = estimateTokens(
      [toolResult('click', 'x'.repeat(4_000))],
      config.imageTokenEstimate,
    )
    const trailing2 = estimateTokens(
      [toolResult('snapshot', 'y'.repeat(8_000))],
      config.imageTokenEstimate,
    )
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

    const result = getCurrentTokenCount(steps, msgs, config)
    const trailing = estimateTokens(msgs.slice(-2), config.imageTokenEstimate)

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

    const result = getCurrentTokenCount(steps, msgs, config)
    expect(result).toBe(50_500)
  })

  it('handles zero outputTokens gracefully', () => {
    const steps: StepWithUsage[] = [{ usage: { inputTokens: 50_000 } }]
    const msgs = [userMsg('hello')]
    const result = getCurrentTokenCount(steps, msgs, config)
    expect(result).toBe(50_000)
  })
})
