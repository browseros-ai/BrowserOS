/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { join } from 'node:path'
import {
  AcpxError,
  type AcpxProvider,
  type AcpxProviderSettings,
  createAcpxProvider,
} from '@browseros/acpx-ai-provider'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { createFileSessionStore } from 'acpx/runtime'
import {
  convertToModelMessages,
  stepCountIs,
  streamText,
  type UIMessage,
  type UIMessageChunk,
} from 'ai'
import { getBrowserosDir } from '../../browseros-dir'
import { logger } from '../../logger'
import type { AcpAgentDefinition } from '../agent-types'
import { deriveAcpSessionKey } from '../storage/acp-agent-store'
import { type AcpAgentPolicy, buildAcpAgentPolicy } from './acp-agent-policy'

export interface AcpAgentRuntimeOptions {
  serverPort: number
  resourcesDir?: string | null
  browserosDir?: string
  stateDir?: string
  idleTimeoutMs?: number
  createProvider?: (settings: AcpxProviderSettings) => AcpxProvider
}

export interface AcpAgentStreamInput {
  agent: AcpAgentDefinition
  conversationId: string
  messages: UIMessage[]
  browserContext?: BrowserContext
  abortSignal?: AbortSignal
  onFinish?: (result: {
    messages: UIMessage[]
    isAborted: boolean
  }) => Promise<void> | void
}

interface ActiveAcpSession {
  provider: AcpxProvider
  policyFingerprint: string
  hasHistory: boolean
  idleTimer: ReturnType<typeof setTimeout> | null
}

const DEFAULT_IDLE_TIMEOUT_MS = 30 * 60_000

export class AcpAgentSessionBusyError extends Error {
  constructor() {
    super('An ACP turn is already running for this conversation')
    this.name = 'AcpAgentSessionBusyError'
  }
}

export class AcpAgentPreparationError extends Error {
  constructor() {
    super('Unable to start the ACP agent.')
    this.name = 'AcpAgentPreparationError'
  }
}

export class AcpAgentRuntime {
  private readonly serverPort: number
  private readonly resourcesDir: string | null
  private readonly browserosDir: string
  private readonly stateDir: string
  private readonly idleTimeoutMs: number
  private readonly createProvider: (
    settings: AcpxProviderSettings,
  ) => AcpxProvider
  private readonly sessions = new Map<string, ActiveAcpSession>()
  private readonly activeTurns = new Set<string>()

  constructor(options: AcpAgentRuntimeOptions) {
    this.serverPort = options.serverPort
    this.resourcesDir = options.resourcesDir ?? null
    this.browserosDir = options.browserosDir ?? getBrowserosDir()
    this.stateDir =
      options.stateDir ?? join(this.browserosDir, 'agents', 'acp-sessions')
    this.idleTimeoutMs = options.idleTimeoutMs ?? DEFAULT_IDLE_TIMEOUT_MS
    this.createProvider = options.createProvider ?? createAcpxProvider
  }

