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
  headers?: Record<string, string> | null
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
    headers: orUndefined(row.headers),
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
    headers: config.headers,
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
