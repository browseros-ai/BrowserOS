import { beforeEach, describe, expect, it, mock } from 'bun:test'
import * as _ai from 'ai'
import type { KlavisProxyStatus } from '../../../src/api/services/klavis'

interface MockMessage {
  id: string
  role: 'user' | 'assistant'
  parts: Array<{ type: 'text'; text: string }>
}

interface MockAgent {
  toolLoopAgent: object
  toolNames: Set<string>
  messages: MockMessage[]
  appendUserMessage(text: string): void
  dispose(): Promise<void>
}

interface StoredSession {
  agent: MockAgent
  hiddenPageId?: number
}

interface StreamResponseOptions {
  uiMessages?: MockMessage[]
  abortSignal?: AbortSignal
  onFinish(args: { messages: MockMessage[] }): Promise<void>
}

let agentToReturn: MockAgent | undefined
let streamResponseHandler:
  | ((options: StreamResponseOptions) => Promise<Response>)
  | undefined

const createAgentSpy = mock(async (config: unknown) => {
  if (!agentToReturn) {
    throw new Error(`No mock agent configured for ${JSON.stringify(config)}`)
  }
  return agentToReturn
})

const createAgentUIStreamResponseSpy = mock(
  async (options: StreamResponseOptions) => {
    if (!streamResponseHandler) {
      throw new Error('No stream response handler configured')
    }
    return await streamResponseHandler(options)
  },
)

const resolveLLMConfigSpy = mock(async () => ({
  provider: 'openai',
  model: 'gpt-5',
  apiKey: 'test-key',
}))

// Spread the real `ai` module so other test files in the same
// bun-test process that import { tool } / { UIMessage } / etc. from
// `ai` still get real exports. Without the spread, this partial mock
// wipes the `ai` module in Bun's process-scoped mock registry and
// unrelated files blow up at load with `SyntaxError: Export named
// 'tool' not found in module .../ai/dist/index.mjs`. Reproducible on
// Linux CI's file-load order but benign on macOS APFS. See the
// 2026-07-17 test reliability audit for the failure mechanism.
mock.module('ai', () => ({
  ..._ai,
  createAgentUIStreamResponse: createAgentUIStreamResponseSpy,
}))

mock.module('../../../src/agent/ai-sdk-agent', () => ({
  AiSdkAgent: {
    create: createAgentSpy,
  },
}))

mock.module('../../../src/lib/clients/llm/config', () => ({
  resolveLLMConfig: resolveLLMConfigSpy,
}))

mock.module('../../../src/lib/logger', () => ({
  logger: {
    info: mock(() => {}),
    warn: mock(() => {}),
    debug: mock(() => {}),
  },
}))

const { ChatService } = await import('../../../src/api/services/chat-service')
const { ServerActivity } = await import(
  '../../../src/api/services/server-activity'
)
const { TurnRegistry } = await import(
  '../../../src/lib/agents/turns/active-turn-registry'
)

function createKlavisStub(
  getStatus: () => KlavisProxyStatus = () => ({
    state: 'stopped',
  }),
) {
  return {
    getProxyStatus: getStatus,
    buildAiSdkToolSet: mock(() => ({})),
    registerMcpTools: mock(() => {}),
  }
}

function createSessionStore() {
  const sessions = new Map<string, StoredSession>()
  return {
    get(conversationId: string) {
      return sessions.get(conversationId)
    },
    set(conversationId: string, session: StoredSession) {
      sessions.set(conversationId, session)
    },
    remove(conversationId: string) {
      return sessions.delete(conversationId)
    },
    async delete(conversationId: string) {
      const session = sessions.get(conversationId)
      if (!session) return false
      await session.agent.dispose()
      sessions.delete(conversationId)
      return true
    },
    count() {
      return sessions.size
    },
  }
}

function createFakeAgent() {
  const messages: MockMessage[] = []
  return {
    toolLoopAgent: {},
    toolNames: new Set<string>(),
    messages,
    appendUserMessage(text: string) {
      // Mirror production's id-per-call: a hardcoded constant would
      // collide on repeat calls in the same agent instance and corrupt
      // the id-diff logic the ACP onFinish branch relies on.
      this.messages.push({
        id: crypto.randomUUID(),
        role: 'user',
        parts: [{ type: 'text', text }],
      })
    },
    dispose: mock(async () => {}),
  }
}

