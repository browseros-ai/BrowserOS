import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

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

mock.module('@/components/agents/AdapterIcon', () => ({
  AdapterIcon: () => createElement('span'),
  adapterLabel: (type: string) => (type === 'codex' ? 'Codex' : 'Claude'),
}))

let CodingAgentCard: FC<import('./CodingAgentCard').CodingAgentCardProps>

beforeAll(async () => {
  CodingAgentCard = (await import('./CodingAgentCard')).CodingAgentCard
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

function renderCard(
  props: Partial<import('./CodingAgentCard').CodingAgentCardProps> = {},
) {
  return renderToStaticMarkup(
    createElement(CodingAgentCard, {
      agent,
      isSelected: false,
      deleting: false,
      onSelect: () => {},
      onDelete: () => {},
      ...props,
    }),
  )
}

describe('CodingAgentCard', () => {
  it('renders ACP identity and configuration separately', () => {
    const html = renderCard()
    expect(html).toContain('Review agent')
    expect(html).toContain('Codex · gpt-5.5 · medium')
    expect(html).toContain('aria-label="Delete Review agent"')
  })

  it('renders selected state and delete progress', () => {
    const html = renderCard({ isSelected: true, deleting: true })
    expect(html).toContain('checked')
    expect(html).toContain('DEFAULT')
    expect(html).toContain('disabled=""')
    expect(html).toContain('animate-spin')
  })

  it('shows the command, brand logo, and edit button for custom agents', () => {
    const customAgent: AcpAgent = {
      id: '00000000-0000-4000-8000-000000000002',
      name: 'My Agent',
      type: 'custom',
      // icon carries the popular-agent brand key set when the user picks one.
      customConfig: { command: 'opencode acp', icon: 'opencode' },
      createdAt: 1,
      updatedAt: 1,
    }
    const html = renderCard({ agent: customAgent, onEdit: () => {} })
    expect(html).toContain('opencode acp')
    expect(html).toContain('aria-label="Edit My Agent"')
    expect(html).toContain('aria-label="opencode"')
  })

  it('omits the edit button when onEdit is absent', () => {
    const customAgent: AcpAgent = {
      ...agent,
      type: 'custom',
      customConfig: { command: 'my-agent' },
    }
    const html = renderCard({ agent: customAgent })
    expect(html).not.toContain('aria-label="Edit')
  })
})
