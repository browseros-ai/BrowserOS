/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { mkdir, utimes } from 'node:fs/promises'
import path from 'node:path'
import { createAgentUIStreamResponse, type UIMessage } from 'ai'
import { AiSdkAgent } from '../../agent/ai-sdk-agent'
import { formatUserMessage } from '../../agent/format-message'
import { filterValidMessages } from '../../agent/message-validation'
import type { SessionStore } from '../../agent/session-store'
import type { ResolvedAgentConfig } from '../../agent/types'
import type { Browser } from '../../browser/browser'
import { getSessionsDir } from '../../lib/browseros-dir'
import type { KlavisClient } from '../../lib/clients/klavis/klavis-client'
import { resolveLLMConfig } from '../../lib/clients/llm/config'
import { logger } from '../../lib/logger'
import type { ToolRegistry } from '../../tools/tool-registry'
import type { BrowserContext, ChatRequest } from '../types'

export interface ChatServiceDeps {
  sessionStore: SessionStore
  klavisClient: KlavisClient
  browser: Browser
  registry: ToolRegistry
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
}

export class ChatService {
  constructor(private deps: ChatServiceDeps) {}

  async processMessage(
    request: ChatRequest,
    abortSignal: AbortSignal,
  ): Promise<Response> {
    const { sessionStore } = this.deps

    const llmConfig = await resolveLLMConfig(request, this.deps.browserosId)

    const workingDir = await this.resolveSessionDir(request)

    const agentConfig: ResolvedAgentConfig = {
      conversationId: request.conversationId,
      provider: llmConfig.provider,
      model: llmConfig.model,
      apiKey: llmConfig.apiKey,
      baseUrl: llmConfig.baseUrl,
      upstreamProvider: llmConfig.upstreamProvider,
      resourceName: llmConfig.resourceName,
      region: llmConfig.region,
      accessKeyId: llmConfig.accessKeyId,
      secretAccessKey: llmConfig.secretAccessKey,
      sessionToken: llmConfig.sessionToken,
      contextWindowSize: request.contextWindowSize,
      userSystemPrompt: request.userSystemPrompt,
      workingDir,
      supportsImages: request.supportsImages,
      chatMode: request.mode === 'chat',
      isScheduledTask: request.isScheduledTask,
      declinedApps: request.declinedApps,
    }

    let session = sessionStore.get(request.conversationId)
    let isNewSession = false

    // Build a stable key from enabled MCP servers for change detection
    const mcpServerKey = this.buildMcpServerKey(request.browserContext)

    // Detect MCP config change mid-conversation → rebuild session
    if (session && session.mcpServerKey !== mcpServerKey) {
      logger.info('MCP servers changed mid-conversation, rebuilding session', {
        conversationId: request.conversationId,
        previous: session.mcpServerKey,
        current: mcpServerKey,
      })
      const previousMessages = session.agent.messages
      await session.agent.dispose()
      sessionStore.remove(request.conversationId)

      const browserContext = await this.resolvePageIds(request.browserContext)
      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser: this.deps.browser,
        registry: this.deps.registry,
        browserContext,
        klavisClient: this.deps.klavisClient,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
      })
      session = { agent, browserContext, mcpServerKey }
      session.agent.messages = previousMessages
      sessionStore.set(request.conversationId, session)
    }

    if (!session) {
      isNewSession = true
      let hiddenWindowId: number | undefined
      let browserContext = await this.resolvePageIds(request.browserContext)
      if (request.isScheduledTask) {
        try {
          const win = await this.deps.browser.createWindow({ hidden: true })
          hiddenWindowId = win.windowId
          const pageId = await this.deps.browser.newPage('about:blank', {
            windowId: hiddenWindowId,
          })
          browserContext = {
            ...browserContext,
            windowId: hiddenWindowId,
            activeTab: {
              id: pageId,
              pageId,
              url: 'about:blank',
              title: 'Scheduled Task',
            },
          }
          logger.info('Created hidden window for scheduled task', {
            conversationId: request.conversationId,
            windowId: hiddenWindowId,
            pageId,
          })
        } catch (error) {
          logger.warn('Failed to create hidden window, using default', {
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }

      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser: this.deps.browser,
        registry: this.deps.registry,
        browserContext,
        klavisClient: this.deps.klavisClient,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
      })
      session = { agent, hiddenWindowId, browserContext, mcpServerKey }
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
    // calling resolvePageIds would pass those to resolveTabIds (which expects Chrome
    // tab IDs), corrupting them back to undefined.
    const resolvedMessageContext = request.isScheduledTask
      ? messageContext
      : await this.resolvePageIds(messageContext)
    const userContent = formatUserMessage(
      request.message,
      resolvedMessageContext,
    )
    session.agent.appendUserMessage(userContent)

    return createAgentUIStreamResponse({
      agent: session.agent.toolLoopAgent,
      uiMessages: filterValidMessages(session.agent.messages),
      abortSignal,
      onFinish: async ({ messages }: { messages: UIMessage[] }) => {
        session.agent.messages = filterValidMessages(messages)
        logger.info('Agent execution complete', {
          conversationId: request.conversationId,
          totalMessages: messages.length,
        })

        if (session?.hiddenWindowId) {
          const windowId = session.hiddenWindowId
          session.hiddenWindowId = undefined
          this.closeHiddenWindow(windowId, request.conversationId)
        }
      },
    })
  }

  async deleteSession(
    conversationId: string,
  ): Promise<{ deleted: boolean; sessionCount: number }> {
    const session = this.deps.sessionStore.get(conversationId)
    if (session?.hiddenWindowId) {
      const windowId = session.hiddenWindowId
      session.hiddenWindowId = undefined
      this.closeHiddenWindow(windowId, conversationId)
    }
    const deleted = await this.deps.sessionStore.delete(conversationId)
    return { deleted, sessionCount: this.deps.sessionStore.count() }
  }

  // Browser context arrives with Chrome tab IDs, but tools expect internal page IDs.
  // Resolve the mapping upfront so the agent's first navigation doesn't fail.
  private async resolvePageIds(
    browserContext?: BrowserContext,
  ): Promise<BrowserContext | undefined> {
    if (!browserContext) return undefined

    const tabIdSet = new Set<number>()
    if (browserContext.activeTab) tabIdSet.add(browserContext.activeTab.id)
    if (browserContext.selectedTabs) {
      for (const tab of browserContext.selectedTabs) tabIdSet.add(tab.id)
    }
    if (browserContext.tabs) {
      for (const tab of browserContext.tabs) tabIdSet.add(tab.id)
    }

    if (tabIdSet.size === 0) return browserContext

    const tabToPage = await this.deps.browser.resolveTabIds([...tabIdSet])

    const addPageId = (tab: { id: number; url?: string; title?: string }) => {
      const pageId = tabToPage.get(tab.id)
      if (pageId === undefined) {
        logger.warn('Could not resolve page ID for tab', { tabId: tab.id })
      }
      return { ...tab, pageId }
    }

    logger.debug('Resolved tab IDs to page IDs', {
      mapping: Object.fromEntries(tabToPage),
    })

    return {
      ...browserContext,
      activeTab: browserContext.activeTab
        ? addPageId(browserContext.activeTab)
        : undefined,
      selectedTabs: browserContext.selectedTabs?.map(addPageId),
      tabs: browserContext.tabs?.map(addPageId),
    }
  }

  private closeHiddenWindow(windowId: number, conversationId: string): void {
    this.deps.browser.closeWindow(windowId).catch((error) => {
      logger.warn('Failed to close hidden window', {
        windowId,
        conversationId,
        error: error instanceof Error ? error.message : String(error),
      })
    })
  }

  private buildMcpServerKey(browserContext?: BrowserContext): string {
    const managed = browserContext?.enabledMcpServers?.slice().sort() ?? []
    const custom =
      browserContext?.customMcpServers?.map((s) => s.url).sort() ?? []
    return [...managed, ...custom].join(',')
  }

  private async resolveSessionDir(request: ChatRequest): Promise<string> {
    const dir = request.userWorkingDir
      ? request.userWorkingDir
      : path.join(getSessionsDir(), request.conversationId)
    await mkdir(dir, { recursive: true })
    if (!request.userWorkingDir) {
      const now = new Date()
      await utimes(dir, now, now).catch(() => {})
    }
    return dir
  }
}