describe('ChatService activity tracking', () => {
  it('returns to idle when a chat stream is aborted mid-response', async () => {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'test-key',
    }))
    const fakeAgent = createFakeAgent()
    agentToReturn = fakeAgent
    const abortController = new AbortController()
    streamResponseHandler = async () => {
      const encoder = new TextEncoder()
      return new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode('data: partial\n\n'))
            abortController.signal.addEventListener(
              'abort',
              () => controller.close(),
              { once: true },
            )
          },
        }),
      )
    }

    const registry = new TurnRegistry({
      retainAfterDoneMs: 1000,
      sweepIntervalMs: 60_000,
    })
    const activity = new ServerActivity(registry)
    const browser = {
      resolveTabIds: mock(async () => new Map<number, number>()),
      closePage: mock(async () => {}),
    }
    const service = new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub() as never,
      browser: browser as never,
      browserSession: { pages: {} } as never,
      serverPort: 32123,
      activity,
    })

    const response = await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'stop after the first chunk',
        isScheduledTask: false,
        mode: 'agent',
        origin: 'sidepanel',
        browserContext: {
          activeTab: {
            id: 3,
            url: 'https://example.com',
            title: 'Example',
          },
        },
      } as never,
      abortController.signal,
    )

    expect(activity.isBusy()).toBe(true)
    const reader = response.body?.getReader()
    await reader?.read()

    abortController.abort()

    expect(activity.isBusy()).toBe(false)
    await reader?.cancel()
    registry.stopSweeper()
  })
})

describe('ChatService scheduled task hidden page lifecycle', () => {
  it('creates and cleans up a hidden page without creating a hidden window', async () => {
    const fakeAgent = createFakeAgent()
    agentToReturn = fakeAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? fakeAgent.messages })
      return new Response('ok')
    }

    const browser = {
      newPage: mock(async () => 77),
      listPages: mock(async () => [
        {
          pageId: 77,
          windowId: 11,
        },
      ]),
      closePage: mock(async () => {}),
      createWindow: mock(async () => ({ windowId: 11 })),
      closeWindow: mock(async () => {}),
      resolveTabIds: mock(async () => new Map<number, number>()),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub() as never,
      browser: browser as never,
      registry: {} as never,
    })

    await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'Run the scheduled task',
        isScheduledTask: true,
        mode: 'agent',
        origin: 'sidepanel',
        browserContext: {
          windowId: 9,
          activeTab: {
            id: 3,
            url: 'https://example.com',
            title: 'Example',
          },
          selectedTabs: [{ id: 4 }],
          enabledMcpServers: ['slack'],
        },
      } as never,
      new AbortController().signal,
    )

    expect(browser.newPage).toHaveBeenCalledWith('about:blank', {
      hidden: true,
      background: true,
    })
    expect(browser.createWindow).not.toHaveBeenCalled()
    expect(browser.closePage).toHaveBeenCalledWith(77)
    expect(browser.closeWindow).not.toHaveBeenCalled()

    const createArgs = createAgentSpy.mock.calls.at(-1)?.[0] as {
      browserContext?: {
        windowId?: number
        selectedTabs?: unknown[]
        activeTab?: {
          id: number
          pageId: number
          url: string
          title: string
        }
        enabledMcpServers?: string[]
      }
    }
    expect(createArgs.browserContext?.windowId).toBe(11)
    expect(createArgs.browserContext?.selectedTabs).toBeUndefined()
    expect(createArgs.browserContext?.activeTab).toEqual({
      id: 77,
      pageId: 77,
      url: 'about:blank',
      title: 'Scheduled Task',
    })
    expect(createArgs.browserContext?.enabledMcpServers).toEqual(['slack'])
  })

  it('deleteSession closes the tracked hidden page', async () => {
    const fakeAgent = createFakeAgent()
    const sessionStore = createSessionStore()
    const browser = {
      closePage: mock(async () => {}),
    }
    const conversationId = crypto.randomUUID()

    sessionStore.set(conversationId, {
      agent: fakeAgent,
      hiddenPageId: 33,
    })

    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub() as never,
      browser: browser as never,
      registry: {} as never,
    })

    const result = await service.deleteSession(conversationId)

    expect(result).toEqual({ deleted: true, sessionCount: 0 })
    expect(browser.closePage).toHaveBeenCalledWith(33)
    expect(fakeAgent.dispose).toHaveBeenCalledTimes(1)
  })

  it('keeps the scheduled hidden page context when metadata lookup fails', async () => {
    const fakeAgent = createFakeAgent()
    agentToReturn = fakeAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? fakeAgent.messages })
      return new Response('ok')
    }

    const browser = {
      newPage: mock(async () => 88),
      listPages: mock(async () => {
        throw new Error('CDP lookup failed')
      }),
      closePage: mock(async () => {}),
      resolveTabIds: mock(async () => new Map<number, number>()),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub() as never,
      browser: browser as never,
      registry: {} as never,
    })

    await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'Run the scheduled task',
        isScheduledTask: true,
        mode: 'agent',
        origin: 'sidepanel',
        browserContext: {
          activeTab: {
            id: 3,
            url: 'https://example.com',
            title: 'Example',
          },
        },
      } as never,
      new AbortController().signal,
    )

    const createArgs = createAgentSpy.mock.calls.at(-1)?.[0] as {
      browserContext?: {
        windowId?: number
        activeTab?: {
          id: number
          pageId: number
          url: string
          title: string
        }
      }
    }
    expect(createArgs.browserContext?.windowId).toBeUndefined()
    expect(createArgs.browserContext?.activeTab).toEqual({
      id: 88,
      pageId: 88,
      url: 'about:blank',
      title: 'Scheduled Task',
    })
    expect(browser.closePage).toHaveBeenCalledWith(88)
  })
})

