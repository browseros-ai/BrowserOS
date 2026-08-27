import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

mock.module('@/components/agents/AdapterIcon', () => ({
  AdapterIcon: () => createElement('span', { 'data-icon': 'adapter' }),
  adapterLabel: (type: string) => (type === 'codex' ? 'Codex' : 'Claude'),
}))
mock.module('@/components/agents/agent-brand-marks', () => ({
  BRAND_MARKS: {},
  agentBrandKey: () => undefined,
}))
mock.module('@/lib/llm-providers/providerIcons', () => ({
  BrowserOSIcon: () => createElement('span', { 'data-icon': 'browseros' }),
  ProviderIcon: () => createElement('span', { 'data-icon': 'provider' }),
}))

let ConfiguredTargetsList: FC<
  import('./ConfiguredTargetsList').ConfiguredTargetsListProps
>

beforeAll(async () => {
  ConfiguredTargetsList = (await import('./ConfiguredTargetsList'))
    .ConfiguredTargetsList
})

const builtIn = {
  id: 'browseros',
  name: 'BrowserOS',
  type: 'browseros',
  modelId: 'default',
} as LlmProviderConfig

const custom = {
  id: 'p1',
  name: 'My OpenAI',
  type: 'openai',
  modelId: 'gpt-5.5',
  baseUrl: 'https://api.openai.com/v1',
} as LlmProviderConfig

const agent: AcpAgent = {
  id: 'a1',
  name: 'Review agent',
  type: 'codex',
  modelId: 'gpt-5.5',
  reasoningEffort: 'medium',
  createdAt: 1,
  updatedAt: 1,
}

const coding = {
  agents: [agent],
  pageError: null,
  dismissPageError: () => {},
  deletingAgentId: null,
  handleDelete: () => {},
} as unknown as import('./coding-agents.hooks').CodingAgentsController

function render(
  props: Partial<
    import('./ConfiguredTargetsList').ConfiguredTargetsListProps
  > = {},
) {
  return renderToStaticMarkup(
    createElement(ConfiguredTargetsList, {
      providers: [builtIn, custom],
      coding,
      selectedProviderId: 'browseros',
      selectedAgentId: null,
      testingProviderId: null,
      onSelectProvider: () => {},
      onSelectAgent: () => {},
      onTestProvider: () => {},
      onEditProvider: () => {},
      onDeleteProvider: () => {},
      ...props,
    }),
  )
}

describe('ConfiguredTargetsList', () => {
  it('renders providers and agents in one list', () => {
    const html = render()
    expect(html).toContain('BrowserOS')
    expect(html).toContain('My OpenAI')
    expect(html).toContain('Review agent')
  })

  it('keeps the brand icons for each kind of target', () => {
    const html = render()
    expect(html).toContain('data-icon="browseros"')
    expect(html).toContain('data-icon="provider"')
    expect(html).toContain('data-icon="adapter"')
  })

  it('describes an agent by adapter, model and effort', () => {
    expect(render()).toContain('Codex · gpt-5.5 · medium')
  })

  it('describes a configured provider by model and base url', () => {
    expect(render()).toContain('gpt-5.5 · https://api.openai.com/v1')
  })

  // One radio group across providers and agents, so the browser enforces a
  // single default and arrow keys move between every target.
  it('puts every target in one radio group', () => {
    const html = render()
    const radios = html.match(/name="default-provider"/g) ?? []
    expect(radios).toHaveLength(3)
  })

  it('marks exactly one row as default', () => {
    const html = render()
    expect((html.match(/Default</g) ?? []).length).toBe(1)
    expect((html.match(/checked=""/g) ?? []).length).toBe(1)
  })

  it('moves the default marker when an agent is selected instead', () => {
    const html = render({ selectedProviderId: null, selectedAgentId: 'a1' })
    const defaultRow = html.slice(html.indexOf('Review agent'))
    expect(defaultRow).toContain('Default<')
  })

  it('offers an actions menu for user-owned targets only', () => {
    const html = render()
    expect(html).toContain('aria-label="Actions for My OpenAI"')
    expect(html).toContain('aria-label="Actions for Review agent"')
    // The built-in provider cannot be tested, edited or deleted.
    expect(html).not.toContain('aria-label="Actions for BrowserOS"')
  })

  // Asserting utility classes rather than layout because the reserved width
  // only exists once CSS is applied, which static markup cannot show. The
  // guard is here because dropping these turns the row back into 220px of
  // reserved trailing slots with nothing left for the name on a narrow pane.
  it('does not reserve the trailing slots on a narrow pane', () => {
    const html = render()
    expect(html).toContain('hidden w-[72px] shrink-0 justify-start sm:flex')
    expect(html).toContain('flex shrink-0 justify-end sm:w-[116px]')
  })

  it('surfaces a list-level error when the agent controller reports one', () => {
    const html = render({
      coding: { ...coding, pageError: 'agent server unreachable' } as never,
    })
    expect(html).toContain('agent server unreachable')
  })
})
