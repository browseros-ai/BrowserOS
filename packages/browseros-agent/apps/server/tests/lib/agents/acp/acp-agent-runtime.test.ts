/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  type AcpxProviderSettings,
  createAcpxProvider,
} from '@browseros/acpx-ai-provider'
import type {
  AcpRuntime,
  AcpRuntimeEnsureInput,
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeTurn,
  AcpRuntimeTurnInput,
  AcpRuntimeTurnResult,
  AcpSessionRecord,
} from 'acpx/runtime'
import { createFileSessionStore } from 'acpx/runtime'
import type { UIMessage, UIMessageChunk } from 'ai'
import { ChatService } from '../../../../src/api/services/chat-service'
import {
  AcpAgentPreparationError,
  AcpAgentRuntime,
  AcpAgentSessionBusyError,
} from '../../../../src/lib/agents/acp/acp-agent-runtime'
import { BROWSEROS_ACP_INSTRUCTIONS } from '../../../../src/lib/agents/acp/browseros-instructions'
import type { AcpAgentDefinition } from '../../../../src/lib/agents/agent-types'
import { DbConversationStore } from '../../../../src/lib/conversations/conversation-store'
import { openBrowserOsDatabase } from '../../../../src/lib/db/client'

const temporaryDirectories: string[] = []
const BROWSER_TOOL_LEASE_TOKEN = 'runtime-test-lease'

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((path) => rm(path, { recursive: true, force: true })),
  )
})

async function runtimeFixture(options: {
  adapter?: AcpAgentDefinition['type']
  runtime?: RecordingAcpRuntime
  agent?: Partial<AcpAgentDefinition>
  idleTimeoutMs?: number
}) {
  const root = await mkdtemp(join(tmpdir(), 'acp-agent-runtime-'))
  temporaryDirectories.push(root)
  const resourcesDir = join(root, 'resources')

  const adapter = options.adapter ?? 'claude'
  const agent: AcpAgentDefinition = {
    id: `${adapter}-agent-id`,
    name: adapter === 'claude' ? 'Claude Code' : 'Codex',
    type: adapter,
    createdAt: 1,
    updatedAt: 1,
    ...options.agent,
  }
  const acpRuntime = options.runtime ?? new RecordingAcpRuntime()
  const providerSettings: AcpxProviderSettings[] = []
  const stateDir = join(root, 'state')
  const runtime = new AcpAgentRuntime({
    serverPort: 9100,
    browserosDir: root,
    resourcesDir,
    stateDir,
    idleTimeoutMs: options.idleTimeoutMs,
    createProvider(settings) {
      providerSettings.push(settings)
      return createAcpxProvider({ ...settings, runtime: acpRuntime })
    },
  })

  return { acpRuntime, agent, providerSettings, runtime, stateDir }
}

function textMessage(
  id: string,
  role: UIMessage['role'],
  text: string,
): UIMessage {
  return { id, role, parts: [{ type: 'text', text }] }
}

async function collect(
  stream: ReadableStream<UIMessageChunk>,
): Promise<UIMessageChunk[]> {
  const parts: UIMessageChunk[] = []
  for await (const part of stream) parts.push(part)
  return parts
}