describe('ChatService browser tool config', () => {
  it('passes browser session into new and rebuilt agent sessions', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    let klavisStatus: KlavisProxyStatus = { state: 'connecting' }
    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 100])),
      ),
      closePage: mock(async () => {}),
    }
    const service = new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub(() => klavisStatus) as never,
      browser: browser as never,
      browserSession: { pages: {} } as never,
    })
    const createCallsBefore = createAgentSpy.mock.calls.length
    const request = {
      conversationId: crypto.randomUUID(),
      message: 'check integrations',
      isScheduledTask: false,
      mode: 'agent',
      origin: 'sidepanel',
      browserContext: {
        activeTab: {
          id: 3,
          url: 'https://example.com',
          title: 'Example',
        },
        enabledMcpServers: ['slack'],
      },
    } as never

    await service.processMessage(request, new AbortController().signal)

    agentToReturn = secondAgent
    klavisStatus = { state: 'ready', toolCount: 0 }

    await service.processMessage(
      { ...request, message: 'check integrations again' },
      new AbortController().signal,
    )

    const createCalls = createAgentSpy.mock.calls.slice(createCallsBefore)
    expect(createCalls).toHaveLength(2)
    for (const [config] of createCalls) {
      expect(config).toMatchObject({ browserSession: { pages: {} } })
    }
  })
})

