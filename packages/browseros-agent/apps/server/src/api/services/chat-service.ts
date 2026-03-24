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

const NEWTAB_EXECUTION_URL = 'about:blank'
const NEWTAB_EXECUTION_TITLE = 'New Tab Task'

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
    const resolvedRequestContext = request.isScheduledTask
      ? request.browserContext
      : await this.resolvePageIds(request.browserContext)

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
      accountId: llmConfig.accountId,
      reasoningEffort: request.reasoningEffort,
      reasoningSummary: request.reasoningSummary,
      contextWindowSize: request.contextWindowSize,
      userSystemPrompt: request.userSystemPrompt,
      workingDir,
      supportsImages: request.supportsImages,
      chatMode: request.mode === 'chat',
      isScheduledTask: request.isScheduledTask,
      declinedApps: request.declinedApps,
      browserosId: this.deps.browserosId,
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

      const browserContext = await this.resolveSessionBrowserContext(
        { ...request, source: session.source },
        resolvedRequestContext,
        session.browserContext,
      )
      const agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser: this.deps.browser,
        registry: this.deps.registry,
        browserContext,
        klavisClient: this.deps.klavisClient,
        browserosId: this.deps.browserosId,
        aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
      })
      session = {
        agent,
        source: session.source,
        hiddenWindowId: session.hiddenWindowId,
        browserContext,
        mcpServerKey,
      }
      session.agent.messages = previousMessages
      sessionStore.set(request.conversationId, session)
    }

    if (!session) {
      isNewSession = true
      let hiddenWindowId: number | undefined
      let browserContext = await this.resolveSessionBrowserContext(
        request,
        resolvedRequestContext,
      )
      if (request.isScheduledTask) {
        try {
          const win = await this.deps.browser.createWindow({
            hidden: true,
            url: NEWTAB_EXECUTION_URL,
          })
          hiddenWindowId = win.windowId
          const target = await this.getOrCreateWindowTarget(
            win.windowId,
            win.activeTabId,
          )
          browserContext = {
            ...browserContext,
            windowId: hiddenWindowId,
            activeTab: {
              id: target.tabId,
              pageId: target.pageId,
              url: NEWTAB_EXECUTION_URL,
              title: 'Scheduled Task',
            },
          }
          logger.info('Created hidden window for scheduled task', {
            conversationId: request.conversationId,
            windowId: hiddenWindowId,
            pageId: target.pageId,
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
      session = {
        agent,
        source: request.source,
        hiddenWindowId,
        browserContext,
        mcpServerKey,
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

    const resolvedMessageContext = this.buildMessageBrowserContext(
      { ...request, source: session.source },
      resolvedRequestContext,
      session.browserContext,
    )
    const userContent = formatUserMessage(
      request.message,
      resolvedMessageContext,
      request.selectedText,
      request.selectedTextSource,
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

  private buildMessageBrowserContext(
    request: ChatRequest,
    requestContext: BrowserContext | undefined,
    sessionContext?: BrowserContext,
  ): BrowserContext | undefined {
    if (request.isScheduledTask) {
      return sessionContext ?? requestContext
    }
    if (request.source === 'newtab') {
      return this.mergeExecutionContext(sessionContext, requestContext)
    }
    return requestContext
  }

  private async resolveSessionBrowserContext(
    request: ChatRequest,
    requestContext: BrowserContext | undefined,
    existingSessionContext?: BrowserContext,
  ): Promise<BrowserContext | undefined> {
    if (request.isScheduledTask) {
      return existingSessionContext ?? requestContext
    }
    if (request.source !== 'newtab') {
      return requestContext
    }
    if (existingSessionContext) {
      return this.mergeExecutionContext(existingSessionContext, requestContext)
    }
    return this.createNewTabExecutionContext(requestContext)
  }

  private mergeExecutionContext(
    executionContext?: BrowserContext,
    requestContext?: BrowserContext,
  ): BrowserContext | undefined {
    if (!executionContext) return requestContext
    if (!requestContext) return executionContext
    return {
      ...requestContext,
      windowId: executionContext.windowId ?? requestContext.windowId,
      activeTab: executionContext.activeTab ?? requestContext.activeTab,
    }
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

  private async createNewTabExecutionContext(
    requestContext?: BrowserContext,
  ): Promise<BrowserContext | undefined> {
    const previousWindowId =
      requestContext?.windowId ??
      (await this.deps.browser.getActivePage())?.windowId
    const win = await this.deps.browser.createWindow({
      url: NEWTAB_EXECUTION_URL,
    })
    const target = await this.getOrCreateWindowTarget(
      win.windowId,
      win.activeTabId,
    )

    if (previousWindowId !== undefined && previousWindowId !== win.windowId) {
      await this.deps.browser
        .activateWindow(previousWindowId)
        .catch((error) => {
          logger.warn('Failed to restore previous window focus', {
            previousWindowId,
            newWindowId: win.windowId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
    }

    logger.info('Created execution window for new-tab chat', {
      windowId: win.windowId,
      pageId: target.pageId,
      previousWindowId,
    })

    return {
      ...requestContext,
      windowId: win.windowId,
      activeTab: {
        id: target.tabId,
        pageId: target.pageId,
        url: NEWTAB_EXECUTION_URL,
        title: NEWTAB_EXECUTION_TITLE,
      },
    }
  }

  private async getOrCreateWindowTarget(
    windowId: number,
    tabId?: number,
  ): Promise<{ pageId: number; tabId: number }> {
    if (tabId !== undefined) {
      const pageId = (await this.deps.browser.resolveTabIds([tabId])).get(tabId)
      if (pageId !== undefined) {
        return { pageId, tabId }
      }
    }

    const pageId = await this.deps.browser.newPage(NEWTAB_EXECUTION_URL, {
      windowId,
      background: true,
    })
    const resolvedTabId = this.deps.browser.getTabIdForPage(pageId)
    if (resolvedTabId === undefined) {
      throw new Error(`Could not resolve tab ID for page ${pageId}`)
    }
    return { pageId, tabId: resolvedTabId }
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