describe('AcpAgentRuntime', () => {
  it('streams Claude directly through the ACP provider with BrowserOS policy', async () => {
    const acpRuntime = new RecordingAcpRuntime({
      turns: [[{ type: 'text_delta', text: 'hello', stream: 'output' }]],
    })
    const fixture = await runtimeFixture({
      runtime: acpRuntime,
      agent: { modelId: 'claude-opus-4-1', reasoningEffort: 'high' },
    })
    const abortController = new AbortController()

    const parts = await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-1',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'say hello')],
        browserContext: { windowId: 7 },
        abortSignal: abortController.signal,
      }),
    )

    expect(parts).toContainEqual({
      type: 'text-delta',
      id: expect.any(String),
      delta: 'hello',
    })
    expect(fixture.providerSettings).toHaveLength(1)
    expect(fixture.providerSettings[0]).toMatchObject({
      agent: 'claude',
      cwd: expect.any(String),
      sessionKey: 'acp:claude-agent-id:conversation-1',
      sessionMode: 'persistent',
      permissionMode: 'approve-all',
      nonInteractivePermissions: 'deny',
      sessionOptions: {
        model: 'claude-opus-4-1',
        systemPrompt: { append: BROWSEROS_ACP_INSTRUCTIONS },
      },
      mcpServers: [
        {
          type: 'http',
          name: 'browseros',
          url: 'http://127.0.0.1:9100/mcp',
          headers: {
            'X-BrowserOS-Internal-Lease': BROWSER_TOOL_LEASE_TOKEN,
          },
        },
      ],
    })
    expect(acpRuntime.setModeCalls).toEqual(['bypassPermissions'])
    expect(acpRuntime.setConfigOptionCalls).toEqual([
      { key: 'effort', value: 'high' },
    ])
    expect(acpRuntime.startTurnCalls[0]?.signal).toBe(abortController.signal)
  })

  it('sends complete initial history and only the new turn on continuation', async () => {
    const acpRuntime = new RecordingAcpRuntime({
      turns: [
        [{ type: 'text_delta', text: 'first answer', stream: 'output' }],
        [{ type: 'text_delta', text: 'second answer', stream: 'output' }],
      ],
    })
    const fixture = await runtimeFixture({ runtime: acpRuntime })
    const initialMessages: UIMessage[] = [
      textMessage('user-1', 'user', 'seed question'),
      textMessage('assistant-1', 'assistant', 'seed answer'),
      {
        id: 'user-2',
        role: 'user',
        parts: [
          { type: 'text', text: 'inspect this image' },
          {
            type: 'file',
            mediaType: 'image/png',
            url: 'data:image/png;base64,Zm9v',
          },
        ],
      },
    ]

    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-2',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: initialMessages,
      }),
    )
    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-2',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [
          ...initialMessages,
          textMessage('assistant-2', 'assistant', 'first answer'),
          textMessage('user-3', 'user', 'follow up only'),
        ],
      }),
    )

    expect(acpRuntime.startTurnCalls).toHaveLength(2)
    expect(acpRuntime.startTurnCalls[0]?.text).toContain('seed question')
    expect(acpRuntime.startTurnCalls[0]?.text).toContain('seed answer')
    expect(acpRuntime.startTurnCalls[0]?.text).toContain('inspect this image')
    expect(acpRuntime.startTurnCalls[0]?.attachments).toEqual([
      { mediaType: 'image/png', data: 'Zm9v' },
    ])
    expect(acpRuntime.startTurnCalls[1]?.text).toBe('User: follow up only')
    expect(acpRuntime.startTurnCalls[1]?.attachments).toBeUndefined()
    expect(fixture.providerSettings).toHaveLength(1)
  })

  it('restores history when a replacement session emits only a startup notice', async () => {
    const fixture = await runtimeFixture({})
    const sessionKey = 'acp:claude-agent-id:legacy-conversation'
    const store = createFileSessionStore({ stateDir: fixture.stateDir })
    const timestamp = new Date(0).toISOString()
    const legacyRecord: AcpSessionRecord = {
      schema: 'acpx.session.v1',
      acpxRecordId: sessionKey,
      acpSessionId: 'legacy-session',
      agentCommand: 'npx -y @agentclientprotocol/claude-agent-acp@^0.31.0',
      cwd: process.cwd(),
      createdAt: timestamp,
      lastUsedAt: timestamp,
      lastSeq: 1,
      eventLog: {
        active_path: 'events.jsonl',
        segment_count: 0,
        max_segment_bytes: 1024,
        max_segments: 1,
      },
      messages: [
        { User: { id: 'legacy-user', content: [{ Text: 'old prompt' }] } },
      ],
      updated_at: timestamp,
      cumulative_token_usage: {},
      request_token_usage: {},
    }
    await store.save(legacyRecord)
    fixture.acpRuntime.ensureSessionHook = async () => {
      await store.save({
        ...legacyRecord,
        acpSessionId: 'fresh-session',
        agentCommand: 'env PATH=/bin claude-agent-acp',
        agentArgv: ['env', 'PATH=/bin', 'claude-agent-acp'],
        messages: [
          {
            Agent: {
              content: [
                { Text: 'Auto mode unavailable; using Accept edits instead.' },
              ],
              tool_results: {},
            },
          },
        ],
      })
    }

    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'legacy-conversation',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [
          textMessage('user-old', 'user', 'previous UI prompt'),
          textMessage('assistant-old', 'assistant', 'previous UI answer'),
          textMessage('user-new', 'user', 'new prompt'),
        ],
      }),
    )

    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain(
      'previous UI prompt',
    )
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain(
      'previous UI answer',
    )
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain('new prompt')
  })

  it('assigns a distinct nonempty ID to every streamed assistant reply', async () => {
    const fixture = await runtimeFixture({
      idleTimeoutMs: 0,
      runtime: new RecordingAcpRuntime({
        turns: [
          [{ type: 'text_delta', text: 'first answer', stream: 'output' }],
          [{ type: 'text_delta', text: 'second answer', stream: 'output' }],
        ],
      }),
    })
    const history: UIMessage[] = []
    for (const turn of [1, 2]) {
      history.push(textMessage(`user-${turn}`, 'user', `question ${turn}`))
      await collect(
        await fixture.runtime.stream({
          agent: fixture.agent,
          conversationId: 'unique-replies',
          browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
          readOnly: false,
          messages: [...history],
          onFinish: ({ messages }) => {
            const reply = messages.at(-1)
            if (!reply) throw new Error('Missing assistant reply')
            expect(reply.role).toBe('assistant')
            expect(reply.id).not.toBe('')
            expect(history.some((message) => message.id === reply.id)).toBe(
              false,
            )
            history.push(reply)
          },
        }),
      )
    }
    expect(
      history.filter((message) => message.role === 'assistant'),
    ).toHaveLength(2)
    await fixture.runtime.closeAllForAgent(fixture.agent.id)
  })

  it('sends only the latest turn when a persisted agent has real user history', async () => {
    const fixture = await runtimeFixture({})
    await saveAgentRecord(fixture, 'resumed', [
      { User: { id: 'old', content: [{ Text: 'old question' }] } },
    ])
    await collect(
      await fixture.runtime.stream(historyInput(fixture, 'resumed')),
    )
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toBe(
      'User: new question',
    )
  })

  it('starts fresh when restoring the old agent fails during setup', async () => {
    const fixture = await runtimeFixture({})
    const store = await saveAgentRecord(fixture, 'setup-failure', [
      { User: { id: 'old', content: [{ Text: 'old question' }] } },
    ])
    fixture.acpRuntime.setModeHook = async () => {
      if (fixture.acpRuntime.ensureSessionCalls.length === 1)
        throw resumeError()
    }
    fixture.acpRuntime.ensureSessionHook = async (input) => {
      if (fixture.acpRuntime.ensureSessionCalls.length === 2) {
        const record = await store.load(input.sessionKey)
        if (!record) throw new Error('Missing saved agent session')
        expect(record.acpx?.reset_on_next_ensure).toBe(true)
        await store.save({ ...record, messages: [], acpx: {} })
      }
    }
    await collect(
      await fixture.runtime.stream(historyInput(fixture, 'setup-failure')),
    )
    expect(fixture.acpRuntime.ensureSessionCalls).toHaveLength(2)
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(1)
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain('old answer')
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain('new question')
  })

  it('retries a resume failure before output and persists only the successful reply', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({
        turns: [
          [],
          [{ type: 'text_delta', text: 'recovered answer', stream: 'output' }],
        ],
        results: [resumeFailure()],
      }),
    })
    await saveAgentRecord(fixture, 'stream-failure', [
      { User: { id: 'old', content: [{ Text: 'old question' }] } },
    ])
    const finished: UIMessage[][] = []
    const parts = await collect(
      await fixture.runtime.stream({
        ...historyInput(fixture, 'stream-failure'),
        onFinish: ({ messages }) => {
          finished.push(messages)
        },
      }),
    )
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(2)
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toBe(
      'User: new question',
    )
    expect(fixture.acpRuntime.startTurnCalls[1]?.text).toContain('old answer')
    expect(parts.filter((part) => part.type === 'error')).toEqual([])
    expect(parts.filter((part) => part.type === 'start')).toHaveLength(1)
    expect(parts.filter((part) => part.type === 'finish')).toHaveLength(1)
    expect(finished).toHaveLength(1)
    expect(finished[0]?.at(-1)?.parts).toContainEqual({
      type: 'text',
      text: 'recovered answer',
      state: 'done',
    })
  })

  it('stops after one fresh-session retry', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({
        results: [resumeFailure(), resumeFailure()],
      }),
    })
    const parts = await collect(
      await fixture.runtime.stream(historyInput(fixture, 'twice-failed')),
    )
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(2)
    expect(parts.filter((part) => part.type === 'error')).toHaveLength(1)
  })

  for (const activity of [
    { type: 'text_delta', text: 'already started', stream: 'output' },
    {
      type: 'tool_call',
      toolCallId: 'tool-1',
      title: 'Click button',
      status: 'in_progress',
      rawInput: { button: 'Submit' },
    },
  ] satisfies AcpRuntimeEvent[]) {
    it(`does not retry after ${activity.type} activity`, async () => {
      const fixture = await runtimeFixture({
        runtime: new RecordingAcpRuntime({
          turns: [[activity]],
          results: [resumeFailure()],
        }),
      })
      const parts = await collect(
        await fixture.runtime.stream(historyInput(fixture, 'already-started')),
      )
      expect(fixture.acpRuntime.startTurnCalls).toHaveLength(1)
      expect(fixture.acpRuntime.closeCalls).toHaveLength(0)
      expect(parts.filter((part) => part.type === 'error')).toHaveLength(1)
    })
  }

  it('honors cancellation while recovering a stale session', async () => {
    const controller = new AbortController()
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({ results: [resumeFailure()] }),
    })
    fixture.acpRuntime.closeHook = async () => controller.abort()
    await collect(
      await fixture.runtime.stream({
        ...historyInput(fixture, 'cancel-recovery'),
        abortSignal: controller.signal,
      }),
    )
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(1)
    expect(fixture.acpRuntime.ensureSessionCalls).toHaveLength(1)
  })

  it('keeps the conversation locked while replacing a stale agent', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({ results: [resumeFailure()] }),
    })
    const closing = Promise.withResolvers<void>()
    const release = Promise.withResolvers<void>()
    fixture.acpRuntime.closeHook = async () => {
      closing.resolve()
      await release.promise
    }
    const input = historyInput(fixture, 'locked-recovery')
    const first = await fixture.runtime.stream(input)
    await closing.promise
    try {
      await expect(fixture.runtime.stream(input)).rejects.toBeInstanceOf(
        AcpAgentSessionBusyError,
      )
    } finally {
      release.resolve()
    }
    await collect(first)
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(2)
  })

  it('restores saved text while omitting an interrupted tool call', async () => {
    const fixture = await runtimeFixture({})
    const input = historyInput(fixture, 'interrupted-tool')
    input.messages[1].parts.push({
      type: 'dynamic-tool',
      toolName: 'browser_click',
      toolCallId: 'partial',
      state: 'input-streaming',
      input: undefined,
    })
    const parts = await collect(await fixture.runtime.stream(input))
    expect(parts.filter((part) => part.type === 'error')).toEqual([])
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain('old answer')
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).not.toContain(
      'browser_click',
    )
  })

  it('continues with the new request when legacy display messages cannot be converted', async () => {
    const fixture = await runtimeFixture({})
    const input = historyInput(fixture, 'legacy-message')
    input.messages[1] = {
      id: 'legacy',
      role: 'assistant',
      content: 'old schema',
    } as unknown as UIMessage
    await collect(await fixture.runtime.stream(input))
    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toBe(
      'User: new question',
    )
  })

  it('persists every reply in SQLite across newtab, sidepanel, and a cold restart', async () => {
    const fixture = await runtimeFixture({
      idleTimeoutMs: 0,
      runtime: new RecordingAcpRuntime({
        turns: ['opened HN', 'we highlighted comments', 'I remember'].map(
          (text) => [{ type: 'text_delta', text, stream: 'output' }],
        ),
      }),
    })
    const handle = openBrowserOsDatabase({
      dbPath: join(fixture.stateDir, 'history.sqlite'),
    })
    const store = new DbConversationStore({ db: handle.db })
    const conversationId = 'persisted-history'
    const initial = [
      textMessage('original-user', 'user', 'Highlight insightful comments'),
      textMessage('', 'assistant', 'I highlighted 10 comments'),
    ]
    try {
      await store.save({
        id: conversationId,
        targetType: 'claude',
        agentId: fixture.agent.id,
        messages: initial,
      })
      for (const [index, message] of [
        'open hn',
        'what did we do before?',
        'remember this',
      ].entries()) {
        if (index === 2)
          await fixture.runtime.closeAllForAgent(fixture.agent.id)
        // A new service instance must reload the transcript from SQLite.
        const service = new ChatService({
          sessionStore: {
            get: () => undefined,
            set: () => {},
            remove: () => false,
            delete: async () => false,
            count: () => 0,
          } as never,
          browser: { resolveTabIds: async () => new Map() } as never,
          browserMcp: {
            createLease: () => ({
              token: BROWSER_TOOL_LEASE_TOKEN,
              updateBrowserContext: () => {},
              revoke: () => {},
            }),
          } as never,
          serverPort: 9100,
          acpAgentStore: { get: async () => fixture.agent },
          acpRuntime: fixture.runtime,
          conversationStore: store,
        })
        const response = await service.processMessage(
          {
            target: { type: 'claude', agentId: fixture.agent.id },
            conversationId,
            message,
            isScheduledTask: false,
            mode: 'agent',
            origin: index === 0 ? 'newtab' : 'sidepanel',
          },
          new AbortController().signal,
        )
        expect(await response.text()).not.toContain('"type":"error"')
      }
      const saved = await store.get(conversationId)
      if (!saved) throw new Error('Missing saved conversation')
      expect(saved.messages).toHaveLength(8)
      expect(saved.messages.slice(0, 2)).toEqual(initial)
      const replies = saved.messages
        .filter((message) => message.role === 'assistant')
        .slice(1)
      expect(replies).toHaveLength(3)
      expect(replies.every((message) => message.id.length > 0)).toBe(true)
      expect(new Set(replies.map((message) => message.id)).size).toBe(3)
      expect(fixture.acpRuntime.startTurnCalls[2]?.text).toContain(
        'I highlighted 10 comments',
      )
      expect(fixture.acpRuntime.startTurnCalls[2]?.text).toContain(
        'we highlighted comments',
      )
      expect(saved.lastUserMessage).toBe('remember this')
    } finally {
      await fixture.runtime.closeAllForAgent(fixture.agent.id)
      handle.sqlite.close()
    }
  })

  it('uses Codex config and falls back across full-access mode ids', async () => {
    const acpRuntime = new RecordingAcpRuntime({
      rejectedModes: ['agent-full-access'],
    })
    const fixture = await runtimeFixture({
      adapter: 'codex',
      runtime: acpRuntime,
      agent: { modelId: 'gpt-5.4', reasoningEffort: 'xhigh' },
    })

    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-3',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'work')],
      }),
    )

    expect(acpRuntime.setModeCalls).toEqual([
      'agent-full-access',
      'full-access',
    ])
    expect(acpRuntime.setConfigOptionCalls).toEqual([
      { key: 'reasoning_effort', value: 'xhigh' },
    ])
    expect(fixture.providerSettings[0]?.sessionOptions).toEqual({})
    const codexOverride =
      fixture.providerSettings[0]?.agentRegistryOverrides?.codex ?? ''
    const codexCommand = Array.isArray(codexOverride)
      ? codexOverride.join('\n')
      : codexOverride
    expect(codexCommand).toContain('CODEX_CONFIG=')
    expect(codexCommand).toContain('"model":"gpt-5.4"')
    expect(codexCommand).toContain('"model_reasoning_effort":"xhigh"')
  })

  it('inlines text files before the turn reaches ACP', async () => {
    const fixture = await runtimeFixture({})

    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-text-file',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [
          {
            id: 'user-1',
            role: 'user',
            parts: [
              { type: 'text', text: 'inspect this' },
              {
                type: 'file',
                filename: 'notes.txt',
                mediaType: 'text/plain',
                url: 'data:text/plain;base64,aGVsbG8=',
              },
            ],
          },
        ],
      }),
    )

    expect(fixture.acpRuntime.startTurnCalls[0]?.text).toContain(
      '[File: notes.txt]\nhello',
    )
    expect(fixture.acpRuntime.startTurnCalls[0]?.attachments).toBeUndefined()
  })

  it('rejects with a preparation error and retains no session when preparation fails', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({
        ensureError: new Error('native adapter is unavailable'),
      }),
    })

    await expect(
      fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-4',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    ).rejects.toBeInstanceOf(AcpAgentPreparationError)
    expect(
      await fixture.runtime.close(fixture.agent.id, 'conversation-4'),
    ).toBe(false)
  })

  it('surfaces the ACP turn failure message', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({
        results: [
          {
            status: 'failed',
            error: {
              message:
                'Internal error: Usage credits are required for long context requests.',
              code: 'usage_credits_required',
            },
          },
        ],
      }),
    })

    const parts = await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-turn-failure',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    )

    expect(parts.filter((part) => part.type === 'error')).toEqual([
      {
        type: 'error',
        errorText:
          'Internal error: Usage credits are required for long context requests.',
      },
    ])
  })

  it('closes only the selected persistent ACP session', async () => {
    const fixture = await runtimeFixture({})
    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-5',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    )

    expect(
      await fixture.runtime.close(fixture.agent.id, 'conversation-5', {
        discardPersistentState: true,
      }),
    ).toBe(true)
    expect(fixture.acpRuntime.closeCalls).toEqual([
      { reason: 'close', discardPersistentState: true },
    ])
    expect(
      await fixture.runtime.close(fixture.agent.id, 'conversation-5'),
    ).toBe(false)
  })

  it('rejects overlapping turns until the active stream ends', async () => {
    const fixture = await runtimeFixture({})
    const input = {
      agent: fixture.agent,
      conversationId: 'conversation-6',
      browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
      readOnly: false,
      messages: [textMessage('user-1', 'user', 'hello')],
    }
    const firstStream = await fixture.runtime.stream(input)

    await expect(fixture.runtime.stream(input)).rejects.toBeInstanceOf(
      AcpAgentSessionBusyError,
    )

    await collect(firstStream)
    await collect(
      await fixture.runtime.stream({
        ...input,
        messages: [
          ...input.messages,
          textMessage('user-2', 'user', 'try again'),
        ],
      }),
    )
    expect(fixture.acpRuntime.startTurnCalls).toHaveLength(2)
  })

  it('closes every loaded session for a deleted agent', async () => {
    const fixture = await runtimeFixture({})
    for (const conversationId of ['conversation-7', 'conversation-8']) {
      await collect(
        await fixture.runtime.stream({
          agent: fixture.agent,
          conversationId,
          browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
          readOnly: false,
          messages: [textMessage(`user-${conversationId}`, 'user', 'hello')],
        }),
      )
    }

    expect(
      await fixture.runtime.closeAllForAgent(fixture.agent.id, {
        discardPersistentState: true,
      }),
    ).toBe(2)
    expect(fixture.acpRuntime.closeCalls).toEqual([
      { reason: 'agent-delete', discardPersistentState: true },
      { reason: 'agent-delete', discardPersistentState: true },
    ])
  })

  it('closes idle providers without discarding resumable state', async () => {
    const fixture = await runtimeFixture({ idleTimeoutMs: 5 })
    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-9',
        browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
        readOnly: false,
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    )

    await new Promise((resolve) => setTimeout(resolve, 20))

    expect(fixture.acpRuntime.closeCalls).toEqual([
      { reason: 'idle', discardPersistentState: false },
    ])
    expect(
      await fixture.runtime.close(fixture.agent.id, 'conversation-9'),
    ).toBe(false)
  })
})