  async stream(
    input: AcpAgentStreamInput,
  ): Promise<ReadableStream<UIMessageChunk>> {
    const sessionKey = deriveAcpSessionKey(input.agent.id, input.conversationId)
    if (this.activeTurns.has(sessionKey)) {
      throw new AcpAgentSessionBusyError()
    }
    this.activeTurns.add(sessionKey)

    let createdSession = false
    let streamStarted = false

    try {
      const policy = await buildAcpAgentPolicy({
        agent: input.agent,
        conversationId: input.conversationId,
        serverPort: this.serverPort,
        browserContext: input.browserContext,
        resourcesDir: this.resourcesDir,
        browserosDir: this.browserosDir,
      })
      const acquired = await this.acquireSession(policy)
      const session = acquired.session
      createdSession = acquired.created
      if (input.abortSignal?.aborted) {
        throw input.abortSignal.reason ?? new Error('ACP turn was aborted')
      }

      await applyFullAccess(session.provider, policy)
      await applyReasoningEffort(session.provider, input.agent)

      const messages = session.hasHistory
        ? latestUserTurn(input.messages)
        : input.messages
      const modelMessages = await convertToModelMessages(messages)
      const result = streamText({
        model: session.provider.languageModel(),
        messages: modelMessages,
        abortSignal: input.abortSignal,
        stopWhen: stepCountIs(1),
        onError: ({ error }) => {
          logger.error('ACP agent stream failed', {
            agentId: input.agent.id,
            conversationId: input.conversationId,
            error: error instanceof Error ? error.message : String(error),
          })
        },
      })
      const stream = result.toUIMessageStream({
        originalMessages: messages,
        onFinish: async ({ messages: finishedMessages, isAborted }) => {
          session.hasHistory = true
          await input.onFinish?.({
            messages: finishedMessages,
            isAborted,
          })
        },
        onError: acpUiErrorMessage,
      })
      streamStarted = true
      return releaseOnEnd(stream, () => {
        this.activeTurns.delete(sessionKey)
        this.scheduleIdleClose(sessionKey, session)
      })
    } catch (error) {
      if (createdSession) {
        await this.close(input.agent.id, input.conversationId).catch(() => {})
      }
      logger.error('ACP agent preparation failed', {
        agentId: input.agent.id,
        conversationId: input.conversationId,
        error: error instanceof Error ? error.message : String(error),
      })
      throw new AcpAgentPreparationError()
    } finally {
      if (!streamStarted) this.activeTurns.delete(sessionKey)
    }
  }

  async close(
    agentId: string,
    conversationId: string,
    options: { discardPersistentState?: boolean } = {},
  ): Promise<boolean> {
    const sessionKey = deriveAcpSessionKey(agentId, conversationId)
    const session = this.sessions.get(sessionKey)
    if (!session) return false
    this.sessions.delete(sessionKey)
    clearIdleTimer(session)
    await session.provider.close('close', options)
    return true
  }

  async closeAllForAgent(
    agentId: string,
    options: { discardPersistentState?: boolean } = {},
  ): Promise<number> {
    const prefix = `acp:${agentId}:`
    const sessionKeys = [...this.sessions.keys()].filter((key) =>
      key.startsWith(prefix),
    )
    await Promise.all(
      sessionKeys.map(async (sessionKey) => {
        const session = this.sessions.get(sessionKey)
        if (!session) return
        this.sessions.delete(sessionKey)
        clearIdleTimer(session)
        await session.provider.close('agent-delete', options)
      }),
    )
    return sessionKeys.length
  }

  private async acquireSession(
    policy: AcpAgentPolicy,
  ): Promise<{ session: ActiveAcpSession; created: boolean }> {
    const fingerprint = JSON.stringify(policy)
    const existing = this.sessions.get(policy.sessionKey)
    if (existing?.policyFingerprint === fingerprint) {
      clearIdleTimer(existing)
      return { session: existing, created: false }
    }

    if (existing) {
      this.sessions.delete(policy.sessionKey)
      clearIdleTimer(existing)
      await existing.provider.close('policy-change')
    }

    const provider = this.createProvider({
      agent: policy.adapter,
      cwd: policy.cwd,
      sessionKey: policy.sessionKey,
      sessionMode: 'persistent',
      stateDir: this.stateDir,
      agentRegistryOverrides: policy.agentRegistryOverrides,
      permissionMode: 'approve-all',
      nonInteractivePermissions: 'deny',
      mcpServers: policy.mcpServers,
      sessionOptions: policy.sessionOptions,
    })

    let hasHistory: boolean
    try {
      await provider.prepare()
      const persistedRecord = await createFileSessionStore({
        stateDir: this.stateDir,
      }).load(policy.sessionKey)
      hasHistory = persistedRecord
        ? persistedRecord.messages.length > 0
        : (existing?.hasHistory ?? false)
    } catch (error) {
      await provider.close('prepare-failed').catch(() => {})
      throw error
    }

    const session = {
      provider,
      policyFingerprint: fingerprint,
      hasHistory,
      idleTimer: null,
    }
    this.sessions.set(policy.sessionKey, session)
    return { session, created: true }
  }

