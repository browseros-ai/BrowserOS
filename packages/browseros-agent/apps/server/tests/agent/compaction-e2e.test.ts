import { describe, expect, it } from 'bun:test'
import type {
  LanguageModelV3CallOptions,
  LanguageModelV3GenerateResult,
  LanguageModelV3StreamPart,
  LanguageModelV3Usage,
} from '@ai-sdk/provider'
import {
  generateText,
  type ModelMessage,
  stepCountIs,
  type ToolResultPart,
  tool,
} from 'ai'
import { MockLanguageModelV3 } from 'ai/test'
import { z } from 'zod'
import {
  computeBudget,
  createCompactionPrepareStep,
  estimateTotalTokens,
} from '../../src/agent/compaction'
import { countBinaryParts } from '../../src/agent/compaction/content'
import { normalizeMessagesForModel } from '../../src/agent/message-normalization'

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

// biome-ignore lint/suspicious/noExplicitAny: test stubs for AI SDK internal types
type StepsStub = any

const SYSTEM_PROMPT =
  'You are a browser automation agent. SYSTEM_PROMPT_SENTINEL.'

function usage(inputTotal: number, outputTotal = 50): LanguageModelV3Usage {
  return {
    inputTokens: {
      total: inputTotal,
      noCache: inputTotal,
      cacheRead: undefined,
      cacheWrite: undefined,
    },
    outputTokens: { total: outputTotal, reasoning: undefined },
  }
}

function resultToStream(
  result: LanguageModelV3GenerateResult,
): ReadableStream<LanguageModelV3StreamPart> {
  return new ReadableStream({
    start(ctrl) {
      for (const part of result.content) {
        if (part.type === 'text') {
          ctrl.enqueue({ type: 'text-delta' as const, delta: part.text })
        } else if (part.type === 'tool-call') {
          const inputStr =
            typeof part.input === 'string'
              ? part.input
              : JSON.stringify(part.input)
          ctrl.enqueue({
            type: 'tool-call' as const,
            toolCallId: part.toolCallId,
            toolName: part.toolName,
            input: inputStr,
            delta: inputStr,
          })
        }
      }
      ctrl.enqueue({
        type: 'finish' as const,
        finishReason: result.finishReason,
        usage: result.usage,
      })
      ctrl.close()
    },
  })
}

type DoGenerateFn = (
  options: LanguageModelV3CallOptions,
) => Promise<LanguageModelV3GenerateResult>

function createMock(
  doGenerate: LanguageModelV3GenerateResult | DoGenerateFn,
): InstanceType<typeof MockLanguageModelV3> {
  const doGenerateFn =
    typeof doGenerate === 'function' ? doGenerate : async () => doGenerate

  return new MockLanguageModelV3({
    doGenerate: doGenerateFn,
    doStream: async (options: LanguageModelV3CallOptions) => {
      try {
        const result = await doGenerateFn(options)
        return { stream: resultToStream(result) }
      } catch (error) {
        return {
          stream: new ReadableStream<LanguageModelV3StreamPart>({
            start(ctrl) {
              ctrl.error(error)
            },
          }),
        }
      }
    },
  })
}

function textResponse(
  text: string,
  inputTokens: number,
): LanguageModelV3GenerateResult {
  return {
    content: [{ type: 'text', text }],
    finishReason: { unified: 'stop', raw: 'stop' },
    usage: usage(inputTokens),
  }
}

