import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'

/**
 * How a user connects the thing, which is the decision they are actually
 * making when they scan this list. Grouping by vendor would not help: the
 * question in front of them is "what will this ask me for".
 */
export type AddProviderCategory = 'agent' | 'subscription' | 'api-key' | 'local'

export const ADD_PROVIDER_CATEGORY_ORDER: readonly AddProviderCategory[] = [
  'agent',
  'subscription',
  'api-key',
  'local',
] as const

export const ADD_PROVIDER_CATEGORY_LABELS: Record<AddProviderCategory, string> =
  {
    agent: 'Coding agents',
    subscription: 'Sign in with a subscription',
    'api-key': 'Bring your own API key',
    local: 'Runs on this machine',
  }

/**
 * Categories for every provider template that ships today. The three
 * subscription entries are exactly the three that run an OAuth flow rather
 * than asking for a key.
 *
 * A template with no entry here falls back to `api-key`, which is the safe
 * default (it is what an unknown hosted provider almost certainly is), and
 * `uncategorizedTemplateIds` exists so a test can fail the build instead of
 * letting a new template quietly land in the wrong group.
 */
const TEMPLATE_CATEGORIES: Record<string, AddProviderCategory> = {
  'chatgpt-pro': 'subscription',
  'github-copilot': 'subscription',
  'qwen-code': 'subscription',
  openai: 'api-key',
  anthropic: 'api-key',
  google: 'api-key',
  openrouter: 'api-key',
  moonshot: 'api-key',
  azure: 'api-key',
  bedrock: 'api-key',
  'openai-compatible': 'api-key',
  ollama: 'local',
  lmstudio: 'local',
}

export function categoryForTemplate(templateId: string): AddProviderCategory {
  return TEMPLATE_CATEGORIES[templateId] ?? 'api-key'
}

export function uncategorizedTemplateIds(
  templates: readonly Pick<ProviderTemplate, 'id'>[],
): string[] {
  return templates
    .map((template) => template.id)
    .filter((id) => !(id in TEMPLATE_CATEGORIES))
}

export interface AddProviderEntryMeta {
  key: string
  label: string
  category: AddProviderCategory
  /**
   * Extra terms this entry should answer to. The custom-agent entry uses it so
   * searching for a specific ACP agent by name finds the tile that connects
   * it, rather than returning nothing.
   */
  keywords?: readonly string[]
}

export interface AddProviderGroup<T extends AddProviderEntryMeta> {
  category: AddProviderCategory
  label: string
  entries: T[]
}

export function matchesQuery(
  label: string,
  query: string,
  keywords: readonly string[] = [],
): boolean {
  const trimmed = query.trim().toLowerCase()
  if (!trimmed) return true
  return [label, ...keywords].some((term) =>
    term.toLowerCase().includes(trimmed),
  )
}

/**
 * Filters by the search query, then buckets into the fixed category order.
 * Groups that end up empty are dropped rather than rendered as bare headings.
 */
export function groupAddProviderEntries<T extends AddProviderEntryMeta>(
  entries: readonly T[],
  query = '',
): AddProviderGroup<T>[] {
  const matched = entries.filter((entry) =>
    matchesQuery(entry.label, query, entry.keywords),
  )

  return ADD_PROVIDER_CATEGORY_ORDER.map((category) => ({
    category,
    label: ADD_PROVIDER_CATEGORY_LABELS[category],
    entries: matched.filter((entry) => entry.category === category),
  })).filter((group) => group.entries.length > 0)
}
