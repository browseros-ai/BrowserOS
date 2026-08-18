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
  scheduledPageId?: number
}

const BROWSEROS_TARGET = { type: 'browseros', providerId: 'browseros' } as const

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
    error: mock(() => {}),
    info: mock(() => {}),
    warn: mock(() => {}),
    debug: mock(() => {}),
  },
}))

const { ChatService } = await import('../../../src/api/services/chat-service')
const { ServerActivity } = await import(
  '../../../src/api/services/server-activity'
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

    const activity = new ServerActivity()
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
        target: BROWSEROS_TARGET,
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
  })
})

describe('ChatService scheduled task page lifecycle', () => {
  it('creates and cleans up a background page without creating a window', async () => {
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
        target: BROWSEROS_TARGET,
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

  it('deleteSession closes the tracked scheduled page', async () => {
    const fakeAgent = createFakeAgent()
    const sessionStore = createSessionStore()
    const browser = {
      closePage: mock(async () => {}),
    }
    const conversationId = crypto.randomUUID()

    sessionStore.set(conversationId, {
      agent: fakeAgent,
      scheduledPageId: 33,
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

  it('keeps the scheduled page context when metadata lookup fails', async () => {
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
        target: BROWSEROS_TARGET,
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
      target: BROWSEROS_TARGET,
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
      target: BROWSEROS_TARGET,
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

    expect(secondAgent.messages).toHaveLength(2)
    const persistedRebuiltMessage =
      secondAgent.messages[1]?.parts[0]?.text ?? ''
    expect(persistedRebuiltMessage).toBe('check integrations again')

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
      target: BROWSEROS_TARGET,
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
      target: BROWSEROS_TARGET,
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

    const promptText = lastPromptUiMessages?.at(-1)?.parts[0]?.text ?? ''
    expect(promptText).toContain('The user switched to agent mode')
    expect(secondAgent.messages.at(-1)?.parts[0]?.text).toBe(
      'please open a new tab and go to github.com',
    )
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
      target: BROWSEROS_TARGET,
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
      target: BROWSEROS_TARGET,
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
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    const captured = captureStreamPrompt()

    let klavis: KlavisProxyStatus = { state: 'connecting' }
    const service = makeService(() => klavis)
    const before = createAgentSpy.mock.calls.length
    const conversationId = crypto.randomUUID()
    const base = {
      target: BROWSEROS_TARGET,
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
})

describe('ChatService history persistence', () => {
  beforeEach(() => {
    resolveLLMConfigSpy.mockImplementation(async () => ({
      provider: 'openai',
      model: 'gpt-5',
      apiKey: 'test-key',
    }))
  })

  function persistenceBrowser() {
    return {
      resolveTabIds: mock(async () => new Map<number, number>()),
      closePage: mock(async () => {}),
    }
  }

  function browserOsRequest(
    conversationId: string,
    historyMode: 'local' | 'cloud',
  ) {
    return {
      target: BROWSEROS_TARGET,
      conversationId,
      message: 'hello',
      isScheduledTask: false,
      mode: 'agent',
      origin: 'sidepanel',
      historyMode,
      browserContext: {
        activeTab: { id: 3, url: 'https://example.com', title: 'Example' },
      },
    } as never
  }

  it('hydrates from and persists to the store in local mode', async () => {
    const agent = createFakeAgent()
    agentToReturn = agent
    streamResponseHandler = async ({ onFinish }) => {
      // A completed turn ends with the assistant reply.
      await onFinish({
        messages: [
          ...agent.messages,
          {
            id: crypto.randomUUID(),
            role: 'assistant',
            parts: [{ type: 'text', text: 'hi there' }],
          },
        ],
      })
      return new Response('ok')
    }
    const conversationStore = {
      get: mock(async () => ({
        id: 'stored',
        messages: [
          {
            id: 'old',
            role: 'user',
            parts: [{ type: 'text', text: 'earlier' }],
          },
        ],
        targetType: 'browseros',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
      save: mock(async () => ({
        id: 'stored',
        targetType: 'browseros',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
    }
    const service = new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub() as never,
      browser: persistenceBrowser() as never,
      conversationStore: conversationStore as never,
    })
    const conversationId = crypto.randomUUID()

    await service.processMessage(
      browserOsRequest(conversationId, 'local'),
      new AbortController().signal,
    )

    expect(conversationStore.get).toHaveBeenCalledWith(conversationId)
    expect(agent.messages[0]?.parts[0]?.text).toBe('earlier')
    expect(conversationStore.save).toHaveBeenCalledTimes(1)
    const saved = conversationStore.save.mock.calls.at(-1)?.[0] as
      | { id: string; targetType: string }
      | undefined
    expect(saved?.id).toBe(conversationId)
    expect(saved?.targetType).toBe('browseros')
  })

  it('does not persist and drops the dangling user message when a turn errors', async () => {
    const agent = createFakeAgent()
    agentToReturn = agent
    // The stream errored before any output: onFinish reports the history
    // ending on the user message, with no assistant reply appended.
    streamResponseHandler = async ({ onFinish }) => {
      await onFinish({ messages: agent.messages })
      return new Response('ok')
    }
    const conversationStore = {
      get: mock(async () => null),
      save: mock(async () => ({
        id: 'stored',
        targetType: 'browseros',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
    }
    const sessionStore = createSessionStore()
    const conversationId = crypto.randomUUID()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavis: createKlavisStub() as never,
      browser: persistenceBrowser() as never,
      conversationStore: conversationStore as never,
    })

    await service.processMessage(
      browserOsRequest(conversationId, 'local'),
      new AbortController().signal,
    )

    // The errored turn produced no assistant reply, so nothing is persisted:
    // a cold reload from SQLite stays clean. The in-memory session still holds
    // the unanswered user message; the guard is persistence, not a session trim.
    expect(conversationStore.save).not.toHaveBeenCalled()
    expect(sessionStore.get(conversationId)).toBeDefined()
  })

  it('never touches the store and seeds from previousConversation in cloud mode', async () => {
    const agent = createFakeAgent()
    agentToReturn = agent
    streamResponseHandler = async ({ onFinish }) => {
      await onFinish({ messages: agent.messages })
      return new Response('ok')
    }
    const conversationStore = {
      get: mock(async () => null),
      save: mock(async () => ({
        id: 'stored',
        targetType: 'browseros',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
    }
    const service = new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub() as never,
      browser: persistenceBrowser() as never,
      conversationStore: conversationStore as never,
    })

    await service.processMessage(
      {
        ...browserOsRequest(crypto.randomUUID(), 'cloud'),
        previousConversation: [{ role: 'user', content: 'from client' }],
      } as never,
      new AbortController().signal,
    )

    expect(conversationStore.get).not.toHaveBeenCalled()
    expect(conversationStore.save).not.toHaveBeenCalled()
    expect(agent.messages.some((m) => m.parts[0]?.text === 'from client')).toBe(
      true,
    )
  })

  it('ignores a stored record whose target is not browseros in local mode', async () => {
    const agent = createFakeAgent()
    agentToReturn = agent
    streamResponseHandler = async ({ onFinish }) => {
      await onFinish({ messages: agent.messages })
      return new Response('ok')
    }
    const conversationStore = {
      get: mock(async () => ({
        id: 'stored',
        messages: [
          {
            id: 'foreign',
            role: 'user',
            parts: [{ type: 'text', text: 'acp history' }],
          },
        ],
        targetType: 'claude',
        agentId: 'some-agent',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
      save: mock(async () => ({
        id: 'stored',
        targetType: 'browseros',
        lastMessagedAt: 1,
        createdAt: 1,
        updatedAt: 1,
      })),
    }
    const service = new ChatService({
      sessionStore: createSessionStore() as never,
      klavis: createKlavisStub() as never,
      browser: persistenceBrowser() as never,
      conversationStore: conversationStore as never,
    })

    await service.processMessage(
      browserOsRequest(crypto.randomUUID(), 'local'),
      new AbortController().signal,
    )

    expect(
      agent.messages.every((m) => m.parts[0]?.text !== 'acp history'),
    ).toBe(true)
  })
})