function toolCallResponse(
  toolName: string,
  input: Record<string, unknown>,
  inputTokens: number,
): LanguageModelV3GenerateResult {
  return {
    content: [
      {
        type: 'tool-call',
        toolCallId: `call_${toolName}_${Math.random().toString(36).slice(2, 8)}`,
        toolName,
        input: JSON.stringify(input),
      },
    ],
    finishReason: { unified: 'tool-calls', raw: 'tool_calls' },
    usage: usage(inputTokens),
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

/** Build messages with tool call/result pairs, the prunable shape. */
function buildToolExchanges(
  exchangeCount: number,
  outputChars = 1000,
): ModelMessage[] {
  const messages: ModelMessage[] = [
    { role: 'user', content: 'Do a multi-step browser task' },
  ]
  for (let i = 0; i < exchangeCount; i++) {
    messages.push({
      role: 'assistant',
      content: [
        {
          type: 'tool-call',
          toolCallId: `call_${i}`,
          toolName: `action_${i}`,
          input: { step: i },
        },
      ],
    })
    messages.push({
      role: 'tool',
      content: [
        {
          type: 'tool-result',
          toolCallId: `call_${i}`,
          toolName: `action_${i}`,
          output: {
            type: 'text' as const,
            value: `Result ${i}: ${'x'.repeat(outputChars)}`,
          },
        },
      ],
    })
    messages.push({ role: 'assistant', content: `Step ${i} done.` })
  }
  return messages
}

/** Text-only exchanges: nothing here is prunable, so they exercise the floor. */
function buildTextHeavyMessages(
  exchangeCount: number,
  charsPerMessage: number,
): ModelMessage[] {
  const messages: ModelMessage[] = [
    { role: 'user', content: 'Do a multi-step analysis task' },
  ]
  for (let i = 0; i < exchangeCount; i++) {
    messages.push({
      role: 'user',
      content: `Question ${i}: ${'q'.repeat(charsPerMessage)}`,
    })
    messages.push({
      role: 'assistant',
      content: `Analysis ${i}: ${'a'.repeat(charsPerMessage)}`,
    })
  }
  return messages
}

const testTools = {
  get_page_content: tool({
    description: 'Gets page content',
    parameters: z.object({ pageId: z.number() }),
    execute: async ({ pageId }) =>
      `Page ${pageId}: ${'Lorem ipsum dolor sit amet. '.repeat(1000)}`,
  }),
  click_element: tool({
    description: 'Clicks an element',
    parameters: z.object({ selector: z.string() }),
    execute: async ({ selector }) =>
      `Clicked ${selector}: ${'Result data. '.repeat(500)}`,
  }),
  navigate_to: tool({
    description: 'Navigate to URL',
    parameters: z.object({ url: z.string() }),
    execute: async ({ url }) =>
      `Navigated to ${url}: ${'Page content. '.repeat(500)}`,
  }),
}

function systemTextOf(prompt: LanguageModelV3CallOptions['prompt']): string {
  const parts: string[] = []
  for (const msg of prompt) {
    if (msg.role !== 'system') continue
    const content = msg.content
    if (typeof content === 'string') {
      parts.push(content)
    } else if (Array.isArray(content)) {
      for (const part of content) {
        if ('text' in part && typeof part.text === 'string') {
          parts.push(part.text)
        }
      }
    }
  }
  return parts.join('\n')
}

function expectNoOrphanedToolResults(
  prompt: LanguageModelV3CallOptions['prompt'],
): void {
  const seenCallIds = new Set<string>()

  for (const msg of prompt) {
    if (msg.role === 'assistant' && Array.isArray(msg.content)) {
      for (const part of msg.content) {
        if (part.type === 'tool-call') {
          seenCallIds.add(part.toolCallId)
        }
      }
    }
    if (msg.role === 'tool') {
      for (const part of msg.content) {
        expect(seenCallIds.has(part.toolCallId)).toBe(true)
      }
    }
  }
}

// ---------------------------------------------------------------------------
// E2E: trigger logic
// ---------------------------------------------------------------------------

describe('compaction E2E: trigger logic', () => {
  it('does not compact when reported usage is below the threshold', async () => {
    const prepareStep = createCompactionPrepareStep({ contextWindow: 200_000 })
    const messages: ModelMessage[] = [
      { role: 'user', content: 'hello' },
      { role: 'assistant', content: 'hi there' },
    ]

    const result = await prepareStep({
      messages,
      steps: [{ usage: { inputTokens: 500 } }] as StepsStub,
    })

    expect(result.messages).toEqual(messages)
  })

  it('compacts when reported usage exceeds the threshold', async () => {
    const contextWindow = 16_000
    const { threshold } = computeBudget(contextWindow)
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildToolExchanges(10, 1_000)
    const result = await prepareStep({
      messages,
      steps: [{ usage: { inputTokens: threshold + 1_000 } }] as StepsStub,
    })

    expect(result.messages.length).toBeLessThan(messages.length)
  })

  it('falls back to estimation on step 0 when no usage has been reported', async () => {
    const contextWindow = 16_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildToolExchanges(20, 2_000)
    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(result.messages.length).toBeLessThan(messages.length)
  })

  it('does not compact on step 0 when the transcript is small', async () => {
    const prepareStep = createCompactionPrepareStep({ contextWindow: 200_000 })
    const messages: ModelMessage[] = [
      { role: 'user', content: 'hello' },
      { role: 'assistant', content: 'hi' },
    ]

    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(result.messages).toEqual(messages)
  })

  it('ignores a zero inputTokens reading and estimates instead', async () => {
    const contextWindow = 16_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildToolExchanges(20, 2_000)
    const result = await prepareStep({
      messages,
      steps: [{ usage: { inputTokens: 0, outputTokens: 0 } }] as StepsStub,
    })

    expect(result.messages.length).toBeLessThan(messages.length)
  })

  it('preserves agent-normalized media messages when compaction does not trigger', async () => {
    const prepareStep = createCompactionPrepareStep({ contextWindow: 200_000 })
    const normalizedMessages = normalizeMessagesForModel(
      [
        { role: 'user', content: 'screenshot the page' },
        {
          role: 'assistant',
          content: [
            {
              type: 'tool-call',
              toolCallId: 'call_screenshot',
              toolName: 'screenshot',
              input: {},
            },
          ],
        },
        toolResultContent('screenshot', [
          { type: 'text', text: 'Captured screenshot' },
          { type: 'image-data', data: 'AAAA', mediaType: 'image/png' },
        ]),
      ],
      { supportsImages: true, supportsMediaInToolResults: false },
    )

    const result = await prepareStep({
      messages: normalizedMessages,
      steps: [{ usage: { inputTokens: 500 } }] as StepsStub,
    })

    expect(result.messages).toEqual(normalizedMessages)
  })

  it('strips content tool-result media before pruning', async () => {
    const contextWindow = 16_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages: ModelMessage[] = [
      { role: 'user', content: 'screenshot the page' },
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 'call_screenshot',
            toolName: 'screenshot',
            input: {},
          },
        ],
      },
      toolResultContent('screenshot', [
        { type: 'text', text: 'x'.repeat(60_000) },
        {
          type: 'image-data',
          data: 'A'.repeat(200_000),
          mediaType: 'image/png',
        },
      ]),
    ]

    expect(countBinaryParts(messages)).toBeGreaterThan(0)

    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(countBinaryParts(result.messages)).toBe(0)
  })
})

