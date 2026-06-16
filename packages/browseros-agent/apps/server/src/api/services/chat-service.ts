/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createAgentUIStreamResponse, type UIMessage } from 'ai'
import { isAcpProvider } from '../../agent/acp-providers'
import { AiSdkAgent } from '../../agent/ai-sdk-agent'
import { formatUserMessage } from '../../agent/format-message'
import {
  filterValidMessages,
  sanitizeMessagesForToolset,
} from '../../agent/message-validation'
import type { AgentSession, SessionStore } from '../../agent/session-store'
import type { ResolvedAgentConfig } from '../../agent/types'
import type { Browser } from '../../browser/browser'
import type { BrowserSession } from '../../browser/core/session'
import { buildAcpMcpServers } from '../../lib/agents/acpx-provider/buildAcpMcpServers'
import { resolveLLMConfig } from '../../lib/clients/llm/config'
import { logger } from '../../lib/logger'
import type { KlavisProxyRef } from '../services/klavis/strata-proxy'
import type { BrowserContext, ChatRequest } from '../types'
import { resolveBrowserContextPageIds } from '../utils/resolve-browser-context-page-ids'

export interface ChatServiceDeps {
  sessionStore: SessionStore
  klavisRef?: KlavisProxyRef
  browser: Browser
  browserSession: BrowserSession
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
  browserUseNewTools: boolean
  /** Port the BrowserOS server bound to. Forwarded into the ACP MCP
   *  bridge so the spawned agent can dial back into /mcp. */
  serverPort: number
  /** BrowserOS resources directory. Threaded into ACP-backed config
   *  resolutions so the bundled-Bun launcher under
   *  <resourcesDir>/bin/third_party/bun can be located. */
  resourcesDir?: string | null
}

export class ChatService {
  constructor(private deps: ChatServiceDeps) {}

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: chat request orchestration; refactor tracked separately
  async processMessage(
    request: ChatRequest,
    abortSignal: AbortSignal,
  ): Promise<Response> {
    const { sessionStore } = this.deps

    const llmConfig = await resolveLLMConfig(request, this.deps.browserosId)

    // Look up the session first so we can stamp isNewConversation onto
    // agentConfig before it flows down into the ACP factory (which uses
    // the flag to decide whether to refresh the workspace instruction
    // file). The original isNewSession flag below stays as-is for the
    // rest of the chat-service logic.
    let session = sessionStore.get(request.conversationId)
    const isFirstTurn = !session

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
      chatMode: request.mode === 'chat',
      isScheduledTask: request.isScheduledTask,
      origin: request.origin,
      declinedApps: request.declinedApps,
      browserosId: this.deps.browserosId,
      acpAgentId: request.acpAgentId,
      acpCommand: request.acpCommand,
      acpFixedWorkspacePath: request.acpFixedWorkspacePath,
      acpMcpServers: isAcpProvider(llmConfig.provider)
        ? buildAcpMcpServers({
            serverPort: this.deps.serverPort,
            conversationId: request.conversationId,
            providerId: llmConfig.provider,
            defaultWindowId: request.browserContext?.windowId,
            customMcpServers: request.browserContext?.customMcpServers,
          })
        : undefined,
      isNewConversation: isFirstTurn,
      resourcesDir: this.deps.resourcesDir,
    }

    let isNewSession = false
    const contextChanges: string[] = []

    // Build stable keys for change detection
    const mcpServerKey = this.buildMcpServerKey(request.browserContext)

    // Detect MCP config change mid-conversation → rebuild session
    if (session && session.mcpServerKey !== mcpServerKey) {
      logger.info('MCP servers changed mid-conversation, rebuilding session', {
        conversationId: request.conversationId,
        previous: session.mcpServerKey,
        current: mcpServerKey,
      })
      const previousMcpKey = session.mcpServerKey
      session = await this.rebuildSession(
        session,
        request,
        agentConfig,
        mcpServerKey,
      )

      const oldParts = (previousMcpKey ?? '').split(',').filter(Boolean)
      const newParts = mcpServerKey.split(',').filter(Boolean)
      const oldKlavisState = oldParts.find((s) => s.startsWith('klavis:'))
      const newKlavisState = newParts.find((s) => s.startsWith('klavis:'))
      const oldServers = new Set(
        oldParts.filter((s) => !s.startsWith('klavis:')),
      )
      const newServers = new Set(
        newParts.filter((s) => !s.startsWith('klavis:')),
      )
      const added = [...newServers].filter((s) => !oldServers.has(s))
      const removed = [...oldServers].filter((s) => !newServers.has(s))

      const parts: string[] = []
      if (removed.length > 0) {
        parts.push(
          `The following app integrations were disconnected: ${removed.join(', ')}. Their tools are no longer available.`,
        )
      }
      if (added.length > 0) {
        parts.push(
          `The following app integrations were connected: ${added.join(', ')}. Their tools are now available.`,
        )
      }
      if (parts.length === 0) {
        if (
          oldKlavisState === 'klavis:pending' &&
          newKlavisState === 'klavis:connected' &&
          newServers.size > 0
        ) {
          parts.push(
            `Klavis app integration tools are now available for the following connected apps: ${[...newServers].join(', ')}.`,
          )
        } else {
          parts.push(
            'Connected app integrations changed during this conversation. Use only tools that are currently registered.',
          )
        }
      }
      contextChanges.push(parts.join(' '))
    }

