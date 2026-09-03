import { isProviderType } from '@/lib/llm-providers/providerTemplates'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'

/**
 * A provider row as the server returns it.
 *
 * Absent values are null rather than undefined, and credentials are not here
 * at all: the server reports only whether each is set, so a key cannot reach
 * a surface that has no use for it.
 */
export interface ProviderRow {
  id: string
  kind: 'llm' | 'acp'
  type: string
  name: string
  baseUrl: string | null
  // Nullable since the table holds coding agents too, and those carry neither.
  modelId: string | null
  supportsImages: boolean
  contextWindow: number | null
  temperature: number
  hasApiKey: boolean
  hasAccessKeyId: boolean
  hasSecretAccessKey: boolean
  hasSessionToken: boolean
  resourceName: string | null
  region: string | null
  reasoningEffort: string | null
  reasoningSummary: string | null
  createdAt: number
  updatedAt: number
}

function orUndefined<T>(value: T | null): T | undefined {
  return value ?? undefined
}

function toReasoningSummary(
  value: string | null,
): LlmProviderConfig['reasoningSummary'] {
  if (value === 'auto' || value === 'concise' || value === 'detailed') {
    return value
  }
  return undefined
}

/**
 * Converts a stored row to the config shape the app works in.
 *
 * Returns null for a type this build does not know, which happens after a
 * downgrade from a build that added one. The row stays in the database and
 * reappears on upgrade; showing it would push an unknown key through the icon
 * map, the template lookup and the default base URLs, all keyed by the union.
 */
export function toProviderConfig(row: ProviderRow): LlmProviderConfig | null {
  // Coding agents share this table and this endpoint, and are served to the
  // surfaces that want them through their own hook. Filtering on kind says
  // that; leaning on the unknown-type guard below to drop them happened to
  // work and said something else entirely.
  if (row.kind !== 'llm') return null
  if (!isProviderType(row.type)) return null
  if (row.modelId === null || row.contextWindow === null) return null

  return {
    id: row.id,
    type: row.type,
    name: row.name,
    baseUrl: orUndefined(row.baseUrl),
    modelId: row.modelId,
    supportsImages: row.supportsImages,
    contextWindow: row.contextWindow,
    temperature: row.temperature,
    hasApiKey: row.hasApiKey,
    hasAccessKeyId: row.hasAccessKeyId,
    hasSecretAccessKey: row.hasSecretAccessKey,
    hasSessionToken: row.hasSessionToken,
    resourceName: orUndefined(row.resourceName),
    region: orUndefined(row.region),
    reasoningEffort: orUndefined(row.reasoningEffort),
    reasoningSummary: toReasoningSummary(row.reasoningSummary),
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

export function toProviderConfigs(rows: readonly ProviderRow[]) {
  return rows
    .map(toProviderConfig)
    .filter((config): config is LlmProviderConfig => config !== null)
}

/** The request body for a provider write. `id` travels in the path instead. */
export function toProviderPayload(config: LlmProviderConfig) {
  return {
    type: config.type,
    name: config.name,
    baseUrl: config.baseUrl,
    modelId: config.modelId,
    supportsImages: config.supportsImages,
    contextWindow: config.contextWindow,
    temperature: config.temperature,
    apiKey: config.apiKey,
    accessKeyId: config.accessKeyId,
    secretAccessKey: config.secretAccessKey,
    sessionToken: config.sessionToken,
    resourceName: config.resourceName,
    region: config.region,
    reasoningEffort: config.reasoningEffort,
    reasoningSummary: config.reasoningSummary,
    createdAt: config.createdAt,
  }
}

/**
 * Provider types where a second copy makes no sense, because the credential is
 * an OAuth grant held once per account rather than a key the user can hold
 * several of.
 */
const SINGLE_INSTANCE_PROVIDER_TYPES = new Set<LlmProviderConfig['type']>([
  'chatgpt-pro',
  'github-copilot',
  'qwen-code',
])

export interface ProviderSavePlan {
  saved: LlmProviderConfig
  removedIds: string[]
}

/**
 * Works out the writes a save turns into.
 *
 * Extension storage took the whole list at once, so collapsing an earlier copy
 * of a single-instance provider fell out of replacing the array. Over HTTP the
 * save is one PUT, so the copies it displaces have to be deleted explicitly,
 * and the surviving id has to be the earlier one so the row keeps its
 * identity rather than accumulating a new one per sign-in.
 */
export function planProviderSave(
  current: readonly LlmProviderConfig[],
  provider: LlmProviderConfig,
  now = Date.now(),
): ProviderSavePlan {
  if (!SINGLE_INSTANCE_PROVIDER_TYPES.has(provider.type)) {
    const existing = current.find((candidate) => candidate.id === provider.id)
    return {
      saved: existing
        ? { ...provider, updatedAt: now }
        : { ...provider, createdAt: now, updatedAt: now },
      removedIds: [],
    }
  }

  const existing =
    current.find((candidate) => candidate.id === provider.id) ??
    current.find((candidate) => candidate.type === provider.type)

  const saved: LlmProviderConfig = {
    ...provider,
    id: existing?.id ?? provider.id,
    createdAt: existing?.createdAt ?? now,
    updatedAt: now,
  }

  const removedIds = current
    .filter(
      (candidate) =>
        candidate.id !== saved.id &&
        (candidate.type === provider.type || candidate.id === provider.id),
    )
    .map((candidate) => candidate.id)

  return { saved, removedIds }
}

/**
 * Ids present before a save but not after.
 *
 * Saving a single-instance provider (an OAuth one, where a second copy makes
 * no sense) collapses any earlier copy into the saved one. In extension
 * storage that fell out of writing the whole list at once; over HTTP the
 * removals have to be issued explicitly.
 */
export function removedProviderIds(
  before: readonly LlmProviderConfig[],
  after: readonly LlmProviderConfig[],
): string[] {
  const kept = new Set(after.map((provider) => provider.id))
  return before
    .filter((provider) => !kept.has(provider.id))
    .map((provider) => provider.id)
}
