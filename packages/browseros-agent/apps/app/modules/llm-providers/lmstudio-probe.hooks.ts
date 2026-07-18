import { useQuery } from '@tanstack/react-query'
import type { ProviderType } from '@/lib/llm-providers/types'

export interface LMStudioProbeModel {
  modelId: string
  contextLength: number
}

export interface LMStudioApiModel {
  id: string
  type?: string
  state?: string
  max_context_length?: number
  loaded_context_length?: number
}

export interface UseLMStudioProbeOptions {
  providerType: ProviderType | undefined
  baseUrl: string | undefined
  enabled?: boolean
}

// LM Studio's model list (and which model is loaded) changes underfoot as
// the user loads/unloads models in the app, so trust a fresh probe on every
// dialog open instead of a cached one (mirrors acp-probe.hooks.ts).
const PROBE_STALE_TIME_MS = 0

export function toLMStudioOrigin(
  baseUrl: string | undefined,
): string | undefined {
  if (!baseUrl) return undefined
  try {
    return new URL(baseUrl).origin
  } catch {
    return undefined
  }
}

export function isLMStudioProbeEnabled(opts: UseLMStudioProbeOptions): boolean {
  if (!(opts.enabled ?? true)) return false
  if (opts.providerType !== 'lmstudio') return false
  return Boolean(toLMStudioOrigin(opts.baseUrl))
}

// LM Studio's native /api/v0/models (not the OpenAI-compatible /v1/models)
// reports type, load state, and context length per model. Embeddings models
// are excluded since they can't serve chat completions.
export function parseLMStudioModels(
  data: LMStudioApiModel[],
): LMStudioProbeModel[] {
  return data
    .filter((m) => m.type === 'llm' || m.type === 'vlm')
    .sort((a, b) => {
      const loadedDiff =
        Number(b.state === 'loaded') - Number(a.state === 'loaded')
      return loadedDiff !== 0 ? loadedDiff : a.id.localeCompare(b.id)
    })
    .map((m) => ({
      modelId: m.id,
      contextLength: m.loaded_context_length ?? m.max_context_length ?? 0,
    }))
}

export function useLMStudioProbe(opts: UseLMStudioProbeOptions) {
  const origin = toLMStudioOrigin(opts.baseUrl)
  const enabled = isLMStudioProbeEnabled(opts)

  return useQuery<LMStudioProbeModel[]>({
    queryKey: ['lmstudio-probe', origin],
    enabled,
    staleTime: PROBE_STALE_TIME_MS,
    retry: false,
    queryFn: async () => {
      const res = await fetch(`${origin}/api/v0/models`)
      if (!res.ok) {
        throw new Error(`LM Studio returned ${res.status}`)
      }
      const body = (await res.json()) as { data: LMStudioApiModel[] }
      return parseLMStudioModels(body.data)
    },
  })
}
