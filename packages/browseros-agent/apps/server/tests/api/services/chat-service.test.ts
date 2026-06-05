import { describe, expect, it, mock } from 'bun:test'

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
  onFinish(args: { messages: MockMessage[] }): Promise<void>
}

type MockStreamChunk =
  | { type: 'start' }
  | { type: 'start-step' }
  | { type: 'text-start'; id: string }
  | { type: 'text-delta'; id: string; delta: string }
  | { type: 'text-end'; id: string }
  | { type: 'finish' }
  | { type: 'error'; errorText: string }

let agentToReturn: MockAgent | undefined
let streamResponseHandler:
  | ((options: StreamResponseOptions) => Promise<MockStreamChunk[] | undefined>)
  | undefined
let drainedStreamChunks: MockStreamChunk[] = []

const createAgentSpy = mock(async (config: unknown) => {
  if (!agentToReturn) {
    throw new Error(`No mock agent configured for ${JSON.stringify(config)}`)
  }
  return agentToReturn
})

const createAgentUIStreamSpy = mock(async (options: StreamResponseOptions) => {
  if (!streamResponseHandler) {
    throw new Error('No stream response handler configured')
  }
  const chunks = await streamResponseHandler(options)
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(chunk)
      }
      controller.close()
    },
  })
})

const createUIMessageStreamResponseSpy = mock(
  async ({ stream }: { stream: ReadableStream<MockStreamChunk> }) => {
    drainedStreamChunks = []
    const reader = stream.getReader()
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      drainedStreamChunks.push(value)
    }
    return new Response('ok')
  },
)

const createAgentUIStreamResponseSpy = mock(
  async (options: StreamResponseOptions) => {
    if (!streamResponseHandler) {
      throw new Error('No stream response handler configured')
    }
    await streamResponseHandler(options)
    return new Response('ok')
  },
)

const resolveLLMConfigSpy = mock(async () => ({
  provider: 'openai',
  model: 'gpt-5',
  apiKey: 'test-key',
}))

mock.module('ai', () => ({
  createAgentUIStream: createAgentUIStreamSpy,
  createUIMessageStreamResponse: createUIMessageStreamResponseSpy,
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
      this.messages.push({
        id: 'user-1',
        role: 'user',
        parts: [{ type: 'text', text }],
      })
    },
    dispose: mock(async () => {}),
  }
}

describe('ChatService scheduled task hidden page lifecycle', () => {
  it('creates and cleans up a hidden page without creating a hidden window', async () => {
    const fakeAgent = createFakeAgent()
    agentToReturn = fakeAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? fakeAgent.messages })
      return []
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
      klavisRef: { handle: null },
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
      klavisRef: { handle: null },
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
      return []
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
      klavisRef: { handle: null },
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

describe('ChatService Klavis session rebuilds', () => {
  it('rebuilds a managed-app session when the shared Klavis handle appears', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    let lastPromptUiMessages: MockMessage[] | undefined
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      lastPromptUiMessages = uiMessages
      await onFinish({ messages: uiMessages ?? [] })
      return []
    }

    const klavisRef = { handle: null as object | null }
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
      klavisRef: klavisRef as never,
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
    klavisRef.handle = {}

    await service.processMessage(
      { ...request, message: 'check integrations again' },
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)

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
    expect(promptRebuiltMessage).not.toContain('klavis:pending')
    expect(promptRebuiltMessage).not.toContain('klavis:connected')
  })

  it('does not rebuild a session with no enabled managed apps when Klavis connects', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      await onFinish({ messages: uiMessages ?? [] })
      return []
    }

    const klavisRef = { handle: null as object | null }
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
      klavisRef: klavisRef as never,
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
    klavisRef.handle = {}

    await service.processMessage(
      { ...request, message: 'check browser only again' },
      new AbortController().signal,
    )

    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(1)
    expect(firstAgent.dispose).not.toHaveBeenCalled()
    expect(firstAgent.messages).toHaveLength(2)
  })
})

