/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import {
  consumeStream,
  createAgentUIStreamResponse,
  createUIMessageStream,
  createUIMessageStreamResponse,
  type UIMessage,
  type UIMessageChunk,
} from 'ai'
import { AiSdkAgent } from '../../agent/ai-sdk-agent'
import { toChatErrorText } from '../../agent/chat-error'
import { formatUserMessage } from '../../agent/format-message'
import {
  filterValidMessages,
  sanitizeMessagesForToolset,
  stripReasoningParts,
} from '../../agent/message-validation'
import type { AgentSession, SessionStore } from '../../agent/session-store'
import type { ResolvedAgentConfig } from '../../agent/types'
import {
  AcpAgentPreparationError,
  AcpAgentRuntime,
  AcpAgentSessionBusyError,
  type AcpAgentStreamInput,
} from '../../lib/agents/acp/acp-agent-runtime'
import type { AcpAgentStore } from '../../lib/agents/storage/acp-agent-store'
import { DbAcpAgentStore } from '../../lib/agents/storage/acp-agent-store'
import { resolveLLMConfig } from '../../lib/clients/llm/config'
import {
  type ConversationStore,
  DbConversationStore,
  type SaveConversationInput,
} from '../../lib/conversations/conversation-store'
import { logger } from '../../lib/logger'
import type { KlavisService } from '../services/klavis'
import type { ServerActivity } from '../services/server-activity'
import type {
  AcpChatRequest,
  BrowserContext,
  BrowserOsChatRequest,
  ChatRequest,
} from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'
import {
  describeMcpChange,
  describeModeChange,
  describeWorkspaceChange,
} from './chat-service.helpers'

export interface ChatServiceDeps {
  sessionStore: SessionStore
  klavis?: KlavisService
  browser: Browser
  browserSession: BrowserSession
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
  serverPort: number
  resourcesDir?: string | null
  activity?: ServerActivity
  acpAgentStore?: Pick<AcpAgentStore, 'get'>
  acpRuntime?: Pick<AcpAgentRuntime, 'stream' | 'close'>
  conversationStore?: Pick<ConversationStore, 'get' | 'save'>
}

export class ChatService {
  private acpAgentStore: Pick<AcpAgentStore, 'get'> | undefined
  private acpRuntime: Pick<AcpAgentRuntime, 'stream' | 'close'> | undefined
  private readonly acpMessages = new Map<string, UIMessage[]>()
  private readonly acpConversationAgents = new Map<string, string>()
  private conversationStore: Pick<ConversationStore, 'get' | 'save'> | undefined

  constructor(private deps: ChatServiceDeps) {
    this.acpAgentStore = deps.acpAgentStore
    this.acpRuntime = deps.acpRuntime
    this.conversationStore = deps.conversationStore
  }

