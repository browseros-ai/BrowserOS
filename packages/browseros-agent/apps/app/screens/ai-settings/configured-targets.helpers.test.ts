import { beforeAll, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

mock.module('@/components/agents/AdapterIcon', () => ({
  AdapterIcon: () => null,
  adapterLabel: (type: string) => (type === 'codex' ? 'Codex' : 'Claude'),
}))

type Helpers = typeof import('./configured-targets.helpers')
let helpers: Helpers

beforeAll(async () => {
  helpers = await import('./configured-targets.helpers')
})

const agent: AcpAgent = {
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Review agent',
  type: 'codex',
  modelId: 'gpt-5.5',
  reasoningEffort: 'medium',
  createdAt: 1,
  updatedAt: 1,
}

const provider: LlmProviderConfig = {
  id: 'p1',
  name: 'My OpenAI',
  type: 'openai',
  modelId: 'gpt-5.5',
  baseUrl: 'https://api.openai.com/v1',
} as LlmProviderConfig

const noop = () => {}

function agentActions(
  overrides: Partial<Parameters<Helpers['buildAgentActions']>[0]> = {},
) {
  return helpers.buildAgentActions({
    agent,
    isDeleting: false,
    onSelectAgent: noop,
    onDeleteAgent: noop,
    ...overrides,
  })
}

function providerActions(
  overrides: Partial<Parameters<Helpers['buildProviderActions']>[0]> = {},
) {
  return helpers.buildProviderActions({
    provider,
    isBuiltIn: false,
    isTesting: false,
    onSelectProvider: noop,
    onTestProvider: noop,
    onEditProvider: noop,
    onDeleteProvider: noop,
    ...overrides,
  })
}

describe('agentDescription', () => {
  it('joins adapter, model and reasoning effort', () => {
    expect(helpers.agentDescription(agent)).toBe('Codex · gpt-5.5 · medium')
  })

  it('identifies a custom agent by its launch command', () => {
    expect(
      helpers.agentDescription({
        ...agent,
        type: 'custom',
        customConfig: { command: 'opencode acp' },
      } as AcpAgent),
    ).toContain('opencode acp')
  })

  it('falls back when the agent has no model of its own', () => {
    const { modelId: _modelId, ...withoutModel } = agent
    expect(helpers.agentDescription(withoutModel as AcpAgent)).toContain(
      'Agent default model',
    )
  })
})

describe('providerDescription', () => {
  it('describes the built-in provider without leaking config', () => {
    expect(helpers.providerDescription(provider, true)).toBe(
      'BrowserOS-hosted model with strict rate limits',
    )
  })

  it('shows model and base url for a configured provider', () => {
    expect(helpers.providerDescription(provider, false)).toBe(
      'gpt-5.5 · https://api.openai.com/v1',
    )
  })

  it('omits the separator when there is no base url', () => {
    expect(
      helpers.providerDescription(
        { ...provider, baseUrl: '' } as LlmProviderConfig,
        false,
      ),
    ).toBe('gpt-5.5')
  })
})

describe('buildProviderActions', () => {
  it('always offers set-as-default first', () => {
    expect(providerActions()[0].label).toBe('Set as default')
  })

  // The built-in provider is not user-owned, so it must not offer destructive
  // or editing actions.
  it('gives the built-in provider nothing beyond set-as-default', () => {
    const actions = providerActions({ isBuiltIn: true })
    expect(actions).toHaveLength(1)
    expect(actions.map((a) => a.label)).not.toContain('Delete')
  })

  it('offers test, edit and delete for a configured provider', () => {
    expect(providerActions().map((a) => a.label)).toEqual([
      'Set as default',
      'Test connection',
      'Edit',
      'Delete',
    ])
  })

  it('marks delete as destructive', () => {
    const del = providerActions().find((a) => a.label === 'Delete')
    expect(del?.destructive).toBe(true)
  })

  it('disables the test action while a test is in flight', () => {
    const testing = providerActions({ isTesting: true }).find((a) =>
      a.label.startsWith('Testing'),
    )
    expect(testing?.disabled).toBe(true)
  })

  it('routes each action to its handler', () => {
    let selected = ''
    let edited = ''
    let deleted = ''
    const actions = helpers.buildProviderActions({
      provider,
      isBuiltIn: false,
      isTesting: false,
      onSelectProvider: (id) => {
        selected = id
      },
      onTestProvider: noop,
      onEditProvider: (p) => {
        edited = p.id
      },
      onDeleteProvider: (p) => {
        deleted = p.id
      },
    })
    for (const action of actions) action.onSelect()
    expect([selected, edited, deleted]).toEqual(['p1', 'p1', 'p1'])
  })
})

describe('buildAgentActions', () => {
  it('offers set-as-default then delete for a built-in adapter', () => {
    expect(agentActions().map((a) => a.label)).toEqual([
      'Set as default',
      'Delete',
    ])
  })

  it('adds edit only for a custom agent', () => {
    const custom = { ...agent, type: 'custom' } as AcpAgent
    expect(
      agentActions({ agent: custom, onEditAgent: noop }).map((a) => a.label),
    ).toEqual(['Set as default', 'Edit', 'Delete'])
  })

  it('omits edit when no edit handler is supplied', () => {
    const custom = { ...agent, type: 'custom' } as AcpAgent
    expect(agentActions({ agent: custom }).map((a) => a.label)).not.toContain(
      'Edit',
    )
  })

  it('omits edit for a non-custom agent even when a handler exists', () => {
    expect(
      agentActions({ onEditAgent: noop }).map((a) => a.label),
    ).not.toContain('Edit')
  })

  it('disables delete while that agent is being deleted', () => {
    const del = agentActions({ isDeleting: true }).find(
      (a) => a.label === 'Delete',
    )
    expect(del?.disabled).toBe(true)
  })

  it('routes delete to the supplied handler', () => {
    let deleted = ''
    const actions = agentActions({
      onDeleteAgent: (a) => {
        deleted = a.id
      },
    })
    actions.find((a) => a.label === 'Delete')?.onSelect()
    expect(deleted).toBe(agent.id)
  })
})
