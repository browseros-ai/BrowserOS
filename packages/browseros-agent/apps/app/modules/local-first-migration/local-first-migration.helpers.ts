import { REMOVED_PROVIDER_TYPES } from '@/lib/llm-providers/removed-provider-types'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ScheduledJob } from '@/lib/schedules/scheduleTypes'

/** Payload for `POST /providers/import`. */
export interface ProviderImport {
  id: string
  type: string
  name: string
  baseUrl?: string
  modelId: string
  supportsImages?: boolean
  contextWindow: number
  temperature?: number
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
  enabled?: boolean
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

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0
}

function optionalString(value: unknown): string | undefined {
  return isNonEmptyString(value) ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

/**
 * Whether a provider can be sent to the import endpoint.
 *
 * The required fields are exactly the ones the server requires, because a
 * single entry it rejects returns 400 for the whole batch. That would be
 * permanent rather than transient: the pref backup has no migration path, so
 * the same bad entry would fail the import, block the scheduled jobs behind
 * it, and leave the done marker unset to retry forever.
 *
 * Removed types are excluded for a related reason. Storage migrations drop
 * them, the pref backup never gets that treatment, and importing one would
 * put a provider of a type the app no longer supports into the database.
 */
export function isImportableProvider(
  value: unknown,
): value is LlmProviderConfig {
  if (typeof value !== 'object' || value === null) return false
  const provider = value as Partial<LlmProviderConfig>
  return (
    isNonEmptyString(provider.id) &&
    isNonEmptyString(provider.type) &&
    !REMOVED_PROVIDER_TYPES.has(provider.type) &&
    isNonEmptyString(provider.name) &&
    isNonEmptyString(provider.modelId) &&
    optionalNumber(provider.contextWindow) !== undefined
  )
}

/** Same contract as `isImportableProvider`, for the scheduled jobs batch. */
export function isImportableJob(value: unknown): value is ScheduledJob {
  if (typeof value !== 'object' || value === null) return false
  const job = value as Partial<ScheduledJob>
  return (
    isNonEmptyString(job.id) &&
    isNonEmptyString(job.name) &&
    isNonEmptyString(job.query) &&
    (job.scheduleType === 'daily' ||
      job.scheduleType === 'hourly' ||
      job.scheduleType === 'minutes')
  )
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

/**
 * Optional fields pass through a type check rather than straight across, so a
 * provider that is well formed where it matters still imports when one of its
 * optional fields holds junk. Dropping the field lets the server apply its own
 * default; sending the wrong type would fail the whole batch.
 */
export function toProviderImport(config: LlmProviderConfig): ProviderImport {
  return {
    id: config.id,
    type: config.type,
    name: config.name,
    baseUrl: optionalString(config.baseUrl),
    modelId: config.modelId,
    supportsImages: optionalBoolean(config.supportsImages),
    contextWindow: config.contextWindow,
    temperature: optionalNumber(config.temperature),
    apiKey: optionalString(config.apiKey),
    accessKeyId: optionalString(config.accessKeyId),
    secretAccessKey: optionalString(config.secretAccessKey),
    sessionToken: optionalString(config.sessionToken),
    resourceName: optionalString(config.resourceName),
    region: optionalString(config.region),
    reasoningEffort: optionalString(config.reasoningEffort),
    reasoningSummary: optionalString(config.reasoningSummary),
    createdAt: optionalNumber(config.createdAt),
  }
}

/**
 * Jobs hold ISO strings here and epoch numbers in the database.
 *
 * An unparseable timestamp is dropped rather than sent as NaN, which would
 * fail validation and take the whole batch with it. The server then stamps its
 * own `createdAt`, so the job still lands.
 */
function toEpoch(value: unknown): number | undefined {
  if (!isNonEmptyString(value)) return undefined
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? undefined : parsed
}

export function toScheduledJobImport(job: ScheduledJob): ScheduledJobImport {
  return {
    id: job.id,
    name: job.name,
    query: job.query,
    scheduleType: job.scheduleType,
    scheduleTime: optionalString(job.scheduleTime),
    scheduleInterval: optionalNumber(job.scheduleInterval),
    enabled: optionalBoolean(job.enabled),
    providerId: optionalString(job.providerId),
    lastRunAt: toEpoch(job.lastRunAt),
    createdAt: toEpoch(job.createdAt),
  }
}