function historyInput(
  fixture: Awaited<ReturnType<typeof runtimeFixture>>,
  conversationId: string,
) {
  return {
    agent: fixture.agent,
    conversationId,
    browserToolLeaseToken: BROWSER_TOOL_LEASE_TOKEN,
    readOnly: false,
    messages: [
      textMessage('old-user', 'user', 'old question'),
      textMessage('old-assistant', 'assistant', 'old answer'),
      textMessage('new-user', 'user', 'new question'),
    ],
  }
}

function resumeError() {
  return Object.assign(
    new Error('Persistent ACP session could not be resumed'),
    { detailCode: 'SESSION_RESUME_REQUIRED' },
  )
}

function resumeFailure(): AcpRuntimeTurnResult {
  return {
    status: 'failed',
    error: {
      code: 'RUNTIME_ERROR',
      detailCode: 'SESSION_RESUME_REQUIRED',
      message: 'Persistent ACP session could not be resumed',
    },
  }
}

async function saveAgentRecord(
  fixture: Awaited<ReturnType<typeof runtimeFixture>>,
  conversationId: string,
  messages: AcpSessionRecord['messages'],
) {
  const timestamp = new Date(0).toISOString()
  const store = createFileSessionStore({ stateDir: fixture.stateDir })
  await store.save({
    schema: 'acpx.session.v1',
    acpxRecordId: `acp:${fixture.agent.id}:${conversationId}`,
    acpSessionId: 'saved-session',
    agentCommand: 'claude-agent-acp',
    cwd: fixture.stateDir,
    createdAt: timestamp,
    lastUsedAt: timestamp,
    lastSeq: 0,
    eventLog: {
      active_path: 'events.jsonl',
      segment_count: 0,
      max_segment_bytes: 1024,
      max_segments: 1,
    },
    messages,
    updated_at: timestamp,
    cumulative_token_usage: {},
    request_token_usage: {},
  })
  return store
}