  async processMessage(
    request: ChatRequest,
    abortSignal: AbortSignal,
  ): Promise<Response> {
    if (request.target.type === 'claude' || request.target.type === 'codex') {
      return this.processAcpMessage(request as AcpChatRequest, abortSignal)
    }

    return this.processBrowserOsMessage(
      request as BrowserOsChatRequest,
      abortSignal,
    )
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: session changes and message persistence must share one ordered transaction
  private async processBrowserOsMessage(
    request: BrowserOsChatRequest,
    abortSignal: AbortSignal,
  ): Promise<Response> {
    const { sessionStore } = this.deps

    const llmConfig = await resolveLLMConfig(request, this.deps.browserosId)

    let session = sessionStore.get(request.conversationId)

    const agentConfig: ResolvedAgentConfig = {
      conversationId: request.conversationId,
      provider: llmConfig.provider,
      providerId: llmConfig.providerId,
      model: llmConfig.model,
      apiKey: llmConfig.apiKey,
      baseUrl: llmConfig.baseUrl,
      upstreamProvider: llmConfig.upstreamProvider,
      resourceName: llmConfig.resourceName,
      region: llmConfig.region,
      accessKeyId: llmConfig.accessKeyId,
      secretAccessKey: llmConfig.secretAccessKey,
      sessionToken: llmConfig.sessionToken,
      accountId: llmConfig.accountId,
      reasoningEffort: request.reasoningEffort,
      reasoningSummary: request.reasoningSummary,
      contextWindowSize: request.contextWindowSize,
      userSystemPrompt: request.userSystemPrompt,
      workingDir: request.userWorkingDir,
      supportsImages: request.supportsImages,
      supportsReasoning: request.supportsReasoning,
      chatMode: request.mode === 'chat',
      isScheduledTask: request.isScheduledTask,
      origin: request.origin,
      declinedApps: request.declinedApps,
      browserosId: this.deps.browserosId,
    }

    let isNewSession = false
    const contextChanges: string[] = []

    const mcpServerKey = this.buildMcpServerKey(request.browserContext)

    // Snapshot the inputs the cached session was built with, before any
    // rebuild. rebuildSession restamps these, so both change detection and the
    // notices below must read from this snapshot, not from the (possibly
    // rebuilt) session.
    const requestChatMode = agentConfig.chatMode ?? false
    const prior = session && {
      mcpServerKey: session.mcpServerKey,
      workingDir: session.workingDir,
      chatMode: session.chatMode,
    }

    const mcpChanged = !!prior && prior.mcpServerKey !== mcpServerKey
    const workspaceChanged =
      !!prior && prior.workingDir !== request.userWorkingDir
    const modeChanged = !!prior && prior.chatMode !== requestChatMode

    // One rebuild reflects every change, because rebuildSession reads the
    // current agentConfig, mcpServerKey, and request. Switching to chat mode
    // drops the agent's record of tool calls it already made
    // (sanitizeMessagesForToolset removes parts the narrower toolset lacks) and
    // switching back does not restore them.
    if (session && (mcpChanged || workspaceChanged || modeChanged)) {
      logger.info('Rebuilding session for mid-conversation input changes', {
        conversationId: request.conversationId,
        mcpChanged,
        workspaceChanged,
        modeChanged,
      })
      session = await this.rebuildSession(
        session,
        request,
        agentConfig,
        mcpServerKey,
      )
    }

    // Emit one notice per change, reading pre-rebuild values from `prior`.
    // Independent of how many rebuilds ran (at most one), so a turn that
    // changes several inputs still tells the model about each of them.
    if (mcpChanged && prior) {
      contextChanges.push(describeMcpChange(prior.mcpServerKey, mcpServerKey))
    }
    if (workspaceChanged && prior) {
      contextChanges.push(
        describeWorkspaceChange(
          prior.workingDir,
          request.userWorkingDir,
          requestChatMode,
        ),
      )
    }
    if (modeChanged) {
      contextChanges.push(
        describeModeChange(requestChatMode, !!request.userWorkingDir),
      )
    }

    if (!session) {
      isNewSession = true
      let scheduledPageId: number | undefined
      let browserContext = await resolveBrowserContextPageIds(
        this.deps.browser,
        request.browserContext,
      )
      if (request.isScheduledTask) {
        try {
          scheduledPageId = await this.deps.browser.newPage('about:blank', {
            background: true,
          })
          let scheduledWindowId: number | undefined
          try {
            const scheduledPage = (await this.deps.browser.listPages()).find(
              (page) => page.pageId === scheduledPageId,
            )
            scheduledWindowId = scheduledPage?.windowId
          } catch (error) {
            logger.warn('Failed to look up scheduled page metadata', {
              conversationId: request.conversationId,
              pageId: scheduledPageId,
              error: error instanceof Error ? error.message : String(error),
            })
          }
          browserContext = {
            ...browserContext,
            windowId: scheduledWindowId,
            selectedTabs: undefined,
            tabs: undefined,
            activeTab: {
              id: scheduledPageId,
              pageId: scheduledPageId,
              url: 'about:blank',
              title: 'Scheduled Task',
            },
          }
          logger.info('Created background page for scheduled task', {
            conversationId: request.conversationId,
            pageId: scheduledPageId,
            windowId: scheduledWindowId,
          })
        } catch (error) {
          logger.warn(
            'Failed to create scheduled page, using default browser context',
            {
              error: error instanceof Error ? error.message : String(error),
            },
          )
        }
      }

      const outputFileAccess = createBrowserOutputFileAccess()
      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browserSession: this.deps.browserSession,
        browserContext,
        klavis: this.deps.klavis,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
        outputFileAccess,
      })
      session = {
        agent,
        scheduledPageId,
        browserContext,
        mcpServerKey,
        workingDir: request.userWorkingDir,
        chatMode: requestChatMode,
        outputFileAccess,
      }
      sessionStore.set(request.conversationId, session)
    }

