/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { join } from 'node:path'
import {
  type AcpxProvider,
  type AcpxProviderSettings,
  createAcpxProvider,
} from '@browseros/acpx-ai-provider'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { createFileSessionStore } from 'acpx/runtime'
import {
  convertToModelMessages,
  createUIMessageStream,
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
}

export class AcpAgentRuntime {
  private readonly serverPort: number
  private readonly resourcesDir: string | null
  private readonly browserosDir: string
  private readonly stateDir: string
  private readonly createProvider: (
    settings: AcpxProviderSettings,
  ) => AcpxProvider
  private readonly sessions = new Map<string, ActiveAcpSession>()

  constructor(options: AcpAgentRuntimeOptions) {
    this.serverPort = options.serverPort
    this.resourcesDir = options.resourcesDir ?? null
    this.browserosDir = options.browserosDir ?? getBrowserosDir()
    this.stateDir =
      options.stateDir ?? join(this.browserosDir, 'agents', 'acp-sessions')
    this.createProvider = options.createProvider ?? createAcpxProvider
  }

  async stream(
    input: AcpAgentStreamInput,
  ): Promise<ReadableStream<UIMessageChunk>> {
    let createdSession = false

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
      session.hasHistory = true

      return result.toUIMessageStream({
        originalMessages: messages,
        onFinish: input.onFinish
          ? async ({ messages: finishedMessages, isAborted }) => {
              await input.onFinish?.({
                messages: finishedMessages,
                isAborted,
              })
            }
          : undefined,
        onError: (error) => {
          logger.error('ACP agent UI stream failed', {
            agentId: input.agent.id,
            conversationId: input.conversationId,
            error: error instanceof Error ? error.message : String(error),
          })
          return 'The ACP agent failed to respond.'
        },
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
      return errorStream('Unable to start the ACP agent.')
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
    await session.provider.close('close', options)
    return true
  }

  private async acquireSession(
    policy: AcpAgentPolicy,
  ): Promise<{ session: ActiveAcpSession; created: boolean }> {
    const fingerprint = JSON.stringify(policy)
    const existing = this.sessions.get(policy.sessionKey)
    if (existing?.policyFingerprint === fingerprint) {
      return { session: existing, created: false }
    }

    const hasHistory =
      existing?.hasHistory ??
      Boolean(
        await createFileSessionStore({ stateDir: this.stateDir }).load(
          policy.sessionKey,
        ),
      )
    if (existing) {
      this.sessions.delete(policy.sessionKey)
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

    try {
      await provider.prepare()
    } catch (error) {
      await provider.close('prepare-failed').catch(() => {})
      throw error
    }

    const session = {
      provider,
      policyFingerprint: fingerprint,
      hasHistory,
    }
    this.sessions.set(policy.sessionKey, session)
    return { session, created: true }
  }
}

async function applyFullAccess(
  provider: AcpxProvider,
  policy: AcpAgentPolicy,
): Promise<void> {
  if (
    !provider.runtime.setMode ||
    policy.fullAccessModeCandidates.length === 0
  ) {
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

async function applyReasoningEffort(
  provider: AcpxProvider,
  agent: AcpAgentDefinition,
): Promise<void> {
  if (!agent.reasoningEffort || !provider.runtime.setConfigOption) return
  const key = agent.type === 'codex' ? 'reasoning_effort' : 'effort'
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

function errorStream(message: string): ReadableStream<UIMessageChunk> {
  return createUIMessageStream({
    execute({ writer }) {
      writer.write({ type: 'error', errorText: message })
    },
  })
}
