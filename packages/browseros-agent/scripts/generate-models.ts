/**
 * Fetches models.dev/api.json and generates a compact models data file
 * for BrowserOS. Run: bun scripts/generate-models.ts
 */

const API_URL = 'https://models.dev/api.json'
const OUTPUT_PATH = new URL(
  '../apps/agent/lib/llm-providers/models-dev-data.json',
  import.meta.url,
).pathname

interface ModelsDevModel {
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

interface ModelsDevProvider {
  id: string
  name: string
  npm: string
  api?: string
  doc: string
  env: string[]
  models: Record<string, ModelsDevModel>
}

interface OutputModel {
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

interface OutputProvider {
  name: string
  api?: string
  doc: string
  models: OutputModel[]
}

// models.dev ID → BrowserOS provider ID
const PROVIDER_MAP: Record<string, string> = {
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

function transformModel(model: ModelsDevModel): OutputModel | null {
  if (model.status === 'deprecated') return null

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

async function main() {
  console.log(`Fetching ${API_URL}...`)
  const response = await fetch(API_URL)
  if (!response.ok) throw new Error(`Failed to fetch: ${response.status}`)

  const data: Record<string, ModelsDevProvider> = await response.json()
  console.log(`Fetched ${Object.keys(data).length} providers`)

  const output: Record<string, OutputProvider> = {}

  for (const [modelsDevId, browserosId] of Object.entries(PROVIDER_MAP)) {
    const provider = data[modelsDevId]
    if (!provider) {
      console.warn(`Provider not found in models.dev: ${modelsDevId}`)
      continue
    }

    const models = Object.values(provider.models)
      .map(transformModel)
      .filter((m): m is OutputModel => m !== null)
      .sort((a, b) => {
        const dateA = provider.models[a.id]?.last_updated ?? ''
        const dateB = provider.models[b.id]?.last_updated ?? ''
        return dateB.localeCompare(dateA)
      })

    output[browserosId] = {
      name: provider.name,
      ...(provider.api && { api: provider.api }),
      doc: provider.doc,
      models,
    }
  }

  const totalModels = Object.values(output).reduce(
    (sum, p) => sum + p.models.length,
    0,
  )
  console.log(
    `Generated ${Object.keys(output).length} providers with ${totalModels} models`,
  )

  await Bun.write(OUTPUT_PATH, JSON.stringify(output, null, 2))
  console.log(`Written to ${OUTPUT_PATH}`)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
