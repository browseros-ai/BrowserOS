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

/**
 * Providers whose model list cannot come from the generated catalogue.
 *
 * `chatgpt-pro` used to be here, as twelve ids typed by hand. It fell a
 * generation behind and stayed there, because adding a model meant shipping an
 * extension release. It is derived in the catalogue generator now.
 */
const CUSTOM_PROVIDER_MODELS: Partial<Record<ProviderType, ModelInfo[]>> = {
  browseros: [{ modelId: 'browseros-auto', contextLength: 200000 }],
  'openai-compatible': [],
  ollama: [],
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

/**
 * Whether the add-model dialog should show reasoning controls for this model.
 *
 * A model the catalogue does not know is assumed to reason, which is what
 * `buildChatRequestBody` already assumes when it tells the server what the
 * model can do. The dialog used to assume it for the whole chatgpt-pro
 * provider instead, because its models were absent from the catalogue; they
 * are in it now, so the question is about the model rather than the provider.
 */
export function modelSupportsReasoning(model: ModelInfo | undefined): boolean {
  return model === undefined || Boolean(model.supportsReasoning)
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
