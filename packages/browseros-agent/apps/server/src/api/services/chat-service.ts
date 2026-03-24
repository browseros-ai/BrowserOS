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
import type { AgentSession, SessionStore } from '../../agent/session-store'
import type { ResolvedAgentConfig } from '../../agent/types'
import type { Browser } from '../../browser/browser'
import { getSessionsDir } from '../../lib/browseros-dir'
import type { KlavisClient } from '../../lib/clients/klavis/klavis-client'
import { resolveLLMConfig } from '../../lib/clients/llm/config'
import { logger } from '../../lib/logger'
import type { ToolRegistry } from '../../tools/tool-registry'
import type { BrowserContext, ChatRequest } from '../types'

const NEWTAB_EXECUTION_URL = 'chrome://newtab'
const NEWTAB_EXECUTION_TITLE = 'New Tab'

interface PreparedRequest {
  agentConfig: ResolvedAgentConfig
  mcpServerKey: string
  resolvedRequestContext?: BrowserContext
}

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
    const prepared = await this.prepareRequest(request)
    const { session, isNewSession } = await this.getOrCreateSession(
      request,
      prepared,
    )

    this.injectPreviousConversation(request, session, isNewSession)

    const resolvedMessageContext = this.buildMessageBrowserContext(
      { ...request, source: session.source },
      prepared.resolvedRequestContext,
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
        this.finalizeStream(request.conversationId, session, messages)
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

  private async prepareRequest(request: ChatRequest): Promise<PreparedRequest> {
    const llmConfig = await resolveLLMConfig(request, this.deps.browserosId)
    const resolvedRequestContext = request.isScheduledTask
      ? request.browserContext
      : await this.resolvePageIds(request.browserContext)
    const workingDir = await this.resolveSessionDir(request)

    return {
      agentConfig: {
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
      },
      mcpServerKey: this.buildMcpServerKey(request.browserContext),
      resolvedRequestContext,
    }
  }

  private async getOrCreateSession(
    request: ChatRequest,
    prepared: PreparedRequest,
  ): Promise<{ session: AgentSession; isNewSession: boolean }> {
    let session = this.deps.sessionStore.get(request.conversationId)
    if (session && session.mcpServerKey !== prepared.mcpServerKey) {
      session = await this.rebuildSession(request, prepared, session)
    }
    if (session) {
      return { session, isNewSession: false }
    }

    return {
      session: await this.createSession(request, prepared),
      isNewSession: true,
    }
  }

  private async rebuildSession(
    request: ChatRequest,
    prepared: PreparedRequest,
    session: AgentSession,
  ): Promise<AgentSession> {
    logger.info('MCP servers changed mid-conversation, rebuilding session', {
      conversationId: request.conversationId,
      previous: session.mcpServerKey,
      current: prepared.mcpServerKey,
    })

    const previousMessages = session.agent.messages
    await session.agent.dispose()
    this.deps.sessionStore.remove(request.conversationId)

    const browserContext = await this.resolveSessionBrowserContext(
      { ...request, source: session.source },
      prepared.resolvedRequestContext,
      session.browserContext,
    )
    const nextSession: AgentSession = {
      agent: await this.createAgent(prepared.agentConfig, browserContext),
      source: session.source,
      hiddenWindowId: session.hiddenWindowId,
      browserContext,
      mcpServerKey: prepared.mcpServerKey,
    }
    nextSession.agent.messages = previousMessages
    this.deps.sessionStore.set(request.conversationId, nextSession)
    return nextSession
  }

  private async createSession(
    request: ChatRequest,
    prepared: PreparedRequest,
  ): Promise<AgentSession> {
    let hiddenWindowId: number | undefined
    let browserContext = await this.resolveSessionBrowserContext(
      request,
      prepared.resolvedRequestContext,
    )

    if (request.isScheduledTask) {
      const scheduledTaskContext = await this.attachScheduledTaskWindow(
        request,
        browserContext,
      )
      browserContext = scheduledTaskContext.browserContext
      hiddenWindowId = scheduledTaskContext.hiddenWindowId
    }

    const session: AgentSession = {
      agent: await this.createAgent(prepared.agentConfig, browserContext),
      source: request.source,
      hiddenWindowId,
      browserContext,
      mcpServerKey: prepared.mcpServerKey,
    }
    this.deps.sessionStore.set(request.conversationId, session)
    return session
  }

  private async createAgent(
    resolvedConfig: ResolvedAgentConfig,
    browserContext?: BrowserContext,
  ) {
    return AiSdkAgent.create({
      resolvedConfig,
      browser: this.deps.browser,
      registry: this.deps.registry,
      browserContext,
      klavisClient: this.deps.klavisClient,
      browserosId: this.deps.browserosId,
      aiSdkDevtoolsEnabled: this.deps.aiSdkDevtoolsEnabled,
    })
  }

  private async attachScheduledTaskWindow(
    request: ChatRequest,
    browserContext?: BrowserContext,
  ): Promise<{
    browserContext?: BrowserContext
    hiddenWindowId?: number
  }> {
    try {
      const win = await this.deps.browser.createWindow({
        hidden: true,
        url: NEWTAB_EXECUTION_URL,
      })
      const target = await this.getOrCreateWindowTarget(
        win.windowId,
        win.activeTabId,
      )

      logger.info('Created hidden window for scheduled task', {
        conversationId: request.conversationId,
        windowId: win.windowId,
        pageId: target.pageId,
      })

      return {
        hiddenWindowId: win.windowId,
        browserContext: {
          ...browserContext,
          windowId: win.windowId,
          activeTab: {
            id: target.tabId,
            pageId: target.pageId,
            url: NEWTAB_EXECUTION_URL,
            title: 'Scheduled Task',
          },
        },
      }
    } catch (error) {
      logger.warn('Failed to create hidden window, using default', {
        error: error instanceof Error ? error.message : String(error),
      })
      return { browserContext }
    }
  }

  private injectPreviousConversation(
    request: ChatRequest,
    session: AgentSession,
    isNewSession: boolean,
  ): void {
    if (!isNewSession || !request.previousConversation?.length) {
      return
    }

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

  private finalizeStream(
    conversationId: string,
    session: AgentSession,
    messages: UIMessage[],
  ): void {
    session.agent.messages = filterValidMessages(messages)
    logger.info('Agent execution complete', {
      conversationId,
      totalMessages: messages.length,
    })

    if (!session.hiddenWindowId) return

    const windowId = session.hiddenWindowId
    session.hiddenWindowId = undefined
    this.closeHiddenWindow(windowId, conversationId)
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
    const windowId =
      requestContext?.windowId ??
      (await this.deps.browser.getActivePage())?.windowId
    const target = await this.createBackgroundExecutionPage(windowId)

    logger.info('Created execution page for new-tab chat', {
      windowId,
      pageId: target.pageId,
    })

    return {
      ...requestContext,
      windowId,
      activeTab: {
        id: target.tabId,
        pageId: target.pageId,
        url: NEWTAB_EXECUTION_URL,
        title: NEWTAB_EXECUTION_TITLE,
      },
    }
  }

  private async createBackgroundExecutionPage(
    windowId?: number,
  ): Promise<{ pageId: number; tabId: number }> {
    const pageId = await this.deps.browser.newPage(NEWTAB_EXECUTION_URL, {
      background: true,
      ...(windowId !== undefined && { windowId }),
    })
    const tabId = this.deps.browser.getTabIdForPage(pageId)
    if (tabId === undefined) {
      throw new Error(`Could not resolve tab ID for page ${pageId}`)
    }
    return { pageId, tabId }
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
