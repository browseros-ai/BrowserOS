/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it, mock } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import { ChatService } from '../../../src/api/services/chat-service'
import {
  AcpAgentPreparationError,
  AcpAgentSessionBusyError,
  type AcpAgentStreamInput,
} from '../../../src/lib/agents/acp/acp-agent-runtime'
import type { AcpAgentDefinition } from '../../../src/lib/agents/agent-types'

const AGENT_ID = '4a815af8-7555-4d65-b789-3be98f567a2d'

function acpAgent(type: AcpAgentDefinition['type'] = 'claude') {
  return {
    id: AGENT_ID,
    name: type === 'claude' ? 'Claude Code' : 'Codex',
    type,
    workingDirectory: '/agent/default',
    createdAt: 1,
    updatedAt: 1,
  } satisfies AcpAgentDefinition
}

function deps(
  options: {
    agent?: AcpAgentDefinition | null
    streamError?: Error
    firstStreamError?: Error
  } = {},
) {
  const calls: AcpAgentStreamInput[] = []
  const close = mock(async () => true)
  let streamAttempt = 0
  const acpRuntime = {
    async stream(input: AcpAgentStreamInput) {
      const error =
        streamAttempt === 0 && options.firstStreamError
          ? options.firstStreamError
          : options.streamError
      streamAttempt += 1
      if (error) throw error
      calls.push(input)
      await input.onFinish?.({
        messages: [
          ...input.messages,
          {
            id: 'assistant-1',
            role: 'assistant',
            parts: [{ type: 'text', text: 'done' }],
          },
        ],
        isAborted: false,
      })
      return chunks([
        { type: 'text-start', id: 'text-1' },
        { type: 'text-delta', id: 'text-1', delta: 'done' },
        { type: 'text-end', id: 'text-1' },
        { type: 'finish', finishReason: 'stop' },
      ])
    },
    close,
  }
  const conversationStore = {
    get: mock(async () => null),
    save: mock(async () => ({
      id: '',
      targetType: 'claude' as const,
      lastMessagedAt: 1,
      createdAt: 1,
      updatedAt: 1,
    })),
  }
  const service = new ChatService({
    sessionStore: {
      get: () => undefined,
      set: () => {},
      remove: () => false,
      delete: async () => false,
      count: () => 0,
    } as never,
    browser: {
      resolveTabIds: mock(async () => new Map<number, number>()),
    } as never,
    browserSession: {} as never,
    serverPort: 9100,
    acpAgentStore: {
      get: mock(async () =>
        options.agent === undefined ? acpAgent() : options.agent,
      ),
    },
    acpRuntime: acpRuntime as never,
    conversationStore: conversationStore as never,
  })
  return { calls, close, service, conversationStore }
}

