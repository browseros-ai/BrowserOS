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
} from 'acpx/runtime'
import type { UIMessage, UIMessageChunk } from 'ai'
import { AcpAgentRuntime } from '../../../../src/lib/agents/acp/acp-agent-runtime'
import type { AcpAgentDefinition } from '../../../../src/lib/agents/agent-types'

const SKILL = [
  '---',
  'name: browserclaw',
  'description: BrowserOS browser skill',
  '---',
  'Use BrowserOS for browser work.',
  '',
].join('\n')

const temporaryDirectories: string[] = []

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
}) {
  const root = await mkdtemp(join(tmpdir(), 'acp-agent-runtime-'))
  temporaryDirectories.push(root)
  const resourcesDir = join(root, 'resources')
  const skillDir = join(resourcesDir, 'skills', 'browserclaw')
  await mkdir(skillDir, { recursive: true })
  await writeFile(join(skillDir, 'SKILL.md'), SKILL)

  const adapter = options.adapter ?? 'claude'
  const agent: AcpAgentDefinition = {
    id: `${adapter}-agent-id`,
    name: adapter === 'claude' ? 'Claude Code' : 'Codex',
    type: adapter,
    pinned: false,
    createdAt: 1,
    updatedAt: 1,
    ...options.agent,
  }
  const acpRuntime = options.runtime ?? new RecordingAcpRuntime()
  const providerSettings: AcpxProviderSettings[] = []
  const runtime = new AcpAgentRuntime({
    serverPort: 9100,
    browserosDir: root,
    resourcesDir,
    stateDir: join(root, 'state'),
    createProvider(settings) {
      providerSettings.push(settings)
      return createAcpxProvider({ ...settings, runtime: acpRuntime })
    },
  })

  return { acpRuntime, agent, providerSettings, runtime }
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
            'X-BrowserOS-Scope-Id': 'conversation-1',
            'X-BrowserOS-Agent-Id': 'claude-agent-id',
            'X-BrowserOS-Default-Window-Id': '7',
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
        messages: initialMessages,
      }),
    )
    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-2',
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
    const codexConfig = JSON.parse(
      fixture.providerSettings[0]?.sessionOptions?.env?.CODEX_CONFIG ?? '{}',
    )
    expect(codexConfig.model).toBe('gpt-5.4')
    expect(codexConfig.model_reasoning_effort).toBe('xhigh')
  })

  it('returns one standard error chunk and retains no session when preparation fails', async () => {
    const fixture = await runtimeFixture({
      runtime: new RecordingAcpRuntime({
        ensureError: new Error('native adapter is unavailable'),
      }),
    })

    const parts = await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-4',
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    )

    expect(parts.filter((part) => part.type === 'error')).toEqual([
      { type: 'error', errorText: 'Unable to start the ACP agent.' },
    ])
    expect(
      await fixture.runtime.cancel(
        fixture.agent.id,
        'conversation-4',
        'cancel',
      ),
    ).toBe(false)
  })

  it('cancels and closes only the selected persistent ACP session', async () => {
    const fixture = await runtimeFixture({})
    await collect(
      await fixture.runtime.stream({
        agent: fixture.agent,
        conversationId: 'conversation-5',
        messages: [textMessage('user-1', 'user', 'hello')],
      }),
    )

    expect(
      await fixture.runtime.cancel(fixture.agent.id, 'conversation-5', 'user'),
    ).toBe(true)
    expect(fixture.acpRuntime.cancelCalls).toEqual(['user'])
    expect(
      await fixture.runtime.close(fixture.agent.id, 'conversation-5', {
        discardPersistentState: true,
      }),
    ).toBe(true)
    expect(fixture.acpRuntime.closeCalls).toEqual([
      { reason: 'close', discardPersistentState: true },
    ])
    expect(
      await fixture.runtime.cancel(fixture.agent.id, 'conversation-5'),
    ).toBe(false)
  })
})

interface RecordingAcpRuntimeOptions {
  turns?: AcpRuntimeEvent[][]
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
  private turnIndex = 0

  constructor(private options: RecordingAcpRuntimeOptions = {}) {}

  async ensureSession(input: AcpRuntimeEnsureInput): Promise<AcpRuntimeHandle> {
    this.ensureSessionCalls.push(input)
    if (this.options.ensureError) throw this.options.ensureError
    return {
      sessionKey: input.sessionKey,
      backend: 'test',
      runtimeSessionName: input.sessionKey,
      cwd: input.cwd,
    }
  }

  startTurn(input: AcpRuntimeTurnInput): AcpRuntimeTurn {
    this.startTurnCalls.push(input)
    const events = this.options.turns?.[this.turnIndex] ?? []
    this.turnIndex += 1
    return {
      requestId: `request-${this.turnIndex}`,
      events: iterate(events),
      result: Promise.resolve<AcpRuntimeTurnResult>({
        status: 'completed',
        stopReason: 'end_turn',
      }),
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
