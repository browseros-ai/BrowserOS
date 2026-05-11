import type { ModelEntry } from './types'

export interface FetchModelsResult {
  success: boolean
  models: ModelEntry[]
  error?: string
}

/** Fetch models from OpenAI-compatible /models endpoint */
export async function fetchModelsFromApi(
  baseUrl: string,
  apiKey?: string,
): Promise<FetchModelsResult> {
  try {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    }
    if (apiKey) headers.Authorization = `Bearer ${apiKey}`

    const response = await fetch(`${baseUrl.replace(/\/$/, '')}/models`, {
      headers,
    })
    if (!response.ok) {
      return { success: false, models: [], error: `HTTP ${response.status}` }
    }

    const json = await response.json()
    // OpenAI format: { data: [{ id: 'model-name' }] }
    // Also try: { models: [{ id: ... }] } or plain array
    const ids: string[] = (json.data ?? json.models ?? [])
      .map((m: any) => m.id ?? m.model ?? m.name)
      .filter(Boolean)

    const models: ModelEntry[] = ids.map((id) => ({
      id,
      contextLength: 0,
      source: 'fetched' as const,
      fetchedAt: Date.now(),
    }))

    return { success: true, models }
  } catch (error) {
    return {
      success: false,
      models: [],
      error: error instanceof Error ? error.message : 'Unknown error',
    }
  }
}

/** Merge static catalog models into a provider's model list */
export function mergeModelLists(
  existing: ModelEntry[],
  incoming: ModelEntry[],
): ModelEntry[] {
  const existingIds = new Set(existing.map((m) => m.id))
  return [...existing, ...incoming.filter((m) => !existingIds.has(m.id))]
}
