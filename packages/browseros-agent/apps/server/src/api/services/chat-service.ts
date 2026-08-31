/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import {
  type BrowserOutputFileAccess,
  createBrowserOutputFileAccess,
} from '@browseros/browser-mcp/output-file'
import {
  createAgentUIStream,
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
import type {
  BrowserMcpModule,
  BrowserToolLease,
} from '../services/mcp/browser-mcp-module'
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
import {
  ConversationRunAlreadyActiveError,
  type ConversationRunSnapshot,
  ConversationRuns,
  type ConversationTabGroupPresentation,
} from './conversation-runs'

export interface ChatServiceDeps {
  sessionStore: SessionStore
  klavis?: KlavisService
  browser: Browser
  browserMcp: Pick<BrowserMcpModule, 'createLease'>
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
  serverPort: number
  resourcesDir?: string | null
  activity?: ServerActivity
  acpAgentStore?: Pick<AcpAgentStore, 'get'>
  acpRuntime?: Pick<AcpAgentRuntime, 'stream' | 'close'>
  conversationStore?: Pick<ConversationStore, 'get' | 'save'>
  conversationRuns?: ConversationRuns
}

export class ChatService {
  private acpAgentStore: Pick<AcpAgentStore, 'get'> | undefined
  private acpRuntime: Pick<AcpAgentRuntime, 'stream' | 'close'> | undefined
  private readonly acpMessages = new Map<string, UIMessage[]>()
  private readonly acpConversationAgents = new Map<string, string>()
  private readonly acpToolLeases = new Map<
    string,
    {
      agentId: string
      fingerprint: string
      lease: BrowserToolLease
      outputFileAccess: BrowserOutputFileAccess
    }
  >()
  private conversationStore: Pick<ConversationStore, 'get' | 'save'> | undefined
  private readonly conversationRuns: ConversationRuns

  constructor(private deps: ChatServiceDeps) {
    this.acpAgentStore = deps.acpAgentStore
    this.acpRuntime = deps.acpRuntime
    this.conversationStore = deps.conversationStore
    this.conversationRuns =
      deps.conversationRuns ?? new ConversationRuns({ activity: deps.activity })
  }

  async processMessage(
    request: ChatRequest,
    _requestAbortSignal: AbortSignal,
  ): Promise<Response> {
    try {
      await this.conversationRuns.start({
        conversationId: request.conversationId,
        messages: [],
        panelTabIds: browserContextTabIds(request.browserContext),
        panelsVisible: !request.isScheduledTask,
        tabGroup: request.isScheduledTask
          ? undefined
          : conversationTabGroup(request),
        createStream: async (abortSignal, _runId, updateMessages) => {
          if (
            request.target.type === 'claude' ||
            request.target.type === 'codex' ||
            request.target.type === 'custom'
          ) {
            return await this.processAcpMessage(
              request as AcpChatRequest,
              abortSignal,
              updateMessages,
            )
          }

          return await this.processBrowserOsMessage(
            request as BrowserOsChatRequest,
            abortSignal,
            updateMessages,
          )
        },
      })
    } catch (error) {
      if (error instanceof ConversationRunAlreadyActiveError) {
        return Response.json(
          { error: 'An agent turn is already running' },
          { status: 409 },
        )
      }
      if (error instanceof ChatRequestError) return error.response
      throw error
    }

    return createUIMessageStreamResponse({
      stream: this.conversationRuns.subscribe(request.conversationId),
    })
  }

  async getRunSnapshot(
    conversationId: string,
  ): Promise<ConversationRunSnapshot | undefined> {
    return await this.conversationRuns.getPreparedSnapshot(conversationId)
  }

  subscribe(
    conversationId: string,
  ): ReadableStream<UIMessageChunk> | undefined {
    if (!this.conversationRuns.getSnapshot(conversationId)) return undefined
    return this.conversationRuns.subscribe(conversationId)
  }

  async stop(conversationId: string): Promise<boolean> {
    return await this.conversationRuns.stop(conversationId)
  }

  subscribePanelAssignments() {
    return this.conversationRuns.subscribePanelAssignments()
  }

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: session changes and message persistence must share one ordered transaction
  private async processBrowserOsMessage(
    request: BrowserOsChatRequest,
    abortSignal: AbortSignal,
    updateMessages: (messages: UIMessage[]) => boolean,
  ): Promise<ReadableStream<UIMessageChunk>> {
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
      const { agent, browserToolLease } = await this.createBrowserOsAgent(
        agentConfig,
        browserContext,
        outputFileAccess,
      )
      session = {
        agent,
        browserToolLease,
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
    // The native MCP client survives across turns, while the active browser
    // window does not. Refresh server-owned context before activating this run
    // so request-scoped MCP servers use the current targeting defaults.
    session.browserContext = resolvedMessageContext
    session.browserToolLease.updateBrowserContext(resolvedMessageContext)
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
    const messagesBeforeTurn = [...session.agent.messages]
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

    // Publish canonical display history before model work can call loopback MCP
    // or a second panel hydrates this server-owned run.
    updateMessages(session.agent.messages)

    try {
      const stream = await createAgentUIStream({
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
          updateMessages(session.agent.messages)

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

      return stream
    } catch (error) {
      session.agent.messages = messagesBeforeTurn
      updateMessages(messagesBeforeTurn)
      throw error
    }
  }

  async deleteSession(
    conversationId: string,
  ): Promise<{ deleted: boolean; sessionCount: number }> {
    const runDeleted = await this.conversationRuns.delete(conversationId)
    let acpDeleted = false
    const acpAgentId = this.acpConversationAgents.get(conversationId)
    if (acpAgentId) {
      await this.getAcpRuntime().close(acpAgentId, conversationId, {
        discardPersistentState: true,
      })
      this.acpConversationAgents.delete(conversationId)
      this.acpMessages.delete(`${acpAgentId}:${conversationId}`)
      this.acpToolLeases.get(conversationId)?.lease.revoke()
      this.acpToolLeases.delete(conversationId)
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
      deleted: deleted || acpDeleted || runDeleted,
      sessionCount: this.deps.sessionStore.count(),
    }
  }

  isAcpSession(conversationId: string): boolean {
    return this.acpConversationAgents.has(conversationId)
  }

  private async processAcpMessage(
    request: AcpChatRequest,
    abortSignal: AbortSignal,
    updateMessages: (messages: UIMessage[]) => boolean,
  ): Promise<ReadableStream<UIMessageChunk>> {
    const agent = await this.getAcpAgentStore().get(request.target.agentId)
    if (!agent) {
      throw new ChatRequestError(
        Response.json({ error: 'Unknown agent' }, { status: 404 }),
      )
    }
    if (agent.type !== request.target.type) {
      throw new ChatRequestError(
        Response.json({ error: 'Agent type mismatch' }, { status: 400 }),
      )
    }

    const browserContext = await resolveBrowserContextPageIds(
      this.deps.browser,
      request.browserContext,
    )
    const acpToolLease = this.getOrCreateAcpToolLease(
      agent.id,
      request,
      browserContext,
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
      browserToolLeaseToken: acpToolLease.token,
      readOnly: request.mode === 'chat',
      messages: promptMessages,
      browserContext,
      abortSignal,
      onFinish: async ({ messages }) => {
        const existingIds = new Set(history.map((message) => message.id))
        history.push(
          ...messages.filter((message) => !existingIds.has(message.id)),
        )
        updateMessages(history)
        await this.persistConversation({
          id: request.conversationId,
          messages: history,
          targetType: agent.type,
          origin: request.origin,
          agentId: agent.id,
        })
      },
    }
    // The run already authorizes this conversation before ACP stream setup;
    // publish display history before the persistent process can call MCP.
    updateMessages(history)
    let stream: ReadableStream<UIMessageChunk>
    try {
      stream = await this.getAcpRuntime().stream(streamInput)
    } catch (error) {
      history.length = priorHistoryLength
      updateMessages(history)
      if (error instanceof AcpAgentSessionBusyError) {
        throw new ChatRequestError(
          Response.json(
            { error: 'An agent turn is already running' },
            { status: 409 },
          ),
        )
      }
      if (!(error instanceof AcpAgentPreparationError)) throw error
      stream = createUIMessageStream({
        execute({ writer }) {
          writer.write({ type: 'error', errorText: error.message })
        },
      })
    }
    return stream
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

  private getOrCreateAcpToolLease(
    agentId: string,
    request: AcpChatRequest,
    browserContext: BrowserContext | undefined,
  ): BrowserToolLease {
    const readOnly = request.mode === 'chat'
    const fingerprint = JSON.stringify({ agentId, readOnly, browserContext })
    const existing = this.acpToolLeases.get(request.conversationId)
    if (existing?.fingerprint === fingerprint) return existing.lease

    existing?.lease.revoke()
    const outputFileAccess =
      existing?.outputFileAccess ?? createBrowserOutputFileAccess()
    const lease = this.deps.browserMcp.createLease({
      conversationId: request.conversationId,
      readOnly,
      outputFileAccess,
      browserContext,
      source: 'acp',
    })
    this.acpToolLeases.set(request.conversationId, {
      agentId,
      fingerprint,
      lease,
      outputFileAccess,
    })
    return lease
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
    // The old capability stops authorizing calls before the replacement agent
    // performs any asynchronous MCP cleanup.
    session.browserToolLease.revoke()
    this.deps.sessionStore.remove(request.conversationId)
    await session.agent.dispose()

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
    const { agent, browserToolLease } = await this.createBrowserOsAgent(
      agentConfig,
      browserContext,
      outputFileAccess,
    )
    const newSession: AgentSession = {
      agent,
      browserToolLease,
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

  /** Mints the capability before constructing the loopback MCP client. */
  private async createBrowserOsAgent(
    agentConfig: ResolvedAgentConfig,
    browserContext: BrowserContext | undefined,
    outputFileAccess: BrowserOutputFileAccess,
  ): Promise<{ agent: AiSdkAgent; browserToolLease: BrowserToolLease }> {
    const browserToolLease = this.deps.browserMcp.createLease({
      conversationId: agentConfig.conversationId,
      readOnly: agentConfig.chatMode ?? false,
      outputFileAccess,
      browserContext,
      source: 'chat',
    })
    try {
      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        serverPort: this.deps.serverPort,
        browserToolLeaseToken: browserToolLease.token,
        browserContext,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
        outputFileAccess,
      })
      return { agent, browserToolLease }
    } catch (error) {
      browserToolLease.revoke()
      throw error
    }
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

/** Carries an intentional HTTP result through the run's stream factory. */
class ChatRequestError extends Error {
  constructor(readonly response: Response) {
    super(`Chat request failed with status ${response.status}`)
    this.name = 'ChatRequestError'
  }
}

function browserContextTabIds(browserContext?: BrowserContext): number[] {
  if (!browserContext) return []
  const tabIds = new Set<number>()
  if (browserContext.activeTab) tabIds.add(browserContext.activeTab.id)
  for (const tab of browserContext.selectedTabs ?? []) tabIds.add(tab.id)
  for (const tab of browserContext.tabs ?? []) tabIds.add(tab.id)
  return [...tabIds]
}

function conversationTabGroup(
  request: ChatRequest,
): ConversationTabGroupPresentation {
  const words = request.message.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []
  const task = words.slice(0, 5).join('-').slice(0, 40) || 'task'
  const prefix = request.target.type
  const colorKey =
    request.target.type === 'browseros'
      ? 'browseros'
      : `${request.target.type}:${request.target.agentId}`
  return { title: `${prefix}/${task}`, colorKey }
}
