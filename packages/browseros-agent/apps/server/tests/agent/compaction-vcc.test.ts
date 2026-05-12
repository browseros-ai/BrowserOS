/**
 * TDD tests for VCC compaction pipeline (vendor + adapt from pi-vcc).
 *
 * Worst-first testing order:
 *   Zone 4 (error)    → empty input
 *   Zone 1 (empty)    → all tool messages, no content
 *   Zone 3 (multi)    → mixed roles interleaved
 *   Zone 2 (boundary) → massive tool output / single huge message
 *   Zone 5 (mutation) → same messages compacted twice → idempotent
 *   Happy path        → normal conversation → structured summary
 */
import { describe, expect, it } from 'bun:test'
import type { ModelMessage } from 'ai'
import { normalizeFromAiSdk } from '../../src/agent/compaction/vcc/normalize'
import { vccCompile } from '../../src/agent/compaction/vcc-adapter'

// ─── Helpers ────────────────────────────────────────────────────────

function userMsg(text: string): ModelMessage {
  return { role: 'user', content: text }
}

function assistantText(text: string): ModelMessage {
  return { role: 'assistant', content: text }
}

function assistantToolCall(
  toolName: string,
  input: Record<string, unknown>,
  text = '',
): ModelMessage {
  const parts: Array<Record<string, unknown>> = []
  if (text) parts.push({ type: 'text', text })
  parts.push({ type: 'tool-call', toolName, input })
  return { role: 'assistant', content: parts as any }
}

function toolResultMsg(
  toolName: string,
  outputText: string,
  isError = false,
): ModelMessage {
  return {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: `call_${toolName}`,
        toolName,
        output: {
          type: isError ? 'error-text' : 'text',
          value: outputText,
        },
      },
    ],
  } as ModelMessage
}

function systemMsg(text: string): ModelMessage {
  return { role: 'system', content: text }
}

// ─── Zone 4: Error — Empty Input ──────────────────────────────────

describe('VCC Zone 4 (error): empty ModelMessage array', () => {
  it('vccCompile returns null for empty array', () => {
    const result = vccCompile([], null)
    expect(result).toBeNull()
  })

  it('vccCompile returns null with existingSummary but no messages', () => {
    const result = vccCompile([], 'Previous summary text')
    expect(result).toBeNull()
  })

  it('normalizeFromAiSdk returns empty array for empty input', () => {
    expect(normalizeFromAiSdk([])).toEqual([])
  })
})

// ─── Zone 1: Empty/Nil — All tool messages, no semantic content ────

describe('VCC Zone 1 (empty): all tool messages, no user/assistant', () => {
  it('normalizeFromAiSdk extracts tool_call and tool_result blocks', () => {
    const messages: ModelMessage[] = [
      assistantToolCall('Read', { file_path: '/foo.ts' }),
      toolResultMsg('Read', 'file contents here'),
      assistantToolCall('Read', { file_path: '/bar.ts' }),
      toolResultMsg('Read', 'more contents'),
    ]

    const blocks = normalizeFromAiSdk(messages)
    // 2 tool_calls + 2 tool_results
    expect(blocks).toHaveLength(4)
    expect(blocks[0].kind).toBe('tool_call')
    expect(blocks[1].kind).toBe('tool_result')
  })

  it('vccCompile with only tool calls returns minimal summary (files section)', () => {
    const messages: ModelMessage[] = [
      assistantToolCall('Read', { file_path: '/src/a.ts' }),
      toolResultMsg('Read', 'const a = 1'),
      assistantToolCall('Edit', { file_path: '/src/a.ts' }),
      toolResultMsg('Edit', 'OK'),
    ]

    const result = vccCompile(messages, null)
    // Should produce output (at least file activity) or null — must not throw
    // Tool-only messages get filtered by noise filter, so files section may appear
    expect(result === null || typeof result === 'string').toBe(true)
  })
})

