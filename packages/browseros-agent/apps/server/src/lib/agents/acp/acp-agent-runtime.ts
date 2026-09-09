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
import { type AcpSessionRecord, createFileSessionStore } from 'acpx/runtime'
import {
  convertToModelMessages,
  createUIMessageStream,
  type ModelMessage,
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
import { ensureAcpWorkspace } from './browseros-instructions'

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
  browserToolLeaseToken: string
  readOnly: boolean
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

/**
 * Owns the persistent ACP process for each agent/conversation pair. Saved display
 * messages seed fresh sessions; a pre-prompt resume retry shares the same UI
 * response and turn lock, so neither replies nor browser actions are duplicated.
 */
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
  private workspaceReady?: Promise<string>

  // Materialize the single shared ACP workspace (CLAUDE.md / AGENTS.md) once and
  // reuse it for every conversation.
  private ensureWorkspace(): Promise<string> {
    this.workspaceReady ??= ensureAcpWorkspace(this.browserosDir)
    return this.workspaceReady
  }

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
      await this.ensureWorkspace()
      const policy = await buildAcpAgentPolicy({
        agent: input.agent,
        conversationId: input.conversationId,
        serverPort: this.serverPort,
        browserToolLeaseToken: input.browserToolLeaseToken,
        readOnly: input.readOnly,
        browserContext: input.browserContext,
        resourcesDir: this.resourcesDir,
        browserosDir: this.browserosDir,
      })
      let session: ActiveAcpSession
      let recovered = false
      const prepare = async () => {
        input.abortSignal?.throwIfAborted()
        const acquired = await this.acquireSession(policy)
        session = acquired.session
        createdSession ||= acquired.created
        input.abortSignal?.throwIfAborted()
        await applyFullAccess(session.provider, policy)
        await applyReasoningEffort(session.provider, input.agent)
      }
      const canRecover = (error: unknown) =>
        !recovered && !input.abortSignal?.aborted && isResumeFailure(error)
      const recover = async (error: unknown) => {
        if (!canRecover(error)) throw error
        recovered = true
        logger.warn('Starting a fresh ACP session after resume failed', {
          agentId: input.agent.id,
          conversationId: input.conversationId,
        })
        await this.resetSession(policy.sessionKey)
        await prepare()
        session.hasHistory = false
      }
      try {
        await prepare()
      } catch (error) {
        await recover(error)
      }

      // One UI response spans a possible pre-prompt resume retry. The failed
      // attempt must not create an empty assistant reply or persist its error.
      const responseId = crypto.randomUUID()
      let failed = false
      const stream = createUIMessageStream<UIMessage>({
        originalMessages: input.messages,
        generateId: () => responseId,
        execute: async ({ writer }) => {
          writer.write({ type: 'start', messageId: responseId })
          while (true) {
            input.abortSignal?.throwIfAborted()
            const messages = session.hasHistory
              ? latestUserTurn(input.messages)
              : input.messages
            const modelMessages = await convertHistory(messages)
            let turnError: unknown
            const result = streamText({
              model: session.provider.languageModel(),
              messages: modelMessages,
              abortSignal: input.abortSignal,
              stopWhen: stepCountIs(1),
              onError: ({ error }) => {
                turnError = error
              },
            })
            const outcome = await forwardAttempt(
              result.toUIMessageStream({
                sendStart: false,
                onError: (error) => {
                  turnError = error
                  return acpUiErrorMessage(error)
                },
              }),
              (chunk) => writer.write(chunk),
              () => canRecover(turnError),
            )
            if (outcome === 'retry') {
              await recover(turnError)
              continue
            }
            failed = outcome === 'failed'
            return
          }
        },
        onFinish: async ({ messages, isAborted }) => {
          if (!failed && !isAborted) session.hasHistory = true
          await input.onFinish?.({ messages, isAborted })
        },
        onError: (error) => {
          failed = true
          logger.error('ACP agent stream failed', {
            agentId: input.agent.id,
            conversationId: input.conversationId,
            error: error instanceof Error ? error.message : String(error),
          })
          return acpUiErrorMessage(error)
        },
      })
      streamStarted = true
      return releaseOnEnd(stream, () => {
        this.activeTurns.delete(sessionKey)
        const active = this.sessions.get(sessionKey)
        if (active) this.scheduleIdleClose(sessionKey, active)
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
      // An agent may emit startup notices during prepare(). Only user turns
      // prove it has conversation context; otherwise seed it from SQLite.
      hasHistory = persistedRecord?.messages.some(isAcpUserMessage) ?? false
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

  private async resetSession(sessionKey: string): Promise<void> {
    const session = this.sessions.get(sessionKey)
    this.sessions.delete(sessionKey)
    if (session) {
      clearIdleTimer(session)
      await session.provider.close('resume-failed')
    }
    const store = createFileSessionStore({ stateDir: this.stateDir })
    const record = await store.load(sessionKey)
    if (record) {
      // ACP session/close is optional and may itself fail for a stale session.
      // Ask ACPX to replace its local session on the next ensure, retaining
      // SQLite and the old record until the replacement is ready.
      await store.save({
        ...record,
        acpx: { ...record.acpx, reset_on_next_ensure: true },
      })
    }
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

// ACPX validates records on load; the only non-object variant is a resume marker.
function isAcpUserMessage(
  message: AcpSessionRecord['messages'][number],
): boolean {
  return typeof message !== 'string' && 'User' in message
}

/**
 * Forward one attempt, holding its step envelope until work starts. A resume
 * failure can be hidden and retried only before any text/reasoning/tool activity;
 * drain that failed attempt before replacing its agent process.
 */
async function forwardAttempt(
  stream: ReadableStream<UIMessageChunk>,
  write: (chunk: UIMessageChunk) => void,
  canRecover: () => boolean,
): Promise<'completed' | 'failed' | 'retry'> {
  let outcome: 'completed' | 'failed' | 'retry' = 'completed'
  let hasOutput = false
  const pending: UIMessageChunk[] = []
  for await (const chunk of stream) {
    if (outcome === 'retry') continue
    if (chunk.type === 'error' && !hasOutput && canRecover()) {
      outcome = 'retry'
      continue
    }
    if (!hasOutput && chunk.type === 'start-step') {
      pending.push(chunk)
      continue
    }
    hasOutput = true
    for (const prefix of pending.splice(0)) write(prefix)
    if (chunk.type === 'error') outcome = 'failed'
    write(chunk)
  }
  if (outcome !== 'retry') {
    for (const prefix of pending) write(prefix)
  }
  return outcome
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

// ACPX uses these detail codes only before a prompt can execute. Other
// failures (auth, quota, tool errors) must surface without replaying the turn.
function isResumeFailure(error: unknown): boolean {
  const visited = new Set<unknown>()
  while (error && typeof error === 'object' && !visited.has(error)) {
    visited.add(error)
    const detail = 'detailCode' in error ? error.detailCode : undefined
    if (
      detail === 'SESSION_RESUME_REQUIRED' ||
      detail === 'SESSION_MODE_REPLAY_FAILED' ||
      detail === 'SESSION_MODEL_REPLAY_FAILED' ||
      detail === 'SESSION_CONFIG_OPTION_REPLAY_FAILED'
    )
      return true
    error = 'cause' in error ? error.cause : undefined
  }
  return false
}

async function convertHistory(messages: UIMessage[]): Promise<ModelMessage[]> {
  try {
    return await convertToModelMessages(messages, {
      ignoreIncompleteToolCalls: true,
    })
  } catch (error) {
    const latest = latestUserTurn(messages)
    if (latest.length === messages.length) throw error
    // Legacy display records can outlive their message schema. Keep the user's
    // new request usable even when the old transcript cannot be replayed.
    logger.warn('Continuing ACP turn without incompatible saved history')
    return convertToModelMessages(latest)
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