// ---------------------------------------------------------------------------
// E2E: convergence
// ---------------------------------------------------------------------------

describe('compaction E2E: convergence', () => {
  for (const contextWindow of [8_000, 16_000, 32_000, 200_000]) {
    it(`${contextWindow / 1000}K context: prunable transcripts land under the threshold`, async () => {
      const { threshold, overhead } = computeBudget(contextWindow)
      const prepareStep = createCompactionPrepareStep({ contextWindow })

      const messages = buildToolExchanges(40, 4_000)
      const result = await prepareStep({ messages, steps: [] as StepsStub })

      expect(
        estimateTotalTokens(result.messages, overhead),
      ).toBeLessThanOrEqual(threshold)
    })
  }

  // Plain text carries no tool calls and no reasoning, so both prune passes
  // are no-ops. Without a message-drop floor the transcript comes back
  // untouched and the provider rejects the request.
  // Message size is scaled to the window: dropping whole messages can only
  // help when several individual messages fit in the budget that is left
  // after overhead. See the oversized-message case below for the other side.
  for (const { contextWindow, charsPerMessage, exchanges } of [
    { contextWindow: 8_000, charsPerMessage: 600, exchanges: 20 },
    { contextWindow: 32_000, charsPerMessage: 3_000, exchanges: 20 },
    { contextWindow: 200_000, charsPerMessage: 12_000, exchanges: 40 },
  ]) {
    it(`${contextWindow / 1000}K context: text-only transcripts still land under the threshold`, async () => {
      const { threshold, overhead } = computeBudget(contextWindow)
      const prepareStep = createCompactionPrepareStep({ contextWindow })

      const messages = buildTextHeavyMessages(exchanges, charsPerMessage)
      expect(estimateTotalTokens(messages, overhead)).toBeGreaterThan(threshold)

      const result = await prepareStep({ messages, steps: [] as StepsStub })

      expect(
        estimateTotalTokens(result.messages, overhead),
      ).toBeLessThanOrEqual(threshold)
      expect(result.messages.length).toBeLessThan(messages.length)
    })
  }

  it('keeps the first message when dropping the oldest', async () => {
    const contextWindow = 8_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildTextHeavyMessages(60, 4_000)
    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(result.messages[0]).toEqual(messages[0])
  })

  it('keeps the most recent message when dropping the oldest', async () => {
    const contextWindow = 8_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildTextHeavyMessages(60, 4_000)
    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(result.messages.at(-1)).toEqual(messages.at(-1))
  })

  // Dropping whole messages cannot shrink a single message that is itself
  // larger than the window. The step must degrade, not throw.
  it('does not throw when one message alone exceeds the window', async () => {
    const contextWindow = 8_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages: ModelMessage[] = [
      { role: 'user', content: 'x'.repeat(400_000) },
      { role: 'assistant', content: 'y'.repeat(400_000) },
      { role: 'user', content: 'z'.repeat(400_000) },
    ]

    const result = await prepareStep({ messages, steps: [] as StepsStub })

    expect(result.messages.length).toBe(2)
    expect(result.messages[0]).toEqual(messages[0])
    expect(result.messages[1]).toEqual(messages[2])
  })

  it('reaches a fixed point after one pass', async () => {
    const contextWindow = 16_000
    const prepareStep = createCompactionPrepareStep({ contextWindow })

    const messages = buildToolExchanges(40, 4_000)
    const once = await prepareStep({ messages, steps: [] as StepsStub })
    const twice = await prepareStep({
      messages: once.messages,
      steps: [] as StepsStub,
    })

    expect(twice.messages).toEqual(once.messages)
  })
})