    if (isNewSession) {
      if (request.historyMode === 'local') {
        const stored = await this.getConversationStore().get(
          request.conversationId,
        )
        // Only reuse a record this target owns; a conversationId shared with an
        // ACP agent must not bleed its history into the BrowserOS agent.
        if (stored?.messages.length && stored.targetType === 'browseros') {
          session.agent.messages = stored.messages
          logger.info('Hydrated conversation history from database', {
            conversationId: request.conversationId,
            messageCount: stored.messages.length,
          })
        }
      } else if (request.previousConversation?.length) {
        for (const msg of request.previousConversation) {
          if (!msg.content.trim()) continue
          session.agent.messages.push({
            id: crypto.randomUUID(),
            role: msg.role === 'assistant' ? 'assistant' : 'user',
            parts: [{ type: 'text', text: msg.content }],
          })
        }
        logger.info('Injected previous conversation history', {
          conversationId: request.conversationId,
          messageCount: request.previousConversation.length,
        })
      }
    }

    const messageContext = request.isScheduledTask
      ? (session.browserContext ?? request.browserContext)
      : request.browserContext
    // Scheduled tasks already have correct internal pageIds from browser.newPage();
    // resolving them again would pass those to resolveTabIds, which expects Chrome
    // tab IDs.
    const resolvedMessageContext = request.isScheduledTask
      ? messageContext
      : await resolveBrowserContextPageIds(this.deps.browser, messageContext)
    const userContent = formatUserMessage(
      request.message,
      resolvedMessageContext,
      request.selectedText,
      request.selectedTextSource,
    )

    const contextPrefix =
      contextChanges.length > 0
        ? `${contextChanges.map((c) => `[Context: ${c}]`).join('\n')}\n\n`
        : ''

    // Persist the *raw* user text in session.agent.messages so it
    // round-trips clean to the client's useChat state and to any
    // future history reload. The wrapped form (browser context +
    // <selected_text> + <USER_QUERY>) is built as a transient prompt
    // copy below — the LLM sees it, the user-visible state never
    // does.
    session.agent.appendUserMessage(request.message)
    const promptUserText = contextPrefix + userContent
    const wrappedUserMessageId =
      session.agent.messages[session.agent.messages.length - 1]?.id

    // Strip reasoning from the request copy only. session.agent.messages keeps
    // reasoning so it still persists to SQLite and renders in the client history;
    // the model must not see replayed reasoning (see stripReasoningParts).
    const promptUiMessages: UIMessage[] = stripReasoningParts(
      filterValidMessages(session.agent.messages),
    ).map((message) =>
      message.id === wrappedUserMessageId && message.role === 'user'
        ? {
            ...message,
            parts: [{ type: 'text' as const, text: promptUserText }],
          }
        : message,
    )

    const response = await createAgentUIStreamResponse({
      agent: session.agent.toolLoopAgent,
      uiMessages: promptUiMessages,
      abortSignal,
      // Without this the SDK substitutes its masking default, which discards
      // the status code, provider code, and message of every mid-stream
      // failure - including every rate limit and credit exhaustion.
      onError: (error: unknown) => {
        logger.error('Agent stream failed', {
          conversationId: request.conversationId,
          provider: agentConfig.provider,
          model: agentConfig.model,
          message: error instanceof Error ? error.message : String(error),
        })
        return toChatErrorText(error, { provider: agentConfig.provider })
      },
      onFinish: async ({ messages }: { messages: UIMessage[] }) => {
        const restored = messages.map((message) =>
          message.id === wrappedUserMessageId && message.role === 'user'
            ? {
                ...message,
                parts: [{ type: 'text' as const, text: request.message }],
              }
            : message,
        )
        session.agent.messages = filterValidMessages(restored)

        // Only persist a turn that produced an assistant reply. A stream that
        // errored before any output leaves the history ending on the user
        // message; persisting that corrupts durable history, because the next
        // turn reloads it and appends another user message, and the provider
        // rejects back-to-back user messages.
        const turnCompleted =
          session.agent.messages[session.agent.messages.length - 1]?.role ===
          'assistant'

        logger.info('Agent execution complete', {
          conversationId: request.conversationId,
          totalMessages: session.agent.messages.length,
          turnCompleted,
        })

        if (request.historyMode === 'local' && turnCompleted) {
          await this.persistConversation({
            id: request.conversationId,
            messages: session.agent.messages,
            targetType: 'browseros',
            origin: request.origin,
          })
        }

        if (session.scheduledPageId) {
          const pageId = session.scheduledPageId
          session.scheduledPageId = undefined
          this.closeScheduledPage(pageId, request.conversationId)
        }
      },
    })

