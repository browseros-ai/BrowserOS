import { describe, expect, it, mock } from 'bun:test'
import type { ProviderRow } from '../../../src/lib/db/schema'

const refineCalls: Array<Record<string, unknown>> = []

mock.module('../../../src/lib/clients/llm/refine-prompt', () => ({
  refinePrompt: async (llmConfig: Record<string, unknown>) => {
    refineCalls.push(llmConfig)
    return { success: true, refined: 'Refined prompt' }
  },
}))

const { createRefinePromptRoutes } = await import(
  '../../../src/api/routes/refine-prompt'
)

function row(overrides: Partial<ProviderRow> = {}): ProviderRow {
  return {
    id: 'openai-1',
    kind: 'llm',
    type: 'openai',
    name: 'My OpenAI',
    baseUrl: null,
    headers: null,
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    apiKey: 'sk-stored',
    accessKeyId: null,
    secretAccessKey: null,
    sessionToken: null,
    resourceName: null,
    region: null,
    reasoningEffort: null,
    reasoningSummary: null,
    isDefault: false,
    workingDirectory: null,
    customConfig: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  } as ProviderRow
}

function lookup(rows: ProviderRow[]) {
  return {
    getWithCredentials: async (id: string) =>
      rows.find((r) => r.id === id) ?? null,
    getDefaultWithCredentials: async () =>
      rows.find((r) => r.isDefault) ?? null,
    listLlm: async () => rows.filter((r) => r.kind === 'llm'),
  }
}

async function refine(
  rows: ProviderRow[],
  body: Record<string, unknown>,
): Promise<{ status: number; json: Record<string, unknown> }> {
  refineCalls.length = 0
  const app = createRefinePromptRoutes({ providerStore: lookup(rows) })
  const response = await app.request('/', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ prompt: 'Check mail', name: 'Brief', ...body }),
  })
  return { status: response.status, json: await response.json() }
}

const agent = row({ id: 'agent-1', kind: 'acp', type: 'codex', apiKey: null })

describe('refine-prompt provider resolution', () => {
  it('uses the named provider and its stored credentials', async () => {
    const { status } = await refine([row()], { providerId: 'openai-1' })

    expect(status).toBe(200)
    expect(refineCalls[0]).toMatchObject({
      provider: 'openai',
      model: 'gpt-5.5',
      apiKey: 'sk-stored',
    })
  })

  it('refines on an LLM provider when the selected target is a coding agent', async () => {
    // The selection covers both kinds, and a coding agent cannot refine. The
    // extension used to resolve through a list that only ever held LLM
    // providers, so this never used to fail; rejecting here would break the
    // rewrite button for anyone whose default is an agent.
    const { status } = await refine([row(), { ...agent, isDefault: true }], {})

    expect(status).toBe(200)
    expect(refineCalls[0]).toMatchObject({ provider: 'openai' })
  })

  it('prefers the selected provider over the rest of the list', async () => {
    const { status } = await refine(
      [row(), row({ id: 'openai-2', type: 'anthropic', isDefault: true })],
      {},
    )

    expect(status).toBe(200)
    expect(refineCalls[0]).toMatchObject({ provider: 'anthropic' })
  })

  it('keeps an older client on the configuration it sent', async () => {
    // No id, whole configuration inline. That is its user's choice as surely
    // as an id is, so the selected provider must not be read over the top.
    const { status } = await refine(
      [row({ id: 'openai-2', type: 'anthropic', isDefault: true })],
      { provider: 'openai', model: 'gpt-4o', apiKey: 'sk-inline' },
    )

    expect(status).toBe(200)
    expect(refineCalls[0]).toMatchObject({
      provider: 'openai',
      model: 'gpt-4o',
      apiKey: 'sk-inline',
    })
  })

  it('rejects a named coding agent rather than silently using another row', async () => {
    const { status, json } = await refine([row(), agent], {
      providerId: 'agent-1',
    })

    expect(status).toBe(400)
    expect(json.message).toContain('coding agent')
    expect(refineCalls).toHaveLength(0)
  })

  it('rejects a named provider that does not exist', async () => {
    const { status, json } = await refine([row()], { providerId: 'gone' })

    expect(status).toBe(400)
    expect(json.message).toContain('Unknown provider gone')
    expect(refineCalls).toHaveLength(0)
  })

  it('says what to do when nothing can refine', async () => {
    const { status, json } = await refine([{ ...agent, isDefault: true }], {})

    expect(status).toBe(400)
    expect(json.message).toContain('Connect an LLM provider')
    expect(refineCalls).toHaveLength(0)
  })
})