// ---------------------------------------------------------------------------
// E2E: generateText with tools and prepareStep
// ---------------------------------------------------------------------------

describe('compaction E2E: generateText with tools and prepareStep', () => {
  for (const contextWindow of [8_000, 16_000, 32_000, 200_000]) {
    const toolCallCount = contextWindow >= 200_000 ? 8 : 4

    it(`${contextWindow / 1000}K context: multi-tool conversation completes`, async () => {
      const { threshold } = computeBudget(contextWindow)
      const prepareStep = createCompactionPrepareStep({ contextWindow })
      let stepCount = 0

      const model = createMock(async () => {
        stepCount++
        if (stepCount <= toolCallCount) {
          const simulatedTokens = Math.floor(
            (stepCount / toolCallCount) * threshold * 1.2,
          )
          return toolCallResponse(
            'get_page_content',
            { pageId: stepCount },
            simulatedTokens,
          )
        }
        return textResponse('All pages processed successfully!', 5_000)
      })

      const result = await generateText({
        model,
        system: SYSTEM_PROMPT,
        tools: testTools,
        stopWhen: stepCountIs(toolCallCount + 5),
        prepareStep,
        messages: [
          { role: 'user', content: `Get content from ${toolCallCount} pages` },
        ],
      })

      expect(result.text).toContain('All pages processed')
      expect(result.steps.length).toBeGreaterThanOrEqual(toolCallCount + 1)
    })
  }

  // The system prompt reaches the model as `system`, never as a message, so
  // compaction cannot reach it. This is the regression guard for that.
  it('keeps the system prompt on every request, including after compaction', async () => {
    const contextWindow = 8_000
    const { threshold } = computeBudget(contextWindow)
    const prepareStep = createCompactionPrepareStep({ contextWindow })
    const systemTexts: string[] = []
    let stepCount = 0

    const model = createMock(async (options) => {
      systemTexts.push(systemTextOf(options.prompt))
      stepCount++
      if (stepCount <= 5) {
        return toolCallResponse(
          'click_element',
          { selector: `#btn-${stepCount}` },
          Math.floor((stepCount / 5) * threshold * 1.3),
        )
      }
      return textResponse('Done!', 5_000)
    })

    const result = await generateText({
      model,
      system: SYSTEM_PROMPT,
      tools: testTools,
      stopWhen: stepCountIs(10),
      prepareStep,
      messages: [{ role: 'user', content: 'Click 5 buttons' }],
    })

    expect(result.text).toContain('Done!')
    expect(systemTexts.length).toBeGreaterThan(1)
    for (const text of systemTexts) {
      expect(text).toContain('SYSTEM_PROMPT_SENTINEL')
    }
  })

  it('never sends an orphaned tool result to the model', async () => {
    const contextWindow = 8_000
    const { threshold } = computeBudget(contextWindow)
    const prepareStep = createCompactionPrepareStep({ contextWindow })
    const prompts: LanguageModelV3CallOptions['prompt'][] = []
    let stepCount = 0

    const model = createMock(async (options) => {
      prompts.push([...options.prompt])
      stepCount++
      if (stepCount <= 5) {
        return toolCallResponse(
          'click_element',
          { selector: `#btn-${stepCount}` },
          Math.floor((stepCount / 5) * threshold * 1.3),
        )
      }
      return textResponse('Done!', 5_000)
    })

    const result = await generateText({
      model,
      system: SYSTEM_PROMPT,
      tools: testTools,
      stopWhen: stepCountIs(10),
      prepareStep,
      messages: [{ role: 'user', content: 'Click 5 buttons' }],
    })

    expect(result.text).toContain('Done!')
    expect(prompts.length).toBeGreaterThan(1)
    for (const prompt of prompts) {
      expectNoOrphanedToolResults(prompt)
    }
  })

  it('keeps the original user request across compactions', async () => {
    const contextWindow = 8_000
    const { threshold } = computeBudget(contextWindow)
    const prepareStep = createCompactionPrepareStep({ contextWindow })
    const prompts: LanguageModelV3CallOptions['prompt'][] = []
    let stepCount = 0

    const model = createMock(async (options) => {
      prompts.push([...options.prompt])
      stepCount++
      if (stepCount <= 5) {
        return toolCallResponse(
          'navigate_to',
          { url: `https://page${stepCount}.com` },
          Math.floor((stepCount / 5) * threshold * 1.4),
        )
      }
      return textResponse('Navigation complete!', 5_000)
    })

    const result = await generateText({
      model,
      system: SYSTEM_PROMPT,
      tools: testTools,
      stopWhen: stepCountIs(10),
      prepareStep,
      messages: [{ role: 'user', content: 'ORIGINAL_REQUEST_SENTINEL' }],
    })

    expect(result.text).toContain('Navigation complete')
    for (const prompt of prompts) {
      const first = prompt.find((m) => m.role === 'user')
      expect(JSON.stringify(first)).toContain('ORIGINAL_REQUEST_SENTINEL')
    }
  })
})
