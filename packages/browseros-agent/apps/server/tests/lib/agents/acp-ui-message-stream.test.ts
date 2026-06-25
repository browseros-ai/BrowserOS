/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import {
  createAcpUIMessageStream,
  mapAcpStopReasonToFinishReason,
} from '../../../src/lib/agents/acp/ui-message-stream'
import type { AgentStreamEvent } from '../../../src/lib/agents/types'

describe('createAcpUIMessageStream', () => {
  it('streams ACP output text into AI SDK text chunks', async () => {
    const chunks = await collectChunks([
      { type: 'text_delta', text: 'Hello', stream: 'output' },
      { type: 'text_delta', text: ' world', stream: 'output' },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toEqual([
      { type: 'start' },
      { type: 'text-start', id: 'acp-text' },
      { type: 'text-delta', id: 'acp-text', delta: 'Hello' },
      { type: 'text-delta', id: 'acp-text', delta: ' world' },
      { type: 'text-end', id: 'acp-text' },
      { type: 'finish', finishReason: 'stop' },
    ])
  })

  it('streams ACP thought deltas into AI SDK reasoning chunks', async () => {
    const chunks = await collectChunks([
      { type: 'text_delta', text: 'Thinking', stream: 'thought' },
      { type: 'text_delta', text: ' carefully', stream: 'thought' },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toEqual([
      { type: 'start' },
      { type: 'reasoning-start', id: 'acp-reasoning' },
      { type: 'reasoning-delta', id: 'acp-reasoning', delta: 'Thinking' },
      { type: 'reasoning-delta', id: 'acp-reasoning', delta: ' carefully' },
      { type: 'reasoning-end', id: 'acp-reasoning' },
      { type: 'finish', finishReason: 'stop' },
    ])
  })

  it('closes and reopens text parts when ACP output and thoughts interleave', async () => {
    const chunks = await collectChunks([
      { type: 'text_delta', text: 'Thinking', stream: 'thought' },
      { type: 'text_delta', text: 'Answer', stream: 'output' },
      { type: 'text_delta', text: 'More thought', stream: 'thought' },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toEqual([
      { type: 'start' },
      { type: 'reasoning-start', id: 'acp-reasoning' },
      { type: 'reasoning-delta', id: 'acp-reasoning', delta: 'Thinking' },
      { type: 'reasoning-end', id: 'acp-reasoning' },
      { type: 'text-start', id: 'acp-text' },
      { type: 'text-delta', id: 'acp-text', delta: 'Answer' },
      { type: 'text-end', id: 'acp-text' },
      { type: 'reasoning-start', id: 'acp-reasoning-2' },
      {
        type: 'reasoning-delta',
        id: 'acp-reasoning-2',
        delta: 'More thought',
      },
      { type: 'reasoning-end', id: 'acp-reasoning-2' },
      { type: 'finish', finishReason: 'stop' },
    ])
  })

  it('maps ACP tool calls to AI SDK tool chunks', async () => {
    const chunks = await collectChunks([
      {
        type: 'tool_call',
        id: 'tool-1',
        title: 'Read file',
        text: 'Reading package.json',
        status: 'running',
      },
      {
        type: 'tool_call',
        id: 'tool-1',
        title: 'Read file',
        text: 'Read package.json',
        status: 'completed',
      },
      {
        type: 'tool_call',
        id: 'tool-2',
        title: 'Run tests',
        text: 'Tests failed',
        status: 'failed',
      },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toContainEqual({
      type: 'tool-input-available',
      toolCallId: 'tool-1',
      toolName: 'read_file',
      title: 'Read file',
      input: { description: 'Reading package.json' },
      dynamic: true,
    })
    expect(chunks).toContainEqual({
      type: 'tool-output-available',
      toolCallId: 'tool-1',
      output: { content: 'Read package.json' },
      dynamic: true,
    })
    expect(chunks).toContainEqual({
      type: 'tool-output-error',
      toolCallId: 'tool-2',
      errorText: 'Tests failed',
      dynamic: true,
    })
  })

  it('uses a fallback error text for failed ACP tool calls without text', async () => {
    const chunks = await collectChunks([
      {
        type: 'tool_call',
        id: 'tool-1',
        title: '',
        text: '',
        status: 'failed',
      },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toContainEqual({
      type: 'tool-output-error',
      toolCallId: 'tool-1',
      errorText: 'Tool failed',
      dynamic: true,
    })
  })

  it('emits byok-shaped tool parts for app_connection_request', async () => {
    // The wire shape must match the byok sentinel verbatim so the
    // sidepanel's existing nudge parser and ConnectAppCard handle it
    // without any code change. Output.content[0].text is a JSON
    // string of { type, appName, reason } that getMessageSegments
    // parses to render the card.
    const chunks = await collectChunks([
      {
        type: 'app_connection_request',
        toolCallId: 'nudge-1',
        appName: 'Linear',
        reason: 'to read your Linear issues',
      },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toContainEqual({
      type: 'tool-input-available',
      toolCallId: 'nudge-1',
      toolName: 'suggest_app_connection',
      input: { appName: 'Linear', reason: 'to read your Linear issues' },
      dynamic: true,
    })
    expect(chunks).toContainEqual({
      type: 'tool-output-available',
      toolCallId: 'nudge-1',
      output: {
        content: [
          {
            type: 'text',
            text: JSON.stringify({
              type: 'app_connection',
              appName: 'Linear',
              reason: 'to read your Linear issues',
            }),
          },
        ],
        isError: false,
      },
      dynamic: true,
    })
  })

  it('suppresses upstream tool_call rendering for nudge tools', async () => {
    // Without suppression the renderer would briefly show a generic
    // tool block ("Tool: nudge/suggest_app_connection") next to the
    // real card. Mirrors agent-company's StreamTranslator drop.
    const chunks = await collectChunks([
      {
        type: 'tool_call',
        id: 'tool-x',
        title: 'nudge/suggest_app_connection',
        text: 'Calling tool',
        status: 'completed',
      },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(
      chunks.some(
        (c) =>
          c.type === 'tool-input-available' &&
          c.toolName === 'nudge/suggest_app_connection',
      ),
    ).toBe(false)
    expect(
      chunks.some(
        (c) => c.type === 'tool-output-available' && c.toolCallId === 'tool-x',
      ),
    ).toBe(false)
  })

  it('keeps ACP status events transient', async () => {
    const chunks = await collectChunks([
      { type: 'status', text: 'Using adapter default' },
      { type: 'done', stopReason: 'end_turn' },
    ])

    expect(chunks).toContainEqual({
      type: 'data-acp-status',
      id: 'acp-status',
      data: { text: 'Using adapter default' },
      transient: true,
    })
  })

  it('emits an AI SDK error chunk and error finish for ACP errors', async () => {
    const chunks = await collectChunks([
      { type: 'text_delta', text: 'Partial', stream: 'output' },
      { type: 'error', message: 'ACP failed', code: 'boom' },
    ])

    expect(chunks).toEqual([
      { type: 'start' },
      { type: 'text-start', id: 'acp-text' },
      { type: 'text-delta', id: 'acp-text', delta: 'Partial' },
      { type: 'text-end', id: 'acp-text' },
      { type: 'error', errorText: 'ACP failed' },
      { type: 'finish', finishReason: 'error' },
    ])
  })

  it('cancels the ACP event reader when the UI stream is cancelled', async () => {
    let cancelled = false
    const acpEvents = new ReadableStream<AgentStreamEvent>({
      cancel() {
        cancelled = true
      },
    })
    const reader = createAcpUIMessageStream(acpEvents).getReader()

    await reader.read()
    await reader.cancel('sidepanel stopped')

    expect(cancelled).toBe(true)
  })
})

describe('mapAcpStopReasonToFinishReason', () => {
  it('maps known ACP stop reasons to AI SDK finish reasons', () => {
    expect(mapAcpStopReasonToFinishReason('end_turn')).toBe('stop')
    expect(mapAcpStopReasonToFinishReason('cancelled')).toBe('other')
    expect(mapAcpStopReasonToFinishReason(undefined)).toBe('stop')
  })
})

async function collectChunks(
  events: AgentStreamEvent[],
): Promise<UIMessageChunk[]> {
  const stream = createAcpUIMessageStream(
    new ReadableStream<AgentStreamEvent>({
      start(controller) {
        for (const event of events) controller.enqueue(event)
        controller.close()
      },
    }),
  )
  const chunks: UIMessageChunk[] = []
  for await (const chunk of stream) chunks.push(chunk)
  return chunks
}
