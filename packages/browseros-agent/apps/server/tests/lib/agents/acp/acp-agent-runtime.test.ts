/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
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
import {
  AcpAgentPreparationError,
  AcpAgentRuntime,
  AcpAgentSessionBusyError,
} from '../../../../src/lib/agents/acp/acp-agent-runtime'
import type { AcpAgentDefinition } from '../../../../src/lib/agents/agent-types'

const SKILL = [
  '---',
  'name: browseros',
  'description: BrowserOS browser skill',
  '---',
  'Use BrowserOS for browser work.',
  '',
].join('\n')

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
  const skillDir = join(resourcesDir, 'skills', 'browseros')
  await mkdir(skillDir, { recursive: true })
  await writeFile(join(skillDir, 'SKILL.md'), SKILL)

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
        systemPrompt: { append: SKILL },
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

  it('sends complete history when ACPX replaces a legacy session during prepare', async () => {
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
        messages: [],
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
  }
}

async function* iterate(
  events: AcpRuntimeEvent[],
): AsyncIterable<AcpRuntimeEvent> {
  yield* events
}