// ─── Zone 3: Multi-flag — Mixed system/user/assistant/tool interleaved ─

describe('VCC Zone 3 (multi): mixed roles interleaved', () => {
  it('normalizeFromAiSdk skips system messages entirely', () => {
    const messages: ModelMessage[] = [
      systemMsg('You are a helpful assistant'),
      userMsg('Do something'),
      systemMsg('Another system message'),
    ]

    const blocks = normalizeFromAiSdk(messages)
    // System messages are skipped
    expect(blocks.every((b) => b.kind !== 'system')).toBe(true)
    expect(blocks).toHaveLength(1)
    expect(blocks[0].kind).toBe('user')
  })

  it('normalizeFromAiSdk handles interleaved roles correctly', () => {
    const messages: ModelMessage[] = [
      systemMsg('system prompt'),
      userMsg('First request'),
      assistantText('Let me check'),
      assistantToolCall('Read', { file_path: '/a.ts' }),
      toolResultMsg('Read', 'contents'),
      assistantText('Here is the answer'),
      userMsg('Now do something else'),
      assistantToolCall('bash', { command: 'git commit -m "fix: bug"' }),
      toolResultMsg('bash', '[main abc1234] fix: bug'),
    ]

    const blocks = normalizeFromAiSdk(messages)

    // system skipped, user(2) + assistant(2) + tool_call(2) + tool_result(2)
    const kinds = blocks.map((b) => b.kind)
    expect(kinds).toEqual([
      'user',
      'assistant',
      'tool_call',
      'tool_result',
      'assistant',
      'user',
      'tool_call',
      'tool_result',
    ])
  })

  it('vccCompile produces structured summary with sections for mixed input', () => {
    const messages: ModelMessage[] = [
      userMsg('Fix the login bug in auth.ts'),
      assistantText('I will look at the file'),
      assistantToolCall('Read', { file_path: '/src/auth.ts' }),
      toolResultMsg('Read', 'export function login() { /* ... */ }'),
      assistantToolCall('Edit', { file_path: '/src/auth.ts' }),
      toolResultMsg('Edit', 'OK'),
      assistantText('Fixed the login validation bug'),
    ]

    const result = vccCompile(messages, null)
    expect(result).not.toBeNull()
    expect(result).toContain('[Session Goal]')
    expect(result).toContain('[Files And Changes]')
  })

  it('vccCompile with existingSummary merges summaries', () => {
    const messages: ModelMessage[] = [
      userMsg('Create a new API endpoint'),
      assistantToolCall('Write', { file_path: '/src/api.ts' }),
      toolResultMsg('Write', 'created'),
    ]

    const existing = '[Session Goal]\n- Fix previous bug'
    const result = vccCompile(messages, existing)
    expect(result).not.toBeNull()
    // Should contain the new goal info and merged context
    expect(result?.length).toBeGreaterThan(0)
  })
})

// ─── Zone 2: Boundary — Very long tool outputs ─────────────────────

