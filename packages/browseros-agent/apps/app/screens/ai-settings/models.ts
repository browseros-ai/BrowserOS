import {
  getModelsDevModels,
  type ModelsDevModel,
  type ReasoningControl,
} from '../../lib/llm-providers/models-dev'
import type { ProviderType } from '../../lib/llm-providers/types'

export interface ModelInfo {
  modelId: string
  contextLength: number
  supportsImages?: boolean
  supportsReasoning?: boolean
  supportsToolCall?: boolean
  supportsTemperature?: boolean
  reasoningControls?: ReasoningControl[]
}

const CUSTOM_PROVIDER_MODELS: Partial<Record<ProviderType, ModelInfo[]>> = {
  browseros: [{ modelId: 'browseros-auto', contextLength: 200000 }],
  'openai-compatible': [],
  ollama: [],
  'chatgpt-pro': [
    { modelId: 'gpt-5.5', contextLength: 1050000 },
    { modelId: 'gpt-5.4', contextLength: 1050000 },
    { modelId: 'gpt-5.4-mini', contextLength: 400000 },
    { modelId: 'gpt-5.4-nano', contextLength: 400000 },
    { modelId: 'gpt-5.3-codex', contextLength: 400000 },
    { modelId: 'gpt-5.3-codex-spark', contextLength: 128000 },
    { modelId: 'gpt-5.2-codex', contextLength: 400000 },
    { modelId: 'gpt-5.2', contextLength: 400000 },
    { modelId: 'gpt-5.1-codex', contextLength: 400000 },
    { modelId: 'gpt-5.1-codex-max', contextLength: 400000 },
    { modelId: 'gpt-5.1-codex-mini', contextLength: 400000 },
    { modelId: 'gpt-5.1', contextLength: 400000 },
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
    supportsTemperature: m.supportsTemperature,
    reasoningControls: m.reasoningControls,
  }
}

export function getModelsForProvider(providerType: ProviderType): ModelInfo[] {
  const custom = CUSTOM_PROVIDER_MODELS[providerType]
  if (custom !== undefined) return custom

  return getModelsDevModels(providerType).map(fromModelsDevModel)
}

export function getModelInfo(
  providerType: ProviderType,
  modelId: string,
): ModelInfo | undefined {
  return getModelsForProvider(providerType).find((m) => m.modelId === modelId)
}

export function getModelContextLength(
  providerType: ProviderType,
  modelId: string,
): number | undefined {
  return getModelInfo(providerType, modelId)?.contextLength
}

const DEFAULT_EFFORT_VALUES = ['low', 'medium', 'high']

/** Whether the add-model dialog should show reasoning controls for this model. */
export function modelSupportsReasoning(
  model: ModelInfo | undefined,
  providerType: ProviderType,
): boolean {
  // chatgpt-pro models are not in the catalog snapshot but always reason.
  return Boolean(model?.supportsReasoning) || providerType === 'chatgpt-pro'
}

/**
 * The reasoning effort levels to offer for a model: the catalog's per-model
 * effort values when present, else a sensible default (toggle/budget models
 * have no effort levels but the server still maps effort to a budget).
 */
export function getReasoningEffortOptions(
  model: ModelInfo | undefined,
): string[] {
  const effort = model?.reasoningControls?.find((c) => c.type === 'effort')
  return effort?.values.length ? effort.values : DEFAULT_EFFORT_VALUES
}
