import { describe, expect, it, mock } from 'bun:test'
import { createAgentRoutes } from '../../../src/api/routes/agents'
import type { AcpAgentDefinition } from '../../../src/lib/agents/agent-types'
import type {
  AcpAgentStore,
  CreateAcpAgentInput,
} from '../../../src/lib/agents/storage/acp-agent-store'

const AGENT_ID = '00000000-0000-4000-8000-000000000001'

describe('ACP agent routes', () => {
  it('creates and lists minimal ACP agents', async () => {
    const store = new MemoryAcpAgentStore()
    const routes = createAgentRoutes({ store })
    const createResponse = await routes.request('/', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: 'Review agent',
        type: 'codex',
        modelId: 'gpt-5.5',
      }),
    })

    expect(createResponse.status).toBe(201)
    expect(await createResponse.json()).toMatchObject({
      agent: {
        id: AGENT_ID,
        name: 'Review agent',
        type: 'codex',
        modelId: 'gpt-5.5',
      },
    })

    const listResponse = await routes.request('/')
    expect(await listResponse.json()).toMatchObject({
      agents: [{ id: AGENT_ID, type: 'codex' }],
    })
  })

  it('rejects legacy harness and custom ACP fields', async () => {
    const routes = createAgentRoutes({ store: new MemoryAcpAgentStore() })
    const response = await routes.request('/', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: 'Legacy agent',
        adapter: 'codex',
        permissionMode: 'approve-all',
        sessionKey: 'main',
        type: 'acp-custom',
      }),
    })

    expect(response.status).toBe(400)
  })

  it('deletes the agent', async () => {
    const store = new MemoryAcpAgentStore()
    const onDelete = mock(async () => {})
    await store.create({ name: 'Claude', type: 'claude' })
    const routes = createAgentRoutes({ store, onDelete })
    expect(
      (await routes.request(`/${AGENT_ID}`, { method: 'DELETE' })).status,
    ).toBe(200)
    expect((await routes.request(`/${AGENT_ID}`)).status).toBe(404)
    expect(onDelete).toHaveBeenCalledWith(AGENT_ID)
  })
})

class MemoryAcpAgentStore implements AcpAgentStore {
  private agent: AcpAgentDefinition | null = null

  async list(): Promise<AcpAgentDefinition[]> {
    return this.agent ? [this.agent] : []
  }

  async get(id: string): Promise<AcpAgentDefinition | null> {
    return this.agent?.id === id ? this.agent : null
  }

  async create(input: CreateAcpAgentInput): Promise<AcpAgentDefinition> {
    this.agent = {
      id: AGENT_ID,
      name: input.name,
      type: input.type,
      modelId: input.modelId,
      reasoningEffort: input.reasoningEffort,
      workingDirectory: input.workingDirectory,
      createdAt: 1,
      updatedAt: 1,
    }
    return this.agent
  }

  async delete(id: string): Promise<boolean> {
    if (this.agent?.id !== id) return false
    this.agent = null
    return true
  }
}
