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
    id: 'acp:claude:haiku:medium',
    name: 'Claude Code Haiku',
    type: 'acp',
    modelControl: 'best-effort',
  },
  {
    kind: 'acp',
    id: 'acp:codex:gpt-5.5:medium',
    name: 'Codex GPT-5.5',
    type: 'acp',
    modelControl: 'runtime-supported',
  },
]

describe('groupProviderOptions', () => {
  it('groups normal providers separately from ACP models', () => {
    expect(groupProviderOptions(options)).toEqual([
      {
        key: 'llm',
        label: 'AI Providers',
        options: [options[0], options[1]],
      },
      {
        key: 'acp',
        label: 'ACP Models',
        options: [options[2], options[3]],
      },
    ])
  })
})

describe('getProviderSearchValue', () => {
  it('matches ACP group labels and item labels', () => {
    expect(getProviderSearchValue(options[2], 'ACP Models')).toContain(
      'ACP Models',
    )
    expect(getProviderSearchValue(options[2], 'ACP Models')).toContain(
      'Claude Code Haiku',
    )
  })
})

describe('getProviderSubtitle', () => {
  it('does not present best-effort ACP models as guaranteed routing', () => {
    expect(getProviderSubtitle(options[2])).toBe('ACP model · best effort')
    expect(getProviderSubtitle(options[3])).toBe('ACP model')
    expect(getProviderSubtitle(options[0])).toBeUndefined()
  })
})
