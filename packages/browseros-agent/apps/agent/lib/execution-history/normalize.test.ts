import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  getMessageText,
  getResponsePreview,
  normalizeExecutionSteps,
} from './normalize'

function asMessagePart(
  part: Record<string, unknown>,
): UIMessage['parts'][number] {
  return part as unknown as UIMessage['parts'][number]
}

function createAssistantMessage(parts: UIMessage['parts']): UIMessage {
  return {
    id: 'assistant-1',
    role: 'assistant',
    parts,
  } as UIMessage
}

describe('normalizeExecutionSteps', () => {
  it('filters nudge tools from the execution history', () => {
    const message = createAssistantMessage([
      asMessagePart({ type: 'text', text: 'I checked that for you.' }),
      asMessagePart({
        type: 'tool-suggest_schedule',
        toolCallId: 'nudge-1',
        state: 'output-available',
        input: { scheduleType: 'daily' },
        output: { suggestedName: 'Morning briefing' },
      }),
      asMessagePart({
        type: 'tool-open',
        toolCallId: 'tool-1',
        state: 'output-available',
        input: { ref_id: 'page-1' },
        output: { pageId: 1 },
      }),
    ])

    const normalized = normalizeExecutionSteps({
      assistantMessage: message,
      nowIso: '2026-03-26T10:00:00.000Z',
    })

    expect(normalized.assistantMessageId).toBe('assistant-1')
    expect(normalized.actionCount).toBe(1)
    expect(normalized.steps).toHaveLength(1)
    expect(normalized.steps[0]).toMatchObject({
      id: 'tool-1',
      toolName: 'open',
      state: 'output-available',
    })
  })

  it('preserves the original start time when a tool step reaches a terminal state', () => {
    const initialTimestamp = '2026-03-26T10:00:00.000Z'
    const completedTimestamp = '2026-03-26T10:00:04.000Z'

    const running = normalizeExecutionSteps({
      assistantMessage: createAssistantMessage([
        asMessagePart({
          type: 'tool-open',
          toolCallId: 'tool-1',
          state: 'input-available',
          input: { ref_id: 'page-1' },
        }),
      ]),
      nowIso: initialTimestamp,
    })

    const completed = normalizeExecutionSteps({
      assistantMessage: createAssistantMessage([
        asMessagePart({
          type: 'tool-open',
          toolCallId: 'tool-1',
          state: 'output-available',
          input: { ref_id: 'page-1' },
          output: { title: 'Example Domain' },
        }),
      ]),
      previousSteps: running.steps,
      nowIso: completedTimestamp,
    })

    expect(completed.steps[0]?.startedAt).toBe(initialTimestamp)
    expect(completed.steps[0]?.completedAt).toBe(completedTimestamp)
  })

  it('uses a compact preview for completed tool output', () => {
    const normalized = normalizeExecutionSteps({
      assistantMessage: createAssistantMessage([
        asMessagePart({
          type: 'tool-open',
          toolCallId: 'tool-1',
          state: 'output-available',
          input: { ref_id: 'page-1' },
          output: {
            content: [
              {
                type: 'text',
                text: 'Navigated to https://amazon.com. Additional context: page snapshot follows.',
              },
            ],
          },
        }),
      ]),
      nowIso: '2026-03-26T10:00:00.000Z',
    })

    expect(normalized.steps[0]?.previewText).toBe('Completed successfully')
  })

  it('surfaces ACL blocks as a compact issue label', () => {
    const normalized = normalizeExecutionSteps({
      assistantMessage: createAssistantMessage([
        asMessagePart({
          type: 'tool-click',
          toolCallId: 'tool-1',
          state: 'output-available',
          input: { x: 10, y: 20 },
          output: {
            content: [
              {
                type: 'text',
                text: "Action blocked by ACL rule: 'add to cart'. The element on this page is restricted.",
              },
            ],
          },
        }),
      ]),
      nowIso: '2026-03-26T10:00:00.000Z',
    })

    expect(normalized.steps[0]?.previewText).toBe('Blocked by ACL rule')
  })
})

describe('execution history text helpers', () => {
  it('joins text parts into a single response body', () => {
    const text = getMessageText({
      parts: [
        asMessagePart({ type: 'text', text: 'First line' }),
        asMessagePart({ type: 'text', text: 'Second line' }),
      ],
    })

    expect(text).toBe('First line\n\nSecond line')
  })

  it('truncates long response previews', () => {
    const preview = getResponsePreview({
      parts: [asMessagePart({ type: 'text', text: 'a'.repeat(220) })],
    })

    expect(preview).toHaveLength(180)
    expect(preview.endsWith('...')).toBe(true)
  })
})
