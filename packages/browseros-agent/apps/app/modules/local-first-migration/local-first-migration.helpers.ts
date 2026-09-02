import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ScheduledJob } from '@/lib/schedules/scheduleTypes'

/** Payload for `POST /llm-providers/import`. */
export interface ProviderImport {
  id: string
  type: string
  name: string
  baseUrl?: string
  modelId: string
  supportsImages: boolean
  contextWindow: number
  temperature: number
  apiKey?: string
  accessKeyId?: string
  secretAccessKey?: string
  sessionToken?: string
  resourceName?: string
  region?: string
  reasoningEffort?: string
  reasoningSummary?: string
  createdAt?: number
}

/** Payload for `POST /scheduled-jobs/import`. */
export interface ScheduledJobImport {
  id: string
  name: string
  query: string
  scheduleType: ScheduledJob['scheduleType']
  scheduleTime?: string
  scheduleInterval?: number
  enabled: boolean
  providerId?: string
  lastRunAt?: number
  createdAt?: number
}

/**
 * Reads the provider list out of the `browseros.providers` pref backup.
 *
 * The pref holds a JSON string of `LlmProvidersBackup`. It is a fallback
 * source, so anything unparseable yields nothing rather than throwing: a
 * corrupt backup must not stop the extension-storage providers from importing.
 */
export function parseProviderBackup(raw: unknown): LlmProviderConfig[] {
  if (typeof raw !== 'string' || raw.length === 0) return []
  try {
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null) return []
    const providers = (parsed as { providers?: unknown }).providers
    if (!Array.isArray(providers)) return []
    return providers.filter(
      (provider): provider is LlmProviderConfig =>
        typeof provider === 'object' &&
        provider !== null &&
        typeof (provider as LlmProviderConfig).id === 'string',
    )
  } catch {
    return []
  }
}

/**
 * Unions the two local provider sources, extension storage winning on id.
 *
 * Extension storage is what the app writes on every save, so it is the current
 * copy. The pref backup only contributes providers missing from it, which is
 * the reinstall case: extension storage was cleared and the per-profile pref
 * outlived it.
 */
export function mergeProviderSources(
  stored: readonly LlmProviderConfig[],
  backup: readonly LlmProviderConfig[],
): LlmProviderConfig[] {
  const merged = [...stored]
  const seen = new Set(stored.map((provider) => provider.id))
  for (const provider of backup) {
    if (seen.has(provider.id)) continue
    seen.add(provider.id)
    merged.push(provider)
  }
  return merged
}

export function toProviderImport(config: LlmProviderConfig): ProviderImport {
  return {
    id: config.id,
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
 * Jobs hold ISO strings here and epoch numbers in the database.
 *
 * An unparseable timestamp is dropped rather than sent as NaN, which would
 * fail validation and take the whole batch with it. The server then stamps its
 * own `createdAt`, so the job still lands.
 */
function toEpoch(value: string | undefined): number | undefined {
  if (!value) return undefined
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? undefined : parsed
}

export function toScheduledJobImport(job: ScheduledJob): ScheduledJobImport {
  return {
    id: job.id,
    name: job.name,
    query: job.query,
    scheduleType: job.scheduleType,
    scheduleTime: job.scheduleTime,
    scheduleInterval: job.scheduleInterval,
    enabled: job.enabled,
    providerId: job.providerId,
    lastRunAt: toEpoch(job.lastRunAt),
    createdAt: toEpoch(job.createdAt),
  }
}