interface RecordingAcpRuntimeOptions {
  turns?: AcpRuntimeEvent[][]
  results?: AcpRuntimeTurnResult[]
  ensureError?: Error
  rejectedModes?: string[]
}

class RecordingAcpRuntime implements AcpRuntime {
  readonly ensureSessionCalls: AcpRuntimeEnsureInput[] = []
  readonly startTurnCalls: AcpRuntimeTurnInput[] = []
  readonly setModeCalls: string[] = []
  readonly setConfigOptionCalls: Array<{ key: string; value: string }> = []
  readonly cancelCalls: Array<string | undefined> = []
  readonly closeCalls: Array<{
    reason: string
    discardPersistentState?: boolean
  }> = []
  ensureSessionHook?: (input: AcpRuntimeEnsureInput) => Promise<void>
  setModeHook?: () => Promise<void>
  closeHook?: () => Promise<void>
  private turnIndex = 0

  constructor(private options: RecordingAcpRuntimeOptions = {}) {}

  async ensureSession(input: AcpRuntimeEnsureInput): Promise<AcpRuntimeHandle> {
    this.ensureSessionCalls.push(input)
    if (this.options.ensureError) throw this.options.ensureError
    await this.ensureSessionHook?.(input)
    return {
      sessionKey: input.sessionKey,
      backend: 'test',
      runtimeSessionName: input.sessionKey,
      cwd: input.cwd,
    }
  }

