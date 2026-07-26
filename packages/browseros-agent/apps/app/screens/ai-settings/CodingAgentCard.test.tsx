import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { AgentListItem } from '@/modules/agents/agents-page-types'

type CodingAgentCardProps = {
  agent: AgentListItem
  adapter: 'codex'
  modelLabel: string | null
  reasoningEffort: string | null
  isSelected: boolean
  deleting: boolean
  onSelect: () => void
  onDelete: (agent: AgentListItem) => void
}

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/lib/utils', () => ({
  cn: (...inputs: Array<string | false | null | undefined>) =>
    inputs.filter(Boolean).join(' '),
}))

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

mock.module('@/components/ui/badge', () => ({
  Badge: ({ children }: { children?: unknown }) =>
    createElement('span', null, children as never),
}))

let CodingAgentCard: FC<CodingAgentCardProps>

beforeAll(async () => {
  CodingAgentCard = (await import('./CodingAgentCard')).CodingAgentCard
})

const codexAgent: AgentListItem = {
  key: 'agent:codex-1',
  agentId: 'codex-1',
  name: 'agent',
  source: 'agent-harness',
  runtimeLabel: 'Codex',
  modelLabel: 'gpt-5.5',
  detail: 'medium',
  canChat: true,
  canDelete: true,
}

function renderCard(props: Partial<CodingAgentCardProps> = {}) {
  return renderToStaticMarkup(
    createElement(CodingAgentCard, {
      agent: codexAgent,
      adapter: 'codex',
      modelLabel: 'gpt-5.5',
      reasoningEffort: 'medium',
      isSelected: false,
      deleting: false,
      onSelect: () => {},
      onDelete: () => {},
      ...props,
    }),
  )
}

describe('CodingAgentCard', () => {
  it('renders a Codex agent with the provider-card shell and metadata', () => {
    const html = renderCard()

    expect(html).toContain('rounded-xl border p-4')
    expect(html).toContain('agent')
    expect(html).toContain('Codex · gpt-5.5 · medium')
    expect(html).toContain('aria-label="Codex"')
  })

  it('renders delete-only actions without the rich agent row controls', () => {
    const html = renderCard()

    expect(html).toContain('aria-label="Delete agent"')
    expect(html).not.toContain('Test')
    expect(html).not.toContain('Edit')
    expect(html).not.toContain('Running')
    expect(html).not.toContain('No messages yet')
  })

  it('disables delete and renders a spinner while deleting', () => {
    const html = renderCard({ deleting: true })

    expect(html).toContain('disabled=""')
    expect(html).toContain('animate-spin')
  })

  it('renders an unselected default-provider radio row', () => {
    const html = renderCard()

    expect(html).toContain('<label')
    expect(html).toContain('name="default-provider"')
    expect(html).toContain('id="agent-codex-1"')
    expect(html).not.toContain('checked')
    expect(html).not.toContain('DEFAULT')
    expect(html).not.toContain('shadow-md')
  })

  it('renders the checked radio, DEFAULT badge, and selected styling when selected', () => {
    const html = renderCard({ isSelected: true })

    expect(html).toContain('checked')
    expect(html).toContain('DEFAULT')
    expect(html).toContain('bg-[var(--accent-orange)]/5 shadow-md')
  })
})
