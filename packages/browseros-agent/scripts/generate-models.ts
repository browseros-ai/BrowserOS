const API_URL = 'https://models.dev/api.json'
const OUTPUT_PATH = new URL(
  '../apps/agent/lib/llm-providers/models-dev-data.json',
  import.meta.url,
).pathname

export interface ModelsDevModel {
  id: string
  name: string
  family?: string
  attachment: boolean
  reasoning: boolean
  tool_call: boolean
  structured_output?: boolean
  modalities: { input: string[]; output: string[] }
  cost?: {
    input: number
    output: number
    cache_read?: number
    cache_write?: number
  }
  limit: { context: number; output: number; input?: number }
  status?: string
  release_date: string
  last_updated: string
}

export interface ModelsDevProvider {
  id: string
  name: string
  npm: string
  api?: string
  doc: string
  env: string[]
  models: Record<string, ModelsDevModel>
}

export interface OutputModel {
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

export interface OutputProvider {
  name: string
  api?: string
  doc: string
  models: OutputModel[]
}

export const PROVIDER_MAP: Record<string, string> = {
  anthropic: 'anthropic',
  openai: 'openai',
  google: 'google',
  openrouter: 'openrouter',
  azure: 'azure',
  'amazon-bedrock': 'bedrock',
  lmstudio: 'lmstudio',
  moonshotai: 'moonshot',
  'github-copilot': 'github-copilot',
}

const NON_CHAT_MODEL_CLASS_TERMS = [
  'embedding',
  'image',
  'audio',
  'tts',
  'transcribe',
  'whisper',
  'moderation',
]

function isNonChatModelClass(model: ModelsDevModel): boolean {
  return [model.id, model.name, model.family ?? ''].some((value) => {
    const normalized = value.toLowerCase()

    return NON_CHAT_MODEL_CLASS_TERMS.some((term) => normalized.includes(term))
  })
}

/** Converts a models.dev model into the compact BrowserOS snapshot shape. */
export function transformModel(model: ModelsDevModel): OutputModel | null {
  if (model.status === 'deprecated') return null
  if (isNonChatModelClass(model)) return null
  if (!model.modalities.input.includes('text')) return null
  if (!model.modalities.output.includes('text')) return null
  if (model.limit.context <= 0 || model.limit.output <= 0) return null

  const supportsImages =
    model.attachment || model.modalities.input.includes('image')

  return {
    id: model.id,
    name: model.name,
    contextWindow: model.limit.context,
    maxOutput: model.limit.output,
    supportsImages,
    supportsReasoning: model.reasoning,
    supportsToolCall: model.tool_call,
    ...(model.cost && {
      inputCost: model.cost.input,
      outputCost: model.cost.output,
    }),
  }
}

function assertUniqueModels(providerId: string, models: OutputModel[]) {
  const seen = new Set<string>()

  for (const model of models) {
    if (seen.has(model.id)) {
      throw new Error(`Duplicate model id for ${providerId}: ${model.id}`)
    }

    seen.add(model.id)
  }
}

/** Builds the BrowserOS provider snapshot from raw models.dev API data. */
export function generateModelsData(
  data: Record<string, ModelsDevProvider>,
  providerMap: Record<string, string> = PROVIDER_MAP,
): Record<string, OutputProvider> {
  const output: Record<string, OutputProvider> = {}

  for (const [modelsDevId, browserosId] of Object.entries(providerMap)) {
    const provider = data[modelsDevId]
    if (!provider) {
      throw new Error(`Provider not found in models.dev: ${modelsDevId}`)
    }

    const models = Object.values(provider.models)
      .map((model) => {
        const transformed = transformModel(model)

        return transformed
          ? { lastUpdated: model.last_updated, model: transformed }
          : null
      })
      .filter(
        (m): m is { lastUpdated: string; model: OutputModel } => m !== null,
      )
      .sort((a, b) => {
        const byLastUpdated = b.lastUpdated.localeCompare(a.lastUpdated)

        return byLastUpdated || a.model.id.localeCompare(b.model.id)
      })
      .map(({ model }) => model)

    assertUniqueModels(browserosId, models)

    output[browserosId] = {
      name: provider.name,
      ...(provider.api && { api: provider.api }),
      doc: provider.doc,
      models,
    }
  }

  return output
}

export function formatModelsData(
  output: Record<string, OutputProvider>,
): string {
  return `${JSON.stringify(output, null, 2)}\n`
}

/** Fetches live models.dev data and writes the checked-in BrowserOS snapshot. */
export async function main() {
  console.log(`Fetching ${API_URL}...`)
  const response = await fetch(API_URL)
  if (!response.ok) throw new Error(`Failed to fetch: ${response.status}`)

  const data: Record<string, ModelsDevProvider> = await response.json()
  console.log(`Fetched ${Object.keys(data).length} providers`)

  const output = generateModelsData(data)

  const totalModels = Object.values(output).reduce(
    (sum, p) => sum + p.models.length,
    0,
  )
  console.log(
    `Generated ${Object.keys(output).length} providers with ${totalModels} models`,
  )

  await Bun.write(OUTPUT_PATH, formatModelsData(output))
  console.log(`Written to ${OUTPUT_PATH}`)
}

if (import.meta.main) {
  main().catch((err) => {
    console.error(err)
    process.exit(1)
  })
}
