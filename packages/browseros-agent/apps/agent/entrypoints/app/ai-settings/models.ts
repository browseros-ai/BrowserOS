import {
  getModelsDevModels,
  type ModelsDevModel,
} from '@/lib/llm-providers/models-dev'
import type { ProviderType } from '@/lib/llm-providers/types'

export interface ModelInfo {
  modelId: string
  contextLength: number
  supportsImages?: boolean
  supportsReasoning?: boolean
  supportsToolCall?: boolean
}

const CUSTOM_PROVIDER_MODELS: Partial<Record<ProviderType, ModelInfo[]>> = {
  browseros: [{ modelId: 'browseros-auto', contextLength: 200000 }],
  'openai-compatible': [],
  ollama: [],
  'chatgpt-pro': [
    { modelId: 'gpt-5.4', contextLength: 400000 },
    { modelId: 'gpt-5.3-codex', contextLength: 400000 },
    { modelId: 'gpt-5.2-codex', contextLength: 400000 },
    { modelId: 'gpt-5.2', contextLength: 200000 },
    { modelId: 'gpt-5.1-codex', contextLength: 400000 },
    { modelId: 'gpt-5.1-codex-max', contextLength: 400000 },
    { modelId: 'gpt-5.1-codex-mini', contextLength: 400000 },
    { modelId: 'gpt-5.1', contextLength: 200000 },
  ],
  'qwen-code': [
    { modelId: 'coder-model', contextLength: 1000000 },
    { modelId: 'qwen3-coder-plus', contextLength: 1000000 },
    { modelId: 'qwen3-coder-flash', contextLength: 1000000 },
    { modelId: 'qwen3.5-plus', contextLength: 1000000 },
  ],
}

function fromModelsDevModel(m: ModelsDevModel): ModelInfo {
  return {
    modelId: m.id,
    contextLength: m.contextWindow,
    supportsImages: m.supportsImages,
    supportsReasoning: m.supportsReasoning,
    supportsToolCall: m.supportsToolCall,
  }
}

export function getModelsForProvider(providerType: ProviderType): ModelInfo[] {
  const custom = CUSTOM_PROVIDER_MODELS[providerType]
  if (custom !== undefined) return custom

  return getModelsDevModels(providerType).map(fromModelsDevModel)
}

export function getModelContextLength(
  providerType: ProviderType,
  modelId: string,
): number | undefined {
  const models = getModelsForProvider(providerType)
  const model = models.find((m) => m.modelId === modelId)
  return model?.contextLength
}