describe('ChatService Klavis session rebuilds', () => {
  it('rebuilds a managed-app session when Klavis becomes ready', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    let lastPromptUiMessages: MockMessage[] | undefined
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      lastPromptUiMessages = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    let klavisStatus: KlavisProxyStatus = { state: 'connecting' }
    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 100])),
      ),
      closePage: mock(async () => {}),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub(() => klavisStatus) as never,
      browser: browser as never,
      registry: {} as never,
    })
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const request = {
      conversationId,
      message: 'check integrations',
      isScheduledTask: false,
      mode: 'agent',
      origin: 'sidepanel',
      browserContext: {
        activeTab: {
          id: 3,
          url: 'https://example.com',
          title: 'Example',
        },
        enabledMcpServers: ['slack'],
      },
    } as never

    await service.processMessage(request, new AbortController().signal)

    agentToReturn = secondAgent
    klavisStatus = { state: 'ready', toolCount: 0 }

    await service.processMessage(
      { ...request, message: 'check integrations again' },
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)
    const firstCreateConfig = createAgentSpy.mock.calls[
      createCallsBefore
    ]?.[0] as { outputFileAccess?: unknown } | undefined
    const secondCreateConfig = createAgentSpy.mock.calls[
      createCallsBefore + 1
    ]?.[0] as { outputFileAccess?: unknown } | undefined
    expect(firstCreateConfig?.outputFileAccess).toBeDefined()
    expect(secondCreateConfig?.outputFileAccess).toBe(
      firstCreateConfig?.outputFileAccess,
    )

    // Persisted form stays the raw user text — TKT-774. The Klavis
    // context-change notice and the formatted user envelope go only
    // into the transient prompt copy fed to the LLM.
    expect(secondAgent.messages).toHaveLength(2)
    const persistedRebuiltMessage =
      secondAgent.messages[1]?.parts[0]?.text ?? ''
    expect(persistedRebuiltMessage).toBe('check integrations again')

    // Prompt copy (what the agent loop actually saw) carries the
    // context-change prefix so the model knows about the new tools.
    const promptRebuiltMessage =
      lastPromptUiMessages?.at(-1)?.parts[0]?.text ?? ''
    expect(promptRebuiltMessage).toContain(
      'Klavis app integration tools are now available for the following connected apps: slack.',
    )
    expect(promptRebuiltMessage).not.toContain('klavis:connecting')
    expect(promptRebuiltMessage).not.toContain('klavis:ready')
  })

  it('does not rebuild a session with no enabled managed apps when Klavis connects', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    let klavisStatus: KlavisProxyStatus = { state: 'connecting' }
    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 200])),
      ),
      closePage: mock(async () => {}),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub(() => klavisStatus) as never,
      browser: browser as never,
      registry: {} as never,
    })
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const request = {
      conversationId,
      message: 'check browser only',
      isScheduledTask: false,
      mode: 'agent',
      origin: 'sidepanel',
      browserContext: {
        activeTab: {
          id: 5,
          url: 'https://example.com',
          title: 'Example',
        },
      },
    } as never

    await service.processMessage(request, new AbortController().signal)

    agentToReturn = secondAgent
    klavisStatus = { state: 'ready', toolCount: 0 }

    await service.processMessage(
      { ...request, message: 'check browser only again' },
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(1)
    expect(firstAgent.dispose).not.toHaveBeenCalled()
    expect(firstAgent.messages).toHaveLength(2)
  })
})

