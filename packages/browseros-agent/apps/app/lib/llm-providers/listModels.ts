import type { ModelInfo } from '../../screens/ai-settings/models'

/**
 * @public
 */
export interface ListModelsInput {
  type: string
  baseUrl?: string
  apiKey?: string
}

/**
 * Discover the models an OpenAI-compatible endpoint actually serves via the
 * agent server's /list-models proxy. Never throws: any failure returns [] so
 * the picker silently falls back to the bundled catalog / free-form entry.
 * @public
 */
export async function listModels(
  input: ListModelsInput,
  agentServerUrl: string,
): Promise<ModelInfo[]> {
  try {
    const response = await fetch(`${agentServerUrl}/list-models`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        provider: input.type,
        baseUrl: input.baseUrl,
        apiKey: input.apiKey,
      }),
    })
    if (!response.ok) return []

    const result = (await response.json()) as {
      models?: Array<{ modelId: string; contextLength?: number }>
    }
    if (!Array.isArray(result.models)) return []

    return result.models.map((m) => ({
      modelId: m.modelId,
      contextLength: m.contextLength ?? 128000,
    }))
  } catch {
    return []
  }
}