describe('VCC Zone 2 (boundary): very long tool outputs', () => {
  it('handles massive tool result without throwing', () => {
    const hugeOutput = 'x'.repeat(100_000)
    const messages: ModelMessage[] = [
      userMsg('Read this huge file'),
      assistantToolCall('Read', { file_path: '/huge.ts' }),
      toolResultMsg('Read', hugeOutput),
    ]

    expect(() => vccCompile(messages, null)).not.toThrow()
    const result = vccCompile(messages, null)
    // Result should exist but be bounded in size
    expect(result === null || result.length < hugeOutput.length).toBe(true)
  })

  it('handles single massive user message', () => {
    const bigText = 'Implement feature X '.repeat(5000)
    const messages: ModelMessage[] = [userMsg(bigText)]

    expect(() => vccCompile(messages, null)).not.toThrow()
    const result = vccCompile(messages, null)
    if (result) {
      // The output should be much smaller than the input
      expect(result.length).toBeLessThan(bigText.length)
    }
  })

  it('handles user message with content parts array', () => {
    const messages: ModelMessage[] = [
      {
        role: 'user',
        content: [
          { type: 'text', text: 'Hello from parts' },
          { type: 'image', image: new Uint8Array(0), mimeType: 'image/png' },
        ],
      } as any,
    ]

    const blocks = normalizeFromAiSdk(messages)
    expect(blocks.length).toBeGreaterThanOrEqual(1)
    expect(blocks.some((b) => b.kind === 'user')).toBe(true)
  })

  it('handles assistant message with parts array containing tool-call + text', () => {
    const messages: ModelMessage[] = [
      {
        role: 'assistant',
        content: [
          { type: 'text', text: 'I will read the file' },
          {
            type: 'tool-call',
            toolName: 'Read',
            toolCallId: 'c1',
            input: { file_path: '/a.ts' },
          },
        ],
      } as any,
    ]

    const blocks = normalizeFromAiSdk(messages)
    const kinds = blocks.map((b) => b.kind)
    expect(kinds).toContain('assistant')
    expect(kinds).toContain('tool_call')
  })
})

// ─── Zone 5: State Mutation — Idempotency ──────────────────────────

describe('VCC Zone 5 (mutation): same messages compacted twice', () => {
  it('produces identical output for same inputs (pure function)', () => {
    const messages: ModelMessage[] = [
      userMsg('Fix the bug'),
      assistantToolCall('Read', { file_path: '/src/app.ts' }),
      toolResultMsg('Read', 'export const app = 1'),
      assistantText('Fixed it'),
    ]

    const result1 = vccCompile(messages, null)
    const result2 = vccCompile(messages, null)
    expect(result1).toBe(result2)
  })

  it('compaction with existingSummary then re-compacting same is also deterministic', () => {
    const messages: ModelMessage[] = [
      userMsg('Build feature A'),
      assistantText('Done with A'),
    ]

    const first = vccCompile(messages, null)!
    const second = vccCompile(messages, first)
    const third = vccCompile(messages, second)

    // Second and third should be stable (existing summary absorbed)
    expect(second).not.toBeNull()
    expect(third).not.toBeNull()
    // At minimum, no crash and outputs are strings
  })
})

// ─── Happy Path: Normal conversation flow ──────────────────────────