describe('ChatService ACP dispatch', () => {
  it('streams an ACP target without resolving an LLM provider', async () => {
    const fixture = deps()
    const response = await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId: crypto.randomUUID(),
        message: 'inspect the page',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
        userWorkingDir: '/request/workspace',
      },
      new AbortController().signal,
    )

    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('text/event-stream')
    expect(await response.text()).toContain('"delta":"done"')
    expect(fixture.calls).toHaveLength(1)
    expect(fixture.calls[0]?.agent).toMatchObject({
      id: AGENT_ID,
      type: 'claude',
      workingDirectory: '/request/workspace',
    })
    const userText = fixture.calls[0]?.messages
      .at(-1)
      ?.parts.find((part) => part.type === 'text')
    expect(userText).toMatchObject({
      type: 'text',
      text: '<USER_QUERY>\ninspect the page\n</USER_QUERY>',
    })
  })

  it('persists an ACP display copy on finish', async () => {
    const fixture = deps()
    const conversationId = crypto.randomUUID()
    await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId,
        message: 'inspect the page',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      new AbortController().signal,
    )

    expect(fixture.conversationStore.save).toHaveBeenCalledTimes(1)
    const saved = fixture.conversationStore.save.mock.calls.at(-1)?.[0] as
      | {
          id: string
          targetType: string
          agentId?: string
          messages: unknown[]
        }
      | undefined
    expect(saved?.id).toBe(conversationId)
    expect(saved?.targetType).toBe('claude')
    expect(saved?.agentId).toBe(AGENT_ID)
    expect(
      (saved?.messages as Array<{ role: string }> | undefined)?.some(
        (message) => message.role === 'assistant',
      ),
    ).toBe(true)
  })

  it('ignores a stored display copy that belongs to a different agent', async () => {
    const fixture = deps()
    fixture.conversationStore.get.mockImplementation(async () => ({
      id: 'shared',
      messages: [
        {
          id: 'foreign',
          role: 'user',
          parts: [{ type: 'text', text: 'other agent secret' }],
        },
      ],
      targetType: 'claude',
      agentId: 'a-different-agent',
      lastMessagedAt: 1,
      createdAt: 1,
      updatedAt: 1,
    }))

    await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId: crypto.randomUUID(),
        message: 'my message',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      new AbortController().signal,
    )

    // The foreign history must not reach the runtime; only the current turn.
    const sent = fixture.calls[0]?.messages ?? []
    expect(sent).toHaveLength(1)
    expect(JSON.stringify(sent)).not.toContain('other agent secret')
  })

  it('rejects a target whose stored agent has a different adapter type', async () => {
    const fixture = deps({ agent: acpAgent('codex') })
    const response = await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId: crypto.randomUUID(),
        message: 'hello',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      new AbortController().signal,
    )

    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({ error: 'Agent type mismatch' })
    expect(fixture.calls).toHaveLength(0)
  })

  it('discards the ACP session when its conversation is deleted', async () => {
    const fixture = deps()
    const conversationId = crypto.randomUUID()
    await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId,
        message: 'hello',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      new AbortController().signal,
    )

    expect(fixture.service.isAcpSession(conversationId)).toBe(true)
    expect(await fixture.service.deleteSession(conversationId)).toEqual({
      deleted: true,
      sessionCount: 0,
    })
    expect(fixture.close).toHaveBeenCalledWith(AGENT_ID, conversationId, {
      discardPersistentState: true,
    })
    expect(fixture.service.isAcpSession(conversationId)).toBe(false)
  })

  it('returns conflict when the ACP conversation is already running', async () => {
    const fixture = deps({ streamError: new AcpAgentSessionBusyError() })
    const response = await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId: crypto.randomUUID(),
        message: 'hello',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      new AbortController().signal,
    )

    expect(response.status).toBe(409)
    expect(await response.json()).toEqual({
      error: 'An agent turn is already running',
    })
  })

  it('rolls back a turn when ACP preparation fails', async () => {
    const fixture = deps({
      firstStreamError: new AcpAgentPreparationError(),
    })
    const conversationId = crypto.randomUUID()
    const abortSignal = new AbortController().signal

    const failedResponse = await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId,
        message: 'failed turn',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      abortSignal,
    )
    expect(await failedResponse.text()).toContain(
      'Unable to start the ACP agent.',
    )

    const retryResponse = await fixture.service.processMessage(
      {
        target: { type: 'claude', agentId: AGENT_ID },
        conversationId,
        message: 'retry turn',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
      },
      abortSignal,
    )
    expect(await retryResponse.text()).toContain('"delta":"done"')
    expect(fixture.calls).toHaveLength(1)
    expect(fixture.calls[0]?.messages).toHaveLength(1)
    expect(fixture.calls[0]?.messages[0]?.parts).toContainEqual({
      type: 'text',
      text: '<USER_QUERY>\nretry turn\n</USER_QUERY>',
    })
  })
})

function chunks(parts: UIMessageChunk[]): ReadableStream<UIMessageChunk> {
  return new ReadableStream({
    start(controller) {
      for (const part of parts) controller.enqueue(part)
      controller.close()
    },
  })
}