describe('ChatService context-limit recovery', () => {
  it('compacts sidepanel chat history, rebuilds the session, and retries once', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent

    let streamCallCount = 0
    let retryPromptMessages: MockMessage[] | undefined
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      streamCallCount++
      if (streamCallCount === 1) {
        agentToReturn = secondAgent
        return [
          { type: 'start' },
          { type: 'start-step' },
          {
            type: 'error',
            errorText:
              'Your input exceeds the context window of this model. Please adjust your input and try again.',
          },
        ]
      }

      retryPromptMessages = uiMessages
      await onFinish({
        messages: [
          ...(uiMessages ?? []),
          {
            id: 'assistant-final',
            role: 'assistant',
            parts: [{ type: 'text', text: 'Recovered response.' }],
          },
        ],
      })
      return [
        { type: 'start' },
        { type: 'start-step' },
        { type: 'text-start', id: 'text-1' },
        { type: 'text-delta', id: 'text-1', delta: 'Recovered response.' },
        { type: 'text-end', id: 'text-1' },
        { type: 'finish' },
      ]
    }

    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 300])),
      ),
      closePage: mock(async () => {}),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavisRef: { handle: null },
      browser: browser as never,
      registry: {} as never,
    })
    const createCallsBefore = createAgentSpy.mock.calls.length

    const previousConversation = Array.from({ length: 8 }, (_, index) => ({
      role: index % 2 === 0 ? ('user' as const) : ('assistant' as const),
      content: `prior message ${index}`,
    }))

    await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'continue the research',
        previousConversation,
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
      new AbortController().signal,
    )

    expect(streamCallCount).toBe(2)
    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)
    expect(
      drainedStreamChunks.some((chunk) => chunk.type === 'error'),
    ).toBeFalse()

    const retryTexts =
      retryPromptMessages?.map((message) => message.parts[0]?.text ?? '') ?? []
    expect(retryTexts[0]).toContain('<continuation_context>')
    expect(retryTexts[0]).toContain('provider reported')
    expect(retryTexts.at(-1)).toContain('<USER_QUERY>')
    expect(retryTexts.at(-1)).toContain('continue the research')

    const persistedTexts = secondAgent.messages.map(
      (message) => message.parts[0]?.text ?? '',
    )
    expect(persistedTexts[0]).toContain('<continuation_context>')
    expect(persistedTexts).toContain('continue the research')
    expect(persistedTexts).toContain('Recovered response.')
  })

  it('retries a context-limit failure after partial assistant output', async () => {
    const firstAgent = createFakeAgent()
    const secondAgent = createFakeAgent()
    agentToReturn = firstAgent

    let streamCallCount = 0
    let retryPromptMessages: MockMessage[] | undefined
    streamResponseHandler = async ({ onFinish, uiMessages }) => {
      streamCallCount++
      if (streamCallCount === 1) {
        agentToReturn = secondAgent
        return [
          { type: 'start' },
          { type: 'start-step' },
          { type: 'text-start', id: 'text-1' },
          {
            type: 'text-delta',
            id: 'text-1',
            delta: 'I found some sources.',
          },
          { type: 'text-end', id: 'text-1' },
          {
            type: 'error',
            errorText:
              'Your input exceeds the context window of this model. Please adjust your input and try again.',
          },
        ]
      }

      retryPromptMessages = uiMessages
      await onFinish({
        messages: [
          ...(uiMessages ?? []),
          {
            id: 'assistant-final',
            role: 'assistant',
            parts: [{ type: 'text', text: 'Recovered continuation.' }],
          },
        ],
      })
      return [
        { type: 'start' },
        { type: 'start-step' },
        { type: 'text-start', id: 'text-2' },
        {
          type: 'text-delta',
          id: 'text-2',
          delta: 'Recovered continuation.',
        },
        { type: 'text-end', id: 'text-2' },
        { type: 'finish' },
      ]
    }

    const browser = {
      resolveTabIds: mock(
        async (tabIds: number[]) =>
          new Map(tabIds.map((tabId) => [tabId, tabId + 400])),
      ),
      closePage: mock(async () => {}),
    }
    const sessionStore = createSessionStore()
    const service = new ChatService({
      sessionStore: sessionStore as never,
      klavisRef: { handle: null },
      browser: browser as never,
      registry: {} as never,
    })
    const createCallsBefore = createAgentSpy.mock.calls.length

    const previousConversation = Array.from({ length: 8 }, (_, index) => ({
      role: index % 2 === 0 ? ('user' as const) : ('assistant' as const),
      content: `prior message ${index}`,
    }))

    await service.processMessage(
      {
        conversationId: crypto.randomUUID(),
        message: 'keep checking those forum sources',
        previousConversation,
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
      new AbortController().signal,
    )

    expect(streamCallCount).toBe(2)
    expect(createAgentSpy.mock.calls.length - createCallsBefore).toBe(2)
    expect(firstAgent.dispose).toHaveBeenCalledTimes(1)
    expect(
      drainedStreamChunks.some((chunk) => chunk.type === 'error'),
    ).toBeFalse()
    expect(
      drainedStreamChunks.filter((chunk) => chunk.type === 'start'),
    ).toHaveLength(1)
    expect(
      drainedStreamChunks
        .filter((chunk) => chunk.type === 'text-delta')
        .map((chunk) => chunk.delta),
    ).toEqual(['I found some sources.', 'Recovered continuation.'])

    const retryTexts =
      retryPromptMessages?.map((message) => message.parts[0]?.text ?? '') ?? []
    expect(retryTexts[0]).toContain('<continuation_context>')
    expect(retryTexts.at(-1)).toContain('keep checking those forum sources')

    const persistedTexts = secondAgent.messages.map(
      (message) => message.parts[0]?.text ?? '',
    )
    expect(persistedTexts[0]).toContain('<continuation_context>')
    expect(persistedTexts).toContain('keep checking those forum sources')
    expect(persistedTexts).toContain('Recovered continuation.')
  })
})