describe('ChatService chat/agent mode switches', () => {
  // An agent's toolset and system prompt are frozen when the session is
  // built, so a mode change only takes effect if the session is rebuilt.

  // resolveLLMConfigSpy is module-scoped and shared with every other describe
  // in this file. Pin it per test rather than inheriting whatever the last one
  // happened to leave behind.
  beforeEach(() => {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'test-key',
    }))
  })

  function createModeSwitchService() {
    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 100])),
      ),
      closePage: mock(async () => {}),
    }
    return new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub() as never,
      browser: browser as never,
      registry: {} as never,
    })
  }

  function modeRequest(conversationId: string, mode: 'chat' | 'agent') {
    return {
      conversationId,
      message: 'please open a new tab and go to github.com',
      isScheduledTask: false,
      mode,
      origin: 'newtab',
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
      },
    } as never
  }

  it('rebuilds the session when the user switches from chat to agent', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    let lastPromptUiMessages: MockMessage[] | undefined
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      lastPromptUiMessages = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    const service = createModeSwitchService()
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()

    await service.processMessage(
      modeRequest(conversationId, 'chat'),
      new AbortController().signal,
    )

    agentToReturn = secondAgent

    await service.processMessage(
      modeRequest(conversationId, 'agent'),
      new AbortController().signal,
    )

    const createCalls = createAgentSpy.mock.calls.slice(createCallsBefore)
    expect(createCalls).toHaveLength(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)

    const firstConfig = createCalls[0]?.[0] as {
      resolvedConfig?: { chatMode?: boolean }
    }
    const secondConfig = createCalls[1]?.[0] as {
      resolvedConfig?: { chatMode?: boolean }
    }
    expect(firstConfig?.resolvedConfig?.chatMode).toBe(true)
    expect(secondConfig?.resolvedConfig?.chatMode).toBe(false)

    // The model-visible half: the prompt copy carries the transition notice,
    // while the persisted message stays the raw user text.
    const promptText = lastPromptUiMessages?.at(-1)?.parts[0]?.text ?? ''
    expect(promptText).toContain('The user switched to agent mode')
    expect(secondAgent.messages.at(-1)?.parts[0]?.text).toBe(
      'please open a new tab and go to github.com',
    )
  })

  it('leaves ACP sessions alone on a mode switch', async () => {
    // Deliberate scope, see the carve-out in chat-service.ts. An ACP agent's
    // BrowserOS instructions live in the workspace instruction file, which a
    // rebuild does not refresh (ensureWorkspaceInstructionFile skips whenever
    // isNewConversation is false, and that is false on every rebuild). A
    // rebuild would leave the agent with a fresh in-band prompt contradicting
    // the stale on-disk one, so mode switching for ACP needs its own change.
    const firstAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'claude-code',
      model: 'claude-opus-4-8',
      apiKey: 'test-key',
    }))

    const service = createModeSwitchService()
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()

    await service.processMessage(
      modeRequest(conversationId, 'chat'),
      new AbortController().signal,
    )
    await service.processMessage(
      modeRequest(conversationId, 'agent'),
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(1)
    expect(firstAgent.dispose).not.toHaveBeenCalled()
  })

  it('re-restricts the session when switching back to chat mode', async () => {
    // The toggle has to enforce in both directions. Rebuilding only on
    // chat -> agent would leave chat -> agent -> chat holding full write tools
    // while the UI reads chat.
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    const thirdAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    const service = createModeSwitchService()
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()

    await service.processMessage(
      modeRequest(conversationId, 'chat'),
      new AbortController().signal,
    )
    agentToReturn = secondAgent
    await service.processMessage(
      modeRequest(conversationId, 'agent'),
      new AbortController().signal,
    )
    agentToReturn = thirdAgent
    await service.processMessage(
      modeRequest(conversationId, 'chat'),
      new AbortController().signal,
    )

    const createCalls = createAgentSpy.mock.calls.slice(createCallsBefore)
    expect(createCalls).toHaveLength(3)
    const thirdConfig = createCalls[2]?.[0] as {
      resolvedConfig?: { chatMode?: boolean }
    }
    expect(thirdConfig?.resolvedConfig?.chatMode).toBe(true)
  })

  it('does not rebuild the session when the mode is unchanged', async () => {
    const firstAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }

    const service = createModeSwitchService()
    const createCallsBefore = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()

    await service.processMessage(
      modeRequest(conversationId, 'agent'),
      new AbortController().signal,
    )
    await service.processMessage(
      modeRequest(conversationId, 'agent'),
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(1)
    expect(firstAgent.dispose).not.toHaveBeenCalled()
  })
})