  private scheduleIdleClose(
    sessionKey: string,
    session: ActiveAcpSession,
  ): void {
    clearIdleTimer(session)
    if (this.idleTimeoutMs <= 0) return
    session.idleTimer = setTimeout(() => {
      if (
        this.activeTurns.has(sessionKey) ||
        this.sessions.get(sessionKey) !== session
      ) {
        return
      }
      this.sessions.delete(sessionKey)
      session.idleTimer = null
      void session.provider.close('idle').catch((error) => {
        logger.warn('Failed to close idle ACP session', {
          sessionKey,
          error: error instanceof Error ? error.message : String(error),
        })
      })
    }, this.idleTimeoutMs)
    const idleTimer = session.idleTimer as ReturnType<typeof setTimeout> & {
      unref?: () => void
    }
    idleTimer.unref?.()
  }
}

function clearIdleTimer(session: ActiveAcpSession): void {
  if (session.idleTimer) clearTimeout(session.idleTimer)
  session.idleTimer = null
}

function releaseOnEnd<T>(
  stream: ReadableStream<T>,
  release: () => void,
): ReadableStream<T> {
  const reader = stream.getReader()
  let released = false
  const releaseOnce = () => {
    if (released) return
    released = true
    release()
  }

  return new ReadableStream<T>({
    async pull(controller) {
      try {
        const result = await reader.read()
        if (result.done) {
          releaseOnce()
          controller.close()
          return
        }
        controller.enqueue(result.value)
      } catch (error) {
        releaseOnce()
        controller.error(error)
      }
    },
    async cancel(reason) {
      try {
        await reader.cancel(reason)
      } finally {
        releaseOnce()
      }
    },
  })
}

async function applyFullAccess(
  provider: AcpxProvider,
  policy: AcpAgentPolicy,
): Promise<void> {
  if (policy.fullAccessModeCandidates.length === 0) {
    // No bypass mode configured (common for custom agents). Run in the agent's
    // own default permission mode rather than forcing one.
    return
  }
  if (!provider.runtime.setMode) {
    throw new Error(`ACP adapter ${policy.adapter} has no full-access mode`)
  }

  let lastError: unknown
  for (const mode of policy.fullAccessModeCandidates) {
    try {
      await provider.setMode(mode)
      return
    } catch (error) {
      lastError = error
    }
  }

  throw new Error(`Unable to enable full access for ${policy.adapter}`, {
    cause: lastError,
  })
}

function resolveReasoningEffortKey(agent: AcpAgentDefinition): string {
  if (agent.type === 'custom') {
    return agent.customConfig?.reasoningEffortKey ?? 'effort'
  }
  return agent.type === 'codex' ? 'reasoning_effort' : 'effort'
}

async function applyReasoningEffort(
  provider: AcpxProvider,
  agent: AcpAgentDefinition,
): Promise<void> {
  if (!agent.reasoningEffort || !provider.runtime.setConfigOption) return
  const key = resolveReasoningEffortKey(agent)
  try {
    await provider.setConfigOption(key, agent.reasoningEffort)
  } catch (error) {
    logger.warn('ACP reasoning effort was rejected', {
      agentId: agent.id,
      adapter: agent.type,
      reasoningEffort: agent.reasoningEffort,
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

function latestUserTurn(messages: UIMessage[]): UIMessage[] {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message?.role === 'user') return [message]
  }
  return []
}

function acpUiErrorMessage(error: unknown): string {
  return error instanceof AcpxError
    ? error.message
    : 'The ACP agent failed to respond.'
}