    // Detect workspace change mid-conversation → rebuild session
    if (session && session.workingDir !== request.userWorkingDir) {
      logger.info('Workspace changed mid-conversation, rebuilding session', {
        conversationId: request.conversationId,
        previous: session.workingDir ?? '(none)',
        current: request.userWorkingDir ?? '(none)',
      })
      const previousWorkingDir = session.workingDir
      session = await this.rebuildSession(
        session,
        request,
        agentConfig,
        mcpServerKey,
      )

      if (!request.userWorkingDir) {
        contextChanges.push(
          'The user disconnected the workspace during this conversation. Filesystem tools (filesystem_read, filesystem_write, filesystem_edit, filesystem_bash, filesystem_grep, filesystem_find, filesystem_ls) are no longer available. Return all output directly in chat. If the user asks for file operations, suggest they select a working directory from the chat toolbar.',
        )
      } else if (!previousWorkingDir) {
        contextChanges.push(
          `The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: ${request.userWorkingDir}`,
        )
      } else {
        contextChanges.push(
          `The user switched workspace during this conversation. Filesystem tools now use the new working directory: ${request.userWorkingDir}`,
        )
      }
    }

    if (!session) {
      isNewSession = true
      let hiddenPageId: number | undefined
      let browserContext = await resolveBrowserContextPageIds(
        this.deps.browser,
        request.browserContext,
      )
      if (request.isScheduledTask) {
        try {
          hiddenPageId = await this.deps.browser.newPage('about:blank', {
            hidden: true,
            background: true,
          })
          let hiddenWindowId: number | undefined
          try {
            const hiddenPage = (await this.deps.browser.listPages()).find(
              (page) => page.pageId === hiddenPageId,
            )
            hiddenWindowId = hiddenPage?.windowId
          } catch (error) {
            logger.warn('Failed to look up hidden page metadata', {
              conversationId: request.conversationId,
              pageId: hiddenPageId,
              error: error instanceof Error ? error.message : String(error),
            })
          }
          browserContext = {
            ...browserContext,
            windowId: hiddenWindowId,
            selectedTabs: undefined,
            tabs: undefined,
            activeTab: {
              id: hiddenPageId,
              pageId: hiddenPageId,
              url: 'about:blank',
              title: 'Scheduled Task',
            },
          }
          logger.info('Created hidden page for scheduled task', {
            conversationId: request.conversationId,
            pageId: hiddenPageId,
            windowId: hiddenWindowId,
          })
        } catch (error) {
          logger.warn(
            'Failed to create hidden page, using default browser context',
            {
              error: error instanceof Error ? error.message : String(error),
            },
          )
        }
      }

      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser: this.deps.browser,
        browserSession: this.deps.browserSession,
        browserContext,
        klavisRef: this.deps.klavisRef,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
        browserUseNewTools: this.deps.browserUseNewTools,
      })
      session = {
        agent,
        hiddenPageId,
        browserContext,
        mcpServerKey,
        workingDir: request.userWorkingDir,
      }
      sessionStore.set(request.conversationId, session)
    }

    if (isNewSession && request.previousConversation?.length) {
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

    // Prepend tool-change context when session was rebuilt mid-conversation
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

    // ACP-backed providers run against a persistent acpx session that
    // owns the agent's conversation memory natively on disk under
    // <stateDir>/<sessionKey>/. Re-feeding the full UIMessage history
    // doubles bookkeeping and, worse, trips the AI SDK validator when
    // it walks phantom tool-<name> parts emitted by acpx-ai-provider
    // under freshly-generated "acpx-N" ids (acpx#37). For ACP turns
    // we send only the new user message — acpx's session/load reads
    // prior turns from disk transparently. The UI continues to see
    // the growing transcript via session.agent.messages.
    //
    // LLM-API providers are stateless and need the full history on
    // each turn, so they keep the existing shape verbatim.
    const isAcp = isAcpProvider(agentConfig.provider)
    const promptUiMessages: UIMessage[] = isAcp
      ? [
          {
            id: wrappedUserMessageId ?? crypto.randomUUID(),
            role: 'user',
            parts: [{ type: 'text', text: promptUserText }],
          },
        ]
      : filterValidMessages(session.agent.messages).map((msg) =>
          msg.id === wrappedUserMessageId && msg.role === 'user'
            ? {
                ...msg,
                parts: [{ type: 'text' as const, text: promptUserText }],
              }
            : msg,
        )

    return createAgentUIStreamResponse({
      agent: session.agent.toolLoopAgent,
      uiMessages: promptUiMessages,
      abortSignal,
      onFinish: async ({ messages }: { messages: UIMessage[] }) => {
        // The agent loop returns `messages` containing the prompt-
        // wrapped user text. Restore the raw form before persisting
        // so subsequent turns see the clean text and the client's
        // local UIMessage matches what was originally typed.
        //
        // ACP path: `messages` is the single user msg we sent plus
        // the assistant's new reply. The user msg already lives in
        // session.agent.messages via appendUserMessage; we only need
        // to restore its raw text and append the new assistant
        // entries from this turn.
        //
        // LLM-API path: `messages` is the full conversation as the
        // AI SDK reconstructed it. Restore the wrapped user message
        // and replace the entire session history with the result.
        if (isAcp) {
          // Invariant: an id in both `messages` and session means the
          // AI SDK handed us back something we already have. With the
          // single-user-msg input shape that means our own user msg —
          // the only collision we expect. Any new id is a fresh
          // assistant entry from this turn. acpx never re-emits prior
          // turns into the AI SDK stream, so this filter cannot drop a
          // legitimately new message.
          const existingIds = new Set(session.agent.messages.map((m) => m.id))
          const newMessages = messages.filter((m) => !existingIds.has(m.id))
          const updated = session.agent.messages.map((m) =>
            m.id === wrappedUserMessageId && m.role === 'user'
              ? {
                  ...m,
                  parts: [{ type: 'text' as const, text: request.message }],
                }
              : m,
          )
          session.agent.messages = filterValidMessages([
            ...updated,
            ...newMessages,
          ])
        } else {
          const restored = messages.map((msg) =>
            msg.id === wrappedUserMessageId && msg.role === 'user'
              ? {
                  ...msg,
                  parts: [{ type: 'text' as const, text: request.message }],
                }
              : msg,
          )
          session.agent.messages = filterValidMessages(restored)
        }
        logger.info('Agent execution complete', {
          conversationId: request.conversationId,
          totalMessages: session.agent.messages.length,
        })

        if (session?.hiddenPageId) {
          const pageId = session.hiddenPageId
          session.hiddenPageId = undefined
          this.closeHiddenPage(pageId, request.conversationId)
        }
      },
    })
  }

  async deleteSession(
    conversationId: string,
  ): Promise<{ deleted: boolean; sessionCount: number }> {
    const session = this.deps.sessionStore.get(conversationId)
    if (session?.hiddenPageId) {
      const pageId = session.hiddenPageId
      session.hiddenPageId = undefined
      this.closeHiddenPage(pageId, conversationId)
    }
    const deleted = await this.deps.sessionStore.delete(conversationId)
    return { deleted, sessionCount: this.deps.sessionStore.count() }
  }

  private closeHiddenPage(pageId: number, conversationId: string): void {
    this.deps.browser.closePage(pageId).catch((error) => {
      logger.warn('Failed to close hidden page', {
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
    const agent = await AiSdkAgent.create({
      resolvedConfig: agentConfig,
      browser: this.deps.browser,
      browserSession: this.deps.browserSession,
      browserContext,
      klavisRef: this.deps.klavisRef,
      browserosId: this.deps.browserosId,
      aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
      browserUseNewTools: this.deps.browserUseNewTools,
    })
    const newSession: AgentSession = {
      agent,
      hiddenPageId: session.hiddenPageId,
      browserContext,
      mcpServerKey,
      workingDir: request.userWorkingDir,
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
        ? this.deps.klavisRef?.handle
          ? 'klavis:connected'
          : 'klavis:pending'
        : null
    return [klavisState, ...managed, ...custom].filter(Boolean).join(',')
  }
}
