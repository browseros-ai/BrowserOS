import data from './models-dev-data.json'

export interface ModelsDevModel {
  id: string
  name: string
  contextWindow: number
  maxOutput: number
  supportsImages: boolean
  supportsReasoning: boolean
  supportsToolCall: boolean
  inputCost?: number
  outputCost?: number
}

export interface ModelsDevProvider {
  name: string
  api?: string
  doc: string
  models: ModelsDevModel[]
}

const modelsDevData: Record<string, ModelsDevProvider> = data as Record<
  string,
  ModelsDevProvider
>

export function getModelsDevProvider(
  providerId: string,
): ModelsDevProvider | undefined {
  return modelsDevData[providerId]
}

export function getModelsDevModels(providerId: string): ModelsDevModel[] {
  return modelsDevData[providerId]?.models ?? []
}