describe('ChatService single-rebuild reconciliation', () => {
  // When several session inputs change in the same turn, the session must be
  // rebuilt exactly once (one AiSdkAgent.create beyond the initial build), and
  // every applicable change notice must still reach the model.

  beforeEach(() => {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'test-key',
    }))
  })

  function makeService(getKlavis: () => KlavisProxyStatus) {
    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 100])),
      ),
      closePage: mock(async () => {}),
    }
    return new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub(getKlavis) as never,
      browser: browser as never,
      registry: {} as never,
    })
  }

  function captureStreamPrompt() {
    const captured: { prompt?: MockMessage[] } = {}
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      captured.prompt = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }
    return captured
  }

  it('rebuilds once and emits both notices when MCP servers and mode change together', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    const captured = captureStreamPrompt()

    let klavis: KlavisProxyStatus = { state: 'connecting' }
    const service = makeService(() => klavis)
    const before = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const base = {
      conversationId,
      isScheduledTask: false,
      origin: 'newtab',
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
        enabledMcpServers: ['slack'],
      },
    }

    await service.processMessage(
      { ...base, message: 'hi', mode: 'chat' } as never,
      new AbortController().signal,
    )
    agentToReturn = secondAgent
    klavis = { state: 'ready', toolCount: 0 }
    await service.processMessage(
      { ...base, message: 'now act', mode: 'agent' } as never,
      new AbortController().signal,
    )

    // One initial build + exactly one rebuild covering both changes.
    expect(createAgentSpy.mock.calls.length - before).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)

    const text = captured.prompt?.at(-1)?.parts[0]?.text ?? ''
    expect(text).toContain(
      'Klavis app integration tools are now available for the following connected apps: slack.',
    )
    expect(text).toContain('The user switched to agent mode')
  })

  it('rebuilds once and emits both notices when workspace and mode change together', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    const captured = captureStreamPrompt()

    const service = makeService(() => ({ state: 'stopped' }))
    const before = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const base = {
      conversationId,
      isScheduledTask: false,
      origin: 'newtab',
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
      },
    }

    await service.processMessage(
      { ...base, message: 'hi', mode: 'agent' } as never,
      new AbortController().signal,
    )
    agentToReturn = secondAgent
    await service.processMessage(
      {
        ...base,
        message: 'restrict me',
        mode: 'chat',
        userWorkingDir: '/ws',
      } as never,
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - before).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)

    const text = captured.prompt?.at(-1)?.parts[0]?.text ?? ''
    expect(text).toContain(
      'The user connected a workspace during this conversation, but read-only chat mode',
    )
    expect(text).toContain('The user switched to read-only chat mode')
  })

  it('keeps the workspace notice when MCP servers also change in the same turn', async () => {
    // Regression guard for the fix. The previous flag-based flow rebuilt on the
    // MCP branch first, which restamped session.workingDir and silently dropped
    // the workspace notice. Reading a pre-rebuild snapshot emits both.
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    const captured = captureStreamPrompt()

    let klavis: KlavisProxyStatus = { state: 'connecting' }
    const service = makeService(() => klavis)
    const before = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const base = {
      conversationId,
      isScheduledTask: false,
      origin: 'newtab',
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
        enabledMcpServers: ['slack'],
      },
    }

    await service.processMessage(
      { ...base, message: 'hi', mode: 'agent' } as never,
      new AbortController().signal,
    )
    agentToReturn = secondAgent
    klavis = { state: 'ready', toolCount: 0 }
    await service.processMessage(
      {
        ...base,
        message: 'connect a workspace',
        mode: 'agent',
        userWorkingDir: '/ws',
      } as never,
      new AbortController().signal,
    )

    // Still a single rebuild for both changes.
    expect(createAgentSpy.mock.calls.length - before).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)

    const text = captured.prompt?.at(-1)?.parts[0]?.text ?? ''
    expect(text).toContain(
      'Klavis app integration tools are now available for the following connected apps: slack.',
    )
    expect(text).toContain(
      'The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: /ws',
    )
  })

  it('rebuilds an ACP session for an MCP change without adopting a mid-conversation mode switch', async () => {
    // The ACP exclusion must hold even when another input triggers the rebuild:
    // an ACP agent's mode lives in the on-disk instruction file that a rebuild
    // does not refresh, so a Klavis-driven rebuild must not flip it to chat.
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    captureStreamPrompt()
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'claude-code',
      model: 'claude-opus-4-8',
      apiKey: 'test-key',
    }))

    let klavis: KlavisProxyStatus = { state: 'connecting' }
    const service = makeService(() => klavis)
    const before = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const base = {
      conversationId,
      isScheduledTask: false,
      origin: 'newtab',
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
        enabledMcpServers: ['slack'],
      },
    }

    await service.processMessage(
      { ...base, message: 'hi', mode: 'agent' } as never,
      new AbortController().signal,
    )
    agentToReturn = secondAgent
    klavis = { state: 'ready', toolCount: 0 }
    await service.processMessage(
      { ...base, message: 'now restrict me', mode: 'chat' } as never,
      new AbortController().signal,
    )

    const createCalls = createAgentSpy.mock.calls.slice(before)
    // The MCP change still rebuilds the ACP session.
    expect(createCalls).toHaveLength(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)
    // ...but the rebuild keeps the conversation's original agent mode instead
    // of adopting the ignored chat toggle.
    const rebuiltConfig = createCalls[1]?.[0] as {
      resolvedConfig?: { chatMode?: boolean }
    }
    expect(rebuiltConfig?.resolvedConfig?.chatMode).toBe(false)
  })

  it('keeps an ACP session in agent mode even when the request asks for chat mode', async () => {
    // ACP ignores the chat toggle entirely; chat mode is never enforced for it,
    // so the build is agent mode regardless of what the request carries.
    const firstAgent = createFakeAgent()
    agentToReturn = firstAgent
    captureStreamPrompt()
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'claude-code',
      model: 'claude-opus-4-8',
      apiKey: 'test-key',
    }))

    const service = makeService(() => ({ state: 'stopped' }))
    const before = createAgentSpy.mock.calls.length
    await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'hi',
        isScheduledTask: false,
        mode: 'chat',
        origin: 'newtab',
        browserContext: {
          activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
        },
      } as never,
      new AbortController().signal,
    )

    const createConfig = createAgentSpy.mock.calls[before]?.[0] as {
      resolvedConfig?: { chatMode?: boolean }
    }
    expect(createConfig?.resolvedConfig?.chatMode).toBe(false)
  })
})

