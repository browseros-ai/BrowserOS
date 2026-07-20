import type { ProviderType } from '../../lib/llm-providers/types'
import type { ModelInfo } from './models'

/**
 * Endpoints that serve whatever the user has loaded or proxied locally, so the
 * bundled models.dev catalog is only ever a stale sample of what they can
 * actually reach. Free-form entry is the primary path for these — LM Studio
 * ships three catalog entries and users read that as the complete list.
 */
const USER_LOADED_MODEL_PROVIDERS: ReadonlySet<ProviderType> = new Set([
  'lmstudio',
  'ollama',
  'openai-compatible',
])

export function servesUserLoadedModels(providerType: ProviderType): boolean {
  return USER_LOADED_MODEL_PROVIDERS.has(providerType)
}

/**
 * Model IDs are usually pasted out of a provider's UI or docs and arrive with
 * surrounding whitespace, which the provider's API then rejects as unknown.
 */
export function normalizeModelId(raw: string): string {
  return raw.trim()
}

/**
 * True when the search text is a usable model ID the catalog doesn't already
 * offer, meaning the picker must surface it as an explicit free-form choice.
 * Matched against the whole catalog rather than the fuzzy-filtered view so an
 * exact hit never renders twice.
 */
export function shouldOfferCustomModel(
  search: string,
  catalog: readonly ModelInfo[],
): boolean {
  const modelId = normalizeModelId(search)
  if (!modelId) return false
  return !catalog.some((model) => model.modelId === modelId)
}

/**
 * Guidance rendered under the Model field. Scoped to providers that show a
 * catalog they cannot actually vouch for: a short list reads as authoritative,
 * which is how LM Studio users concluded their loaded models were unsupported.
 * A provider with no catalog already renders a free-form input, and a cloud
 * catalog is authoritative, so both get null and stay uncluttered.
 */
export function getIncompleteCatalogHint(
  providerType: ProviderType,
  catalogSize: number,
  providerName?: string,
): string | null {
  if (catalogSize === 0) return null
  if (!servesUserLoadedModels(providerType)) return null
  return `${providerName ?? 'This provider'} lists only common models — paste the exact ID of any model you have loaded.`
}
