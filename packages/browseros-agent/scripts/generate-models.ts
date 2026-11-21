const API_URL = 'https://models.dev/api.json'
const OUTPUT_PATH = new URL(
  '../apps/app/lib/llm-providers/models-dev-data.json',
  import.meta.url,
).pathname

export interface ModelsDevModel {
  id: string
  name: string
  family?: string
  attachment: boolean
  reasoning: boolean
  reasoning_options?: Array<{ type: string; values?: string[] }>
  tool_call: boolean
  structured_output?: boolean
  temperature?: boolean
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

/**
 * Per-model reasoning control surface from models.dev. `effort` carries the
 * allowed level values; `toggle` and `budget_tokens` carry none.
 */
export type ReasoningControlType = 'effort' | 'toggle' | 'budget_tokens'

export interface ReasoningControl {
  type: ReasoningControlType
  values: string[]
}

export interface OutputModel {
  id: string
  name: string
  contextWindow: number
  maxOutput: number
  supportsImages: boolean
  supportsReasoning: boolean
  supportsToolCall: boolean
  /** Whether the model accepts a temperature parameter. Reasoning models often do not. */
  supportsTemperature: boolean
  /** Reasoning control descriptors (effort levels / toggle / budget). Absent when the model exposes none. */
  reasoningControls?: ReasoningControl[]
  inputCost?: number
  outputCost?: number
}

export interface OutputProvider {
  name: string
  api?: string
  doc: string
  models: OutputModel[]
}

/**
 * BrowserOS id for the provider added by signing in with a ChatGPT
 * subscription. models.dev has no entry for it: it describes the platform API,
 * and a subscription reaches the Codex backend instead. So this one is derived
 * rather than mapped, from the openai catalogue plus the corrections below.
 */
export const CHATGPT_SUBSCRIPTION_ID = 'chatgpt-pro'

/**
 * Lowest GPT generation the Codex backend serves.
 *
 * A floor rather than a list of generations, on purpose. The hand-written
 * catalogue this replaces went stale the day a new generation shipped, and
 * naming the generations here would have reintroduced exactly that. Below the
 * floor sit GPT-4.x, the o-series, realtime and embedding models, none of
 * which that backend accepts.
 */
const CODEX_MIN_GENERATION = 5

// The ChatGPT backend returned model_not_found for these IDs on 2026-11-21.
// Checked with one ChatGPT account. This filter does not apply to the OpenAI API.
const CODEX_REJECTED_MODELS = new Set([
  'gpt-5.3-codex',
  'gpt-5.3-codex-spark',
  'gpt-5.4',
  'gpt-5.4-mini',
  'gpt-5.4-nano',
  'gpt-5.5',
  'gpt-5.6',
])

// Include Codex-only models and use subscription-specific limits where known.
const CODEX_MODELS: DatedModel[] = [
  {
    lastUpdated: '2026-09-29',
    model: {
      ...codexVariant('gpt-6.1-sol', 'GPT-6.1 Sol', 500000),
      supportsTemperature: false,
      reasoningControls: [
        { type: 'effort', values: ['low', 'medium', 'high', 'xhigh', 'max'] },
      ],
    },
  },
  {
    lastUpdated: '2025-12-11',
    model: codexVariant('gpt-5.2-codex', 'GPT-5.2 Codex', 400000),
  },
  {
    lastUpdated: '2025-11-13',
    model: codexVariant('gpt-5.1-codex-max', 'GPT-5.1 Codex Max', 400000),
  },
  {
    lastUpdated: '2025-11-13',
    model: codexVariant('gpt-5.1-codex', 'GPT-5.1 Codex', 400000),
  },
  {
    lastUpdated: '2025-11-13',
    model: codexVariant('gpt-5.1-codex-mini', 'GPT-5.1 Codex Mini', 400000),
  },
]

function codexVariant(
  id: string,
  name: string,
  contextWindow: number,
): OutputModel {
  return {
    id,
    name,
    contextWindow,
    maxOutput: 128000,
    supportsImages: true,
    supportsReasoning: true,
    supportsToolCall: true,
    supportsTemperature: true,
    reasoningControls: [
      { type: 'effort', values: ['none', 'low', 'medium', 'high', 'xhigh'] },
    ],
  }
}

/** The generation in a `gpt-<n>` id, or null when the id is not of that shape. */
function gptGeneration(id: string): number | null {
  const match = /^gpt-(\d+)/.exec(id)
  return match ? Number(match[1]) : null
}

/**
 * Whether the Codex backend both serves this model and can stream it.
 *
 * The `-pro` tier answers on a background channel, while the Codex request
 * wrapper sets `stream: true` on every request, so offering one would hand the
 * user an option that fails when used. Remove that condition once the Codex
 * path can read a non-streaming response.
 */
function isServedByCodexChat(model: OutputModel): boolean {
  const generation = gptGeneration(model.id)
  if (generation === null || generation < CODEX_MIN_GENERATION) return false
  return !model.id.endsWith('-pro') && !CODEX_REJECTED_MODELS.has(model.id)
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

  const reasoningControls = (model.reasoning_options ?? [])
    .filter((o): o is { type: ReasoningControlType; values?: string[] } =>
      REASONING_CONTROL_TYPES.includes(o.type as ReasoningControlType),
    )
    .map((o) => ({ type: o.type, values: o.values ?? [] }))

  return {
    id: model.id,
    name: model.name,
    contextWindow: model.limit.context,
    maxOutput: model.limit.output,
    supportsImages,
    supportsReasoning: model.reasoning,
    supportsToolCall: model.tool_call,
    supportsTemperature: model.temperature ?? true,
    ...(reasoningControls.length > 0 && { reasoningControls }),
    ...(model.cost && {
      inputCost: model.cost.input,
      outputCost: model.cost.output,
    }),
  }
}

const REASONING_CONTROL_TYPES: ReasoningControlType[] = [
  'effort',
  'toggle',
  'budget_tokens',
]

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

