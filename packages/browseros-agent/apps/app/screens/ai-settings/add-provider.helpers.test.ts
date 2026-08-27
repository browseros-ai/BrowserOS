import { describe, expect, it } from 'bun:test'
import { providerTemplates } from '@/lib/llm-providers/providerTemplates'
import {
  ADD_PROVIDER_CATEGORY_ORDER,
  type AddProviderEntryMeta,
  categoryForTemplate,
  groupAddProviderEntries,
  matchesQuery,
  uncategorizedTemplateIds,
} from './add-provider.helpers'

const entries: AddProviderEntryMeta[] = [
  { key: 'claude', label: 'Claude Code', category: 'agent' },
  { key: 'codex', label: 'Codex', category: 'agent' },
  { key: 'chatgpt-pro', label: 'ChatGPT', category: 'subscription' },
  { key: 'openai', label: 'OpenAI', category: 'api-key' },
  { key: 'ollama', label: 'Ollama', category: 'local' },
]

describe('categoryForTemplate', () => {
  // These three are the templates that run an OAuth flow instead of asking
  // for a key, so they must not sit in the api-key group.
  for (const id of ['chatgpt-pro', 'github-copilot', 'qwen-code']) {
    it(`${id} is a subscription sign-in`, () => {
      expect(categoryForTemplate(id)).toBe('subscription')
    })
  }

  it('groups the on-device runtimes as local', () => {
    expect(categoryForTemplate('ollama')).toBe('local')
    expect(categoryForTemplate('lmstudio')).toBe('local')
  })

  it('falls back to api-key for an unknown template', () => {
    expect(categoryForTemplate('some-new-provider')).toBe('api-key')
  })
})

describe('uncategorizedTemplateIds', () => {
  // Guards the fallback: adding a template without categorising it should
  // fail here rather than silently landing in "Bring your own API key".
  it('every shipped template has an explicit category', () => {
    expect(uncategorizedTemplateIds(providerTemplates)).toEqual([])
  })
})

describe('matchesQuery', () => {
  it('matches case-insensitively on a substring', () => {
    expect(matchesQuery('Claude Code', 'claude')).toBe(true)
    expect(matchesQuery('Claude Code', 'CODE')).toBe(true)
  })

  it('treats blank and whitespace-only queries as no filter', () => {
    expect(matchesQuery('Ollama', '')).toBe(true)
    expect(matchesQuery('Ollama', '   ')).toBe(true)
  })

  it('does not match an unrelated query', () => {
    expect(matchesQuery('Ollama', 'anthropic')).toBe(false)
  })

  // Searching for a specific ACP agent should find the tile that connects it,
  // rather than returning nothing because the label says "Custom".
  it('matches on keywords as well as the label', () => {
    expect(matchesQuery('Custom ACP agent', 'opencode', ['opencode'])).toBe(
      true,
    )
    expect(matchesQuery('Custom ACP agent', 'hermes', ['Hermes'])).toBe(true)
    expect(matchesQuery('Custom ACP agent', 'ollama', ['opencode'])).toBe(false)
  })
})

describe('groupAddProviderEntries', () => {
  it('returns groups in the fixed category order', () => {
    const groups = groupAddProviderEntries(entries)
    expect(groups.map((group) => group.category)).toEqual([
      'agent',
      'subscription',
      'api-key',
      'local',
    ])
  })

  it('keeps every entry exactly once', () => {
    const groups = groupAddProviderEntries(entries)
    const keys = groups.flatMap((group) => group.entries.map((e) => e.key))
    expect(keys.sort()).toEqual(entries.map((e) => e.key).sort())
  })

  it('drops groups that the query empties instead of rendering bare headings', () => {
    const groups = groupAddProviderEntries(entries, 'code')
    expect(groups).toHaveLength(1)
    expect(groups[0].category).toBe('agent')
    expect(groups[0].entries.map((e) => e.label)).toEqual([
      'Claude Code',
      'Codex',
    ])
  })

  it('keeps an entry whose keyword matches even when its label does not', () => {
    const withKeywords = [
      {
        key: 'custom',
        label: 'Custom ACP agent',
        category: 'agent' as const,
        keywords: ['opencode', 'Hermes'],
      },
    ]
    expect(groupAddProviderEntries(withKeywords, 'hermes')).toHaveLength(1)
  })

  it('returns nothing when the query matches no entry', () => {
    expect(groupAddProviderEntries(entries, 'zzzz')).toEqual([])
  })

  it('exposes a label for every category it can emit', () => {
    for (const category of ADD_PROVIDER_CATEGORY_ORDER) {
      const groups = groupAddProviderEntries(
        [{ key: 'k', label: 'L', category }],
        '',
      )
      expect(groups[0].label.length).toBeGreaterThan(0)
    }
  })
})
