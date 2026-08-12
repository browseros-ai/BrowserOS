import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import { createConversationRoutes } from '../../../src/api/routes/conversations'
import type {
  ConversationDetail,
  ConversationStore,
  ConversationSummary,
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
  implements Pick<ConversationStore, 'list' | 'get' | 'delete'>
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

  async delete(id: string): Promise<boolean> {
    return this.byId.delete(id)
  }
}
