import { describe, expect, it } from 'bun:test'
import type { Provider } from '@/components/chat/chatComponentTypes'
import { formatSessionCompletedLabel } from './SessionCompletedFooter'

describe('formatSessionCompletedLabel', () => {
  it('returns null when provider is undefined', () => {
    expect(formatSessionCompletedLabel(undefined)).toBeNull()
  })

  it('returns the display name for an llm provider', () => {
    const provider: Provider = {
      id: 'p1',
      name: 'OpenAI GPT-4',
      type: 'openai',
      kind: 'llm',
    }
    expect(formatSessionCompletedLabel(provider)).toBe('OpenAI GPT-4')
  })

  it('joins adapter name and model label for an acp target', () => {
    const provider: Provider = {
      id: 'a1',
      name: 'Claude Code · Sonnet 3.5',
      type: 'acp',
      kind: 'acp',
      agentId: 'a1',
      adapterName: 'Claude Code',
      modelLabel: 'Sonnet 3.5',
    }
    expect(formatSessionCompletedLabel(provider)).toBe(
      'Claude Code · Sonnet 3.5',
    )
  })

  it('falls back to adapter name when model label is missing for acp', () => {
    const provider: Provider = {
      id: 'a1',
      name: 'Claude Code',
      type: 'acp',
      kind: 'acp',
      agentId: 'a1',
      adapterName: 'Claude Code',
    }
    expect(formatSessionCompletedLabel(provider)).toBe('Claude Code')
  })

  it('falls back to model label when adapter name is missing for acp', () => {
    const provider: Provider = {
      id: 'a1',
      name: 'Sonnet 3.5',
      type: 'acp',
      kind: 'acp',
      agentId: 'a1',
      modelLabel: 'Sonnet 3.5',
    }
    expect(formatSessionCompletedLabel(provider)).toBe('Sonnet 3.5')
  })

  it('falls back to provider.name when both adapter name and model label are missing for acp', () => {
    const provider: Provider = {
      id: 'a1',
      name: 'Some Agent',
      type: 'acp',
      kind: 'acp',
      agentId: 'a1',
    }
    expect(formatSessionCompletedLabel(provider)).toBe('Some Agent')
  })
})