describe('ChatService ACP provider chat history handling', () => {
  // ACP-backed providers (claude-code, codex, acp-custom) run against
  // a persistent acpx session that owns the agent's conversation
  // memory on disk. Re-feeding the full UIMessage history would double
  // bookkeeping and trip the AI SDK validator when it walks phantom
  // tool-<name> parts emitted by acpx-ai-provider under freshly-
  // generated "acpx-N" ids (acpx#37). The chat-service therefore sends
  // only the new user message on ACP turns; acpx loads prior turns
  // from disk transparently. These tests pin that branch.

  function withAcpProvider() {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'claude-code',
      model: 'opus',
      apiKey: 'unused',
    }))
  }

  function withLlmProvider() {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'test-key',
    }))
  }

  function baseDeps() {
    const browser = {
      newPage: mock(async () => 0),
      listPages: mock(async () => []),
      closePage: mock(async () => {}),
      createWindow: mock(async () => ({ windowId: 0 })),
      closeWindow: mock(async () => {}),
      resolveTabIds: mock(async () => new Map<number, number>()),
    }
    return {
      browser,
      klavis: createKlavisStub(),
      sessionStore: createSessionStore(),
    }
  }

  function chatRequest(overrides: Record<string, unknown> = {}) {
    return {
      conversationId: crypto.randomUUID(),
      message: 'hello',
      isScheduledTask: false,
      mode: 'agent',
      origin: 'sidepanel',
      browserContext: {
        activeTab: { id: 1, url: 'https://example.com', title: 'Example' },
      },
      ...overrides,
    } as never
  }

  it('passes only the new user message to streamText for ACP providers', async () => {
    withAcpProvider()
    const agent = createFakeAgent()
    agentToReturn = agent
    let captured: MockMessage[] | undefined
    streamResponseHandler = async ({ uiMessages, onFinish }) => {
      captured = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }
    const deps = baseDeps()
    const service = new ChatService({
      sessionStore: deps.sessionStore as never,
      klavis: deps.klavis as never,
      browser: deps.browser as never,
      registry: {} as never,
    })

    await service.processMessage(
      chatRequest({
        browserContext: {
          activeTab: { id: 1, url: 'https://example.com', title: 'Example' },
          enabledMcpServers: ['Slack', 'Google Docs'],
        },
      }),
      new AbortController().signal,
    )

    expect(captured).toHaveLength(1)
    expect(captured?.[0]?.role).toBe('user')
    expect(captured?.[0]?.parts[0]?.type).toBe('text')
    const createArgs = createAgentSpy.mock.calls.at(-1)?.[0] as {
      resolvedConfig?: {
        acpMcpServers?: Array<{
          type: 'http'
          headers: Array<{ name: string; value: string }>
        }>
      }
    }
    expect(
      createArgs.resolvedConfig?.acpMcpServers?.[0]?.headers.find(
        (h) => h.name === 'X-BrowserOS-Managed-Mcp-Servers',
      )?.value,
    ).toBe('Slack,Google%20Docs')
  })

  it('still passes the full filtered history for LLM-API providers', async () => {
    withLlmProvider()
    const agent = createFakeAgent()
    // Seed prior turns.
    agent.messages.push(
      { id: 'u-0', role: 'user', parts: [{ type: 'text', text: 'hi' }] },
      {
        id: 'a-0',
        role: 'assistant',
        parts: [{ type: 'text', text: 'hello' }],
      },
    )
    agentToReturn = agent
    let captured: MockMessage[] | undefined
    streamResponseHandler = async ({ uiMessages, onFinish }) => {
      captured = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }
    const deps = baseDeps()
    const service = new ChatService({
      sessionStore: deps.sessionStore as never,
      klavis: deps.klavis as never,
      browser: deps.browser as never,
      registry: {} as never,
    })

    await service.processMessage(chatRequest(), new AbortController().signal)

    expect(captured?.length).toBeGreaterThan(1)
    expect(captured?.map((m) => m.role)).toContain('assistant')
  })

  it('does not re-feed phantom acpx-N tool parts to streamText on a follow-up ACP turn', async () => {
    withAcpProvider()
    const agent = createFakeAgent()
    // Simulate a prior turn where acpx-ai-provider's translator left
    // a phantom tool part behind in session.agent.messages.
    agent.messages.push(
      {
        id: 'u-prior',
        role: 'user',
        parts: [{ type: 'text', text: 'list files' }],
      },
      {
        id: 'a-prior',
        role: 'assistant',
        parts: [
          { type: 'text', text: 'I will list them.' },
          // The phantom shape we worry about: tool part with the
          // acpx-N toolCallId and no input. With the old code this
          // would re-enter streamText on the next turn and trip
          // the AI SDK validator with the 500 the user reported.
          // The new code never includes this in promptUiMessages.
          {
            type: 'tool-mcp.browseros.grep',
            toolCallId: 'acpx-3',
            state: 'input-streaming',
            input: undefined,
          } as never,
        ],
      },
    )
    agentToReturn = agent
    let captured: MockMessage[] | undefined
    streamResponseHandler = async ({ uiMessages, onFinish }) => {
      captured = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return new Response('ok')
    }
    const deps = baseDeps()
    const service = new ChatService({
      sessionStore: deps.sessionStore as never,
      klavis: deps.klavis as never,
      browser: deps.browser as never,
      registry: {} as never,
    })

    await service.processMessage(
      chatRequest({ message: 'what about gaming' }),
      new AbortController().signal,
    )

    // Crucial: the phantom part never reaches streamText.
    const allParts = (captured ?? []).flatMap((m) => m.parts)
    expect(
      allParts.some((p) => (p as { type?: string }).type?.startsWith('tool-')),
    ).toBe(false)
    expect(captured?.length).toBe(1)
  })

  it('preserves UI display state by appending the assistant reply to session.agent.messages on an ACP turn', async () => {
    withAcpProvider()
    const agent = createFakeAgent()
    agent.messages.push(
      {
        id: 'u-prior',
        role: 'user',
        parts: [{ type: 'text', text: 'list files' }],
      },
      {
        id: 'a-prior',
        role: 'assistant',
        parts: [{ type: 'text', text: 'one, two, three.' }],
      },
    )
    agentToReturn = agent
    streamResponseHandler = async ({ uiMessages, onFinish }) => {
      // Simulate the AI SDK reducer yielding the single user msg we
      // sent + a fresh assistant reply.
      const assistantMsg = {
        id: 'a-new',
        role: 'assistant' as const,
        parts: [{ type: 'text' as const, text: 'foo, bar, baz.' }],
      }
      await onFinish({ messages: [...(uiMessages ?? []), assistantMsg] })
      return new Response('ok')
    }
    const deps = baseDeps()
    const service = new ChatService({
      sessionStore: deps.sessionStore as never,
      klavis: deps.klavis as never,
      browser: deps.browser as never,
      registry: {} as never,
    })

    await service.processMessage(
      chatRequest({ message: 'now read foo.md' }),
      new AbortController().signal,
    )

    // Prior turns survive, the new user msg has raw text, the
    // assistant reply is appended at the end.
    expect(agent.messages.map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'user',
      'assistant',
    ])
    expect(agent.messages.at(-1)?.parts[0]?.text).toBe('foo, bar, baz.')
    expect(agent.messages.at(-2)?.parts[0]?.text).toBe('now read foo.md')
  })
})
