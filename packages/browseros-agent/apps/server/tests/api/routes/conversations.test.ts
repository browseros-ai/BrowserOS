import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import { createConversationRoutes } from '../../../src/api/routes/conversations'
import type {
  ConversationDetail,
  ConversationStore,
  ConversationSummary,
  SaveConversationInput,
} from '../../../src/lib/conversations/conversation-store'

const CONVERSATION_ID = '00000000-0000-4000-8000-000000000001'

describe('conversation routes', () => {
  it('lists conversation summaries', async () => {
    const store = new MemoryConversationStore([
      detail(CONVERSATION_ID, 'latest question'),
    ])
    const routes = createConversationRoutes({ store })

    const response = await routes.request('/')
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      conversations: [
        { id: CONVERSATION_ID, lastUserMessage: 'latest question' },
      ],
    })
  })

  it('loads a conversation with its full message blob', async () => {
    const store = new MemoryConversationStore([detail(CONVERSATION_ID, 'hi')])
    const routes = createConversationRoutes({ store })

    const response = await routes.request(`/${CONVERSATION_ID}`)
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      conversation: { id: CONVERSATION_ID, messages: [{ id: 'u1' }] },
    })
  })

  it('returns 404 for an unknown conversation', async () => {
    const routes = createConversationRoutes({
      store: new MemoryConversationStore([]),
    })
    expect((await routes.request(`/${CONVERSATION_ID}`)).status).toBe(404)
  })

  it('rejects a non-uuid conversation id', async () => {
    const routes = createConversationRoutes({
      store: new MemoryConversationStore([]),
    })
    expect((await routes.request('/not-a-uuid')).status).toBe(400)
  })

  it('deletes a conversation', async () => {
    const store = new MemoryConversationStore([detail(CONVERSATION_ID, 'hi')])
    const routes = createConversationRoutes({ store })

    expect(
      (await routes.request(`/${CONVERSATION_ID}`, { method: 'DELETE' }))
        .status,
    ).toBe(200)
    expect(
      (await routes.request(`/${CONVERSATION_ID}`, { method: 'DELETE' }))
        .status,
    ).toBe(404)
  })

  it('imports a conversation via put, preserving lastMessagedAt', async () => {
    const store = new MemoryConversationStore([])
    const routes = createConversationRoutes({ store })

    const response = await routes.request(`/${CONVERSATION_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: [
          { id: 'u1', role: 'user', parts: [{ type: 'text', text: 'hi' }] },
        ],
        lastMessagedAt: 42,
      }),
    })

    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      imported: true,
      conversation: {
        id: CONVERSATION_ID,
        lastMessagedAt: 42,
        targetType: 'browseros',
      },
    })
    expect((await store.get(CONVERSATION_ID))?.messages).toHaveLength(1)
  })

  it('does not overwrite an existing conversation on import', async () => {
    const store = new MemoryConversationStore([
      detail(CONVERSATION_ID, 'newer'),
    ])
    const routes = createConversationRoutes({ store })

    const response = await routes.request(`/${CONVERSATION_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: [{ id: 'legacy', role: 'user', parts: [] }],
        lastMessagedAt: 1,
      }),
    })

    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({ imported: false })
    expect((await store.get(CONVERSATION_ID))?.messages[0]?.id).toBe('u1')
  })
})

function detail(id: string, lastUserMessage: string): ConversationDetail {
  const messages: UIMessage[] = [
    {
      id: 'u1',
      role: 'user',
      parts: [{ type: 'text', text: lastUserMessage }],
    },
  ]
  return {
    id,
    lastUserMessage,
    targetType: 'browseros',
    lastMessagedAt: 1,
    createdAt: 1,
    updatedAt: 1,
    messages,
  }
}

class MemoryConversationStore
  implements
    Pick<
      ConversationStore,
      'list' | 'get' | 'save' | 'insertIfAbsent' | 'delete'
    >
{
  private readonly byId = new Map<string, ConversationDetail>()

  constructor(seed: ConversationDetail[]) {
    for (const conversation of seed)
      this.byId.set(conversation.id, conversation)
  }

  async list(): Promise<ConversationSummary[]> {
    return [...this.byId.values()]
  }

  async get(id: string): Promise<ConversationDetail | null> {
    return this.byId.get(id) ?? null
  }

  async save(input: SaveConversationInput): Promise<ConversationSummary> {
    const timestamp = input.lastMessagedAt ?? 1
    const conversation: ConversationDetail = {
      id: input.id,
      messages: input.messages,
      targetType: input.targetType,
      origin: input.origin,
      agentId: input.agentId,
      lastUserMessage: undefined,
      lastMessagedAt: timestamp,
      createdAt: timestamp,
      updatedAt: timestamp,
    }
    this.byId.set(input.id, conversation)
    return conversation
  }

  async insertIfAbsent(
    input: SaveConversationInput,
  ): Promise<ConversationSummary | null> {
    if (this.byId.has(input.id)) return null
    return this.save(input)
  }

  async delete(id: string): Promise<boolean> {
    return this.byId.delete(id)
  }
}