    const dated = datedModels(provider)
    const models = sortByRecency(dated)

    assertUniqueModels(browserosId, models)

    output[browserosId] = {
      name: provider.name,
      ...(provider.api && { api: provider.api }),
      doc: provider.doc,
      models,
    }
  }

  const openai = data.openai
  if (openai) {
    output[CHATGPT_SUBSCRIPTION_ID] = deriveChatGptSubscription(openai)
    assertUniqueModels(
      CHATGPT_SUBSCRIPTION_ID,
      output[CHATGPT_SUBSCRIPTION_ID].models,
    )
  }

  return output
}

type DatedModel = { lastUpdated: string; model: OutputModel }

function datedModels(provider: ModelsDevProvider): DatedModel[] {
  return Object.values(provider.models)
    .map((model) => {
      const transformed = transformModel(model)

      return transformed
        ? { lastUpdated: model.last_updated, model: transformed }
        : null
    })
    .filter((m): m is DatedModel => m !== null)
}

function sortByRecency(models: DatedModel[]): OutputModel[] {
  return [...models]
    .sort((a, b) => {
      const byLastUpdated = b.lastUpdated.localeCompare(a.lastUpdated)

      return byLastUpdated || a.model.id.localeCompare(b.model.id)
    })
    .map(({ model }) => model)
}

/**
 * The catalogue for a ChatGPT subscription.
 *
 * Built from the openai catalogue rather than listed by hand, so a new
 * generation is selectable the day models.dev carries it. Two corrections are
 * applied on top: drop models the Codex chat path cannot serve, then use
 * Codex's model entries to include its variants and apply subscription limits.
 */
export function deriveChatGptSubscription(
  openai: ModelsDevProvider,
): OutputProvider {
  const served = new Map(
    [...datedModels(openai), ...CODEX_MODELS]
      .filter(({ model }) => isServedByCodexChat(model))
      .map((entry) => [entry.model.id, entry]),
  )

  return {
    name: 'ChatGPT',
    doc: openai.doc,
    models: sortByRecency([...served.values()]),
  }
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