  startTurn(input: AcpRuntimeTurnInput): AcpRuntimeTurn {
    this.startTurnCalls.push(input)
    const turnIndex = this.turnIndex
    const events = this.options.turns?.[turnIndex] ?? []
    this.turnIndex += 1
    return {
      requestId: `request-${this.turnIndex}`,
      promptStarted: Promise.resolve(),
      events: iterate(events),
      result: Promise.resolve<AcpRuntimeTurnResult>(
        this.options.results?.[turnIndex] ?? {
          status: 'completed',
          stopReason: 'end_turn',
        },
      ),
      cancel: async () => {},
      closeStream: async () => {},
    }
  }

  async *runTurn(input: AcpRuntimeTurnInput): AsyncIterable<AcpRuntimeEvent> {
    const turn = this.startTurn(input)
    yield* turn.events
  }

  async setMode(input: {
    handle: AcpRuntimeHandle
    mode: string
  }): Promise<void> {
    this.setModeCalls.push(input.mode)
    await this.setModeHook?.()
    if (this.options.rejectedModes?.includes(input.mode)) {
      throw new Error(`unsupported mode: ${input.mode}`)
    }
  }

  async setConfigOption(input: {
    handle: AcpRuntimeHandle
    key: string
    value: string
  }): Promise<void> {
    this.setConfigOptionCalls.push({ key: input.key, value: input.value })
  }

  async cancel(input: {
    handle: AcpRuntimeHandle
    reason?: string
  }): Promise<void> {
    this.cancelCalls.push(input.reason)
  }

  async close(input: {
    handle: AcpRuntimeHandle
    reason: string
    discardPersistentState?: boolean
  }): Promise<void> {
    this.closeCalls.push({
      reason: input.reason,
      discardPersistentState: input.discardPersistentState,
    })
    await this.closeHook?.()
  }
}

async function* iterate(
  events: AcpRuntimeEvent[],
): AsyncIterable<AcpRuntimeEvent> {
  yield* events
}