    return (
      this.deps.activity?.trackChatResponse(response, abortSignal) ?? response
    )
  }

  async deleteSession(
    conversationId: string,
  ): Promise<{ deleted: boolean; sessionCount: number }> {
    let acpDeleted = false
    const acpAgentId = this.acpConversationAgents.get(conversationId)
    if (acpAgentId) {
      await this.getAcpRuntime().close(acpAgentId, conversationId, {
        discardPersistentState: true,
      })
      this.acpConversationAgents.delete(conversationId)
      this.acpMessages.delete(`${acpAgentId}:${conversationId}`)
      acpDeleted = true
    }

    const session = this.deps.sessionStore.get(conversationId)
    if (session?.scheduledPageId) {
      const pageId = session.scheduledPageId
      session.scheduledPageId = undefined
      this.closeScheduledPage(pageId, conversationId)
    }
    const deleted = await this.deps.sessionStore.delete(conversationId)
    return {
      deleted: deleted || acpDeleted,
      sessionCount: this.deps.sessionStore.count(),
    }
  }

  isAcpSession(conversationId: string): boolean {
    return this.acpConversationAgents.has(conversationId)
  }

  private async processAcpMessage(
    request: AcpChatRequest,
    abortSignal: AbortSignal,
  ): Promise<Response> {
    const agent = await this.getAcpAgentStore().get(request.target.agentId)
    if (!agent)
      return Response.json({ error: 'Unknown agent' }, { status: 404 })
    if (agent.type !== request.target.type) {
      return Response.json({ error: 'Agent type mismatch' }, { status: 400 })
    }

    const browserContext = await resolveBrowserContextPageIds(
      this.deps.browser,
      request.browserContext,
    )
    const userContent = formatUserMessage(
      request.message,
      browserContext,
      request.selectedText,
      request.selectedTextSource,
    )
    const promptText = request.userSystemPrompt?.trim()
      ? `${request.userSystemPrompt.trim()}\n\n${userContent}`
      : userContent
    const historyKey = `${agent.id}:${request.conversationId}`
    const history: UIMessage[] =
      this.acpMessages.get(historyKey) ??
      (await this.loadAcpDisplayHistory(agent.id, request))
    const priorHistoryLength = history.length
    const messageId = crypto.randomUUID()
    const files = (request.attachments ?? []).map((attachment) => ({
      type: 'file' as const,
      mediaType: attachment.mediaType,
      url: attachment.data.startsWith('data:')
        ? attachment.data
        : `data:${attachment.mediaType};base64,${attachment.data}`,
    }))
    const visibleUserMessage: UIMessage = {
      id: messageId,
      role: 'user',
      parts: [
        ...(request.message
          ? [{ type: 'text' as const, text: request.message }]
          : []),
        ...files,
      ],
    }
    history.push(visibleUserMessage)
    this.acpMessages.set(historyKey, history)
    this.acpConversationAgents.set(request.conversationId, agent.id)
    const promptMessages = history.map((message) =>
      message.id === messageId
        ? {
            ...message,
            parts: [{ type: 'text' as const, text: promptText }, ...files],
          }
        : message,
    )
    const streamInput: AcpAgentStreamInput = {
      agent: {
        ...agent,
        workingDirectory: request.userWorkingDir ?? agent.workingDirectory,
      },
      conversationId: request.conversationId,
      messages: promptMessages,
      browserContext,
      abortSignal,
      onFinish: async ({ messages }) => {
        const existingIds = new Set(history.map((message) => message.id))
        history.push(
          ...messages.filter((message) => !existingIds.has(message.id)),
        )
        await this.persistConversation({
          id: request.conversationId,
          messages: history,
          targetType: agent.type,
          origin: request.origin,
          agentId: agent.id,
        })
      },
    }
    let stream: ReadableStream<UIMessageChunk>
    try {
      stream = await this.getAcpRuntime().stream(streamInput)
    } catch (error) {
      history.length = priorHistoryLength
      if (error instanceof AcpAgentSessionBusyError) {
        return Response.json(
          { error: 'An agent turn is already running' },
          { status: 409 },
        )
      }
      if (!(error instanceof AcpAgentPreparationError)) throw error
      stream = createUIMessageStream({
        execute({ writer }) {
          writer.write({ type: 'error', errorText: error.message })
        },
      })
    }
    const response = createUIMessageStreamResponse({
      stream,
      consumeSseStream: consumeStream,
    })
    return (
      this.deps.activity?.trackChatResponse(response, abortSignal) ?? response
    )
  }

  private getAcpAgentStore(): Pick<AcpAgentStore, 'get'> {
    this.acpAgentStore ??= new DbAcpAgentStore()
    return this.acpAgentStore
  }

  private getAcpRuntime(): Pick<AcpAgentRuntime, 'stream' | 'close'> {
    this.acpRuntime ??= new AcpAgentRuntime({
      serverPort: this.deps.serverPort,
      resourcesDir: this.deps.resourcesDir,
    })
    return this.acpRuntime
  }

  private getConversationStore(): Pick<ConversationStore, 'get' | 'save'> {
    this.conversationStore ??= new DbConversationStore()
    return this.conversationStore
  }

  // Best-effort: a persistence failure must not fail an already-streamed turn.
  private async persistConversation(
    input: SaveConversationInput,
  ): Promise<void> {
    try {
      await this.getConversationStore().save(input)
    } catch (error) {
      logger.warn('Failed to persist conversation history', {
        conversationId: input.id,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  // ACP continuity lives in acpx; SQLite holds a display copy. On a cold turn
  // (e.g. after a restart) re-seed from that copy before falling back to the
  // client-supplied history.
  private async loadAcpDisplayHistory(
    agentId: string,
    request: AcpChatRequest,
  ): Promise<UIMessage[]> {
    try {
      const stored = await this.getConversationStore().get(
        request.conversationId,
      )
      // Only reuse the display copy when it belongs to this agent; the in-memory
      // key is agent-scoped, so a shared conversationId must not cross agents.
      if (stored?.messages.length && stored.agentId === agentId) {
        return stored.messages
      }
    } catch (error) {
      logger.warn('Failed to load ACP display history', {
        conversationId: request.conversationId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
    return (request.previousConversation ?? []).map((message) => ({
      id: crypto.randomUUID(),
      role: message.role,
      parts: [{ type: 'text' as const, text: message.content }],
    }))
  }

  private closeScheduledPage(pageId: number, conversationId: string): void {
    this.deps.browser.closePage(pageId).catch((error) => {
      logger.warn('Failed to close scheduled page', {
        pageId,
        conversationId,
        error: error instanceof Error ? error.message : String(error),
      })
    })
  }

  private async rebuildSession(
    session: AgentSession,
    request: ChatRequest,
    agentConfig: ResolvedAgentConfig,
    mcpServerKey: string,
  ): Promise<AgentSession> {
    const previousMessages = session.agent.messages
    await session.agent.dispose()
    this.deps.sessionStore.remove(request.conversationId)

    const browserContext = agentConfig.isScheduledTask
      ? (session.browserContext ??
        (await resolveBrowserContextPageIds(
          this.deps.browser,
          request.browserContext,
        )))
      : await resolveBrowserContextPageIds(
          this.deps.browser,
          request.browserContext,
        )
    const outputFileAccess =
      session.outputFileAccess ?? createBrowserOutputFileAccess()
    const agent = await AiSdkAgent.create({
      resolvedConfig: agentConfig,
      browserSession: this.deps.browserSession,
      browserContext,
      klavis: this.deps.klavis,
      browserosId: this.deps.browserosId,
      aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
      outputFileAccess,
    })
    const newSession: AgentSession = {
      agent,
      scheduledPageId: session.scheduledPageId,
      browserContext,
      mcpServerKey,
      workingDir: request.userWorkingDir,
      chatMode: agentConfig.chatMode ?? false,
      outputFileAccess,
    }
    newSession.agent.messages = sanitizeMessagesForToolset(
      previousMessages,
      agent.toolNames,
    )
    this.deps.sessionStore.set(request.conversationId, newSession)
    return newSession
  }

  private buildMcpServerKey(browserContext?: BrowserContext): string {
    const managed = browserContext?.enabledMcpServers?.slice().sort() ?? []
    const custom =
      browserContext?.customMcpServers?.map((s) => s.url).sort() ?? []
    const klavisState =
      managed.length > 0
        ? `klavis:${this.deps.klavis?.getProxyStatus().state ?? 'disabled'}`
        : null
    return [klavisState, ...managed, ...custom].filter(Boolean).join(',')
  }
}
