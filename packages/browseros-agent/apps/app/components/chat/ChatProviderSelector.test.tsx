import { describe, expect, it } from 'bun:test'
import {
  getProviderSearchValue,
  getProviderSubtitle,
  groupProviderOptions,
} from './ChatProviderSelector.helpers'
import type { Provider } from './chatComponentTypes'

const options: Provider[] = [
  { kind: 'llm', id: 'browseros', name: 'BrowserOS', type: 'browseros' },
  {
    kind: 'llm',
    id: 'anthropic-sonnet',
    name: 'Anthropic Sonnet',
    type: 'anthropic',
  },
  {
    kind: 'acp',
    id: 'agent-claude-review',
    name: 'Review Bot',
    type: 'acp',
    adapterName: 'Claude Code',
    modelLabel: 'Haiku',
    modelControl: 'best-effort',
  },
  {
    kind: 'acp',
    id: 'agent-codex-browser',
    name: 'Browser Driver',
    type: 'acp',
    adapterName: 'Codex',
    modelLabel: 'GPT-5.5',
    modelControl: 'runtime-supported',
  },
]

describe('groupProviderOptions', () => {
  it('groups normal providers separately from created agents', () => {
    expect(groupProviderOptions(options)).toEqual([
      {
        key: 'llm',
        label: 'BrowserOS agent + your LLM',
        options: [options[0], options[1]],
      },
      {
        key: 'acp',
        label: '3p agents',
        options: [options[2], options[3]],
      },
    ])
  })
})

describe('getProviderSearchValue', () => {
  it('matches created-agent group labels and item labels', () => {
    expect(getProviderSearchValue(options[2], '3p agents')).toContain(
      '3p agents',
    )
    expect(getProviderSearchValue(options[2], '3p agents')).toContain(
      'Review Bot',
    )
    expect(getProviderSearchValue(options[2], '3p agents')).toContain(
      'Claude Code',
    )
  })
})

describe('getProviderSubtitle', () => {
  it('describes created-agent runtime context without model-target copy', () => {
    expect(getProviderSubtitle(options[2])).toBe(
      'Claude Code · Haiku · best effort',
    )
    expect(getProviderSubtitle(options[3])).toBe('Codex · GPT-5.5')
    expect(getProviderSubtitle(options[0])).toBeUndefined()
  })
})