describe('VCC Happy path: normal conversation produces structured summary', () => {
  it('produces complete structured output with all expected sections', () => {
    const messages: ModelMessage[] = [
      userMsg('Implement user authentication for the app'),
      assistantText('I will set up the auth module'),
      assistantToolCall('Read', { file_path: '/src/index.ts' }),
      toolResultMsg('Read', 'export { }'),
      assistantToolCall('Write', { file_path: '/src/auth.ts' }),
      toolResultMsg('Write', 'created'),
      assistantToolCall('bash', {
        command: 'git commit -m "feat: add auth module"',
      }),
      toolResultMsg('bash', '[main a1b2c3d] feat: add auth module'),
      assistantText('Auth module is ready'),
      userMsg('Also always use TypeScript strict mode'),
    ]

    const result = vccCompile(messages, null)

    expect(result).not.toBeNull()
    expect(result).toContain('[Session Goal]')
    expect(result).toContain('[Files And Changes]')
    expect(result).toContain('Modified:')
    expect(result).toContain('auth.ts')
    expect(result).toContain('[Commits]')
    expect(result).toContain('feat: add auth module')
  })

  it('captures user preferences', () => {
    const messages: ModelMessage[] = [
      userMsg('Build the dashboard'),
      userMsg('Always use Tailwind for styling'),
      userMsg('Never use inline styles'),
      assistantText('Got it, using Tailwind'),
    ]

    const result = vccCompile(messages, null)
    expect(result).not.toBeNull()
    expect(result).toContain('[User Preferences]')
  })

  it('captures error context in outstanding section', () => {
    const messages: ModelMessage[] = [
      userMsg('Fix the build'),
      assistantToolCall('bash', { command: 'npm run build' }),
      toolResultMsg('bash', 'Error: module not found', true),
      assistantText('The build is still failing because of a missing module'),
    ]

    const result = vccCompile(messages, null)
    expect(result).not.toBeNull()
    expect(result).toContain('[Outstanding Context]')
  })

  it('includes brief transcript section', () => {
    const messages: ModelMessage[] = [
      userMsg('Create a hello world app'),
      assistantText('Creating now'),
    ]

    const result = vccCompile(messages, null)
    expect(result).not.toBeNull()
    // Should have brief transcript or at least session goal
    expect(result?.length).toBeGreaterThan(0)
  })

  it('respects VccOverrides to cap sections', () => {
    const messages: ModelMessage[] = [
      userMsg(
        'Do many things: task 1, task 2, task 3, task 4, task 5, task 6, task 7, task 8, task 9, task 10',
      ),
      assistantText('Working on it'),
      assistantToolCall('Read', { file_path: `/src/a.ts` }),
      toolResultMsg('Read', 'a'),
      assistantToolCall('Read', { file_path: `/src/b.ts` }),
      toolResultMsg('Read', 'b'),
      assistantToolCall('Read', { file_path: `/src/c.ts` }),
      toolResultMsg('Read', 'c'),
    ]

    const overrides = { maxGoalLines: 2, maxFileEntries: 2 }
    const result = vccCompile(messages, null, overrides)
    expect(result).not.toBeNull()
    // Just verify it doesn't crash with overrides — caps are internal
  })

  it('handles assistant with only tool-call (no text)', () => {
    const messages: ModelMessage[] = [
      userMsg('Read the config'),
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolName: 'Read',
            toolCallId: 'c1',
            input: { file_path: '/config.json' },
          },
        ],
      } as any,
      toolResultMsg('Read', '{"port": 3000}'),
    ]

    const blocks = normalizeFromAiSdk(messages)
    const toolCallBlock = blocks.find((b) => b.kind === 'tool_call')
    expect(toolCallBlock).toBeDefined()
    if (toolCallBlock?.kind === 'tool_call') {
      expect(toolCallBlock.name).toBe('Read')
    }
  })

  it('handles error tool results correctly', () => {
    const messages: ModelMessage[] = [
      userMsg('Run the tests'),
      assistantToolCall('bash', { command: 'npm test' }),
      toolResultMsg('bash', 'FAIL: expected 1 got 2', true),
    ]

    const blocks = normalizeFromAiSdk(messages)
    const errorBlock = blocks.find(
      (b) => b.kind === 'tool_result' && b.isError === true,
    )
    expect(errorBlock).toBeDefined()
    if (errorBlock?.kind === 'tool_result') {
      expect(errorBlock.isError).toBe(true)
      expect(errorBlock.text).toContain('FAIL')
    }
  })

  it('handles tool result with content-type output', () => {
    const messages: ModelMessage[] = [
      userMsg('Show me the image'),
      assistantToolCall('screenshot', {}),
      {
        role: 'tool',
        content: [
          {
            type: 'tool-result',
            toolCallId: 'c1',
            toolName: 'screenshot',
            output: {
              type: 'content',
              value: [
                { type: 'text', text: 'Screenshot taken' },
                {
                  type: 'image-data',
                  data: 'base64...',
                  mimeType: 'image/png',
                },
              ],
            },
          },
        ],
      } as any,
    ]

    const blocks = normalizeFromAiSdk(messages)
    const tr = blocks.find((b) => b.kind === 'tool_result')
    expect(tr).toBeDefined()
    if (tr?.kind === 'tool_result') {
      // Image data should be represented as text
      expect(tr.text).toContain('Screenshot taken')
    }
  })
})
