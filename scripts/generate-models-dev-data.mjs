#!/usr/bin/env node
/**
 * Fetch live data from models.dev and regenerate
 * packages/browseros-agent/apps/agent/lib/llm-providers/models-dev-data.json
 *
 * Usage (from repo root):
 *   node scripts/generate-models-dev-data.mjs
 *
 * Requires Node >= 18 (uses built-in fetch).
 */

import { writeFileSync, readFileSync } from 'fs'
import { join, dirname } from 'path'
import { fileURLToPath } from 'url'

const __dirname = dirname(fileURLToPath(import.meta.url))

const MODELS_DEV_URL = 'https://models.dev/api.json'
const OUTPUT_PATH = join(
  __dirname,
  '../packages/browseros-agent/apps/agent/lib/llm-providers/models-dev-data.json',
)

/**
 * Map BrowserOS provider key → models.dev provider key.
 * Keys not present in models.dev are sourced from the current snapshot.
 */
const PROVIDER_MAP = {
  anthropic: 'anthropic',
  openai: 'openai',
  google: 'google',
  openrouter: 'openrouter',
  azure: 'azure',
  bedrock: 'amazon-bedrock',
  lmstudio: 'lmstudio',
  'github-copilot': 'github-copilot',
}

function transformModel(modelId, m) {
  const out = {
    id: m.id ?? modelId,
    name: m.name ?? modelId,
    contextWindow: m.limit?.context ?? 0,
    maxOutput: m.limit?.output ?? 0,
    supportsImages: Boolean(m.attachment),
    supportsReasoning: Boolean(m.reasoning),
    supportsToolCall: Boolean(m.tool_call),
  }
  if (m.cost?.input != null) out.inputCost = m.cost.input
  if (m.cost?.output != null) out.outputCost = m.cost.output
  return out
}

function transformProvider(devKey, provider) {
  const modelsDict = provider.models ?? {}
  const models = Object.entries(modelsDict)
    .filter(([, m]) => typeof m === 'object' && m !== null)
    .map(([id, m]) => transformModel(id, m))

  const out = {
    name: provider.name ?? devKey,
    doc: provider.doc ?? '',
    models,
  }
  if (provider.api) out.api = provider.api
  return out
}

async function main() {
  console.log(`Fetching ${MODELS_DEV_URL} …`)
  const res = await fetch(MODELS_DEV_URL)
  if (!res.ok) throw new Error(`HTTP ${res.status} from models.dev`)
  const apiData = await res.json()
  console.log(`Got ${Object.keys(apiData).length} providers`)

  // Load existing snapshot to preserve providers absent from models.dev
  const existing = JSON.parse(readFileSync(OUTPUT_PATH, 'utf8'))

  const output = {}

  for (const [bosKey, devKey] of Object.entries(PROVIDER_MAP)) {
    if (!(devKey in apiData)) {
      console.warn(`WARN: ${devKey} not in models.dev — skipping ${bosKey}`)
      continue
    }
    output[bosKey] = transformProvider(devKey, apiData[devKey])
    console.log(`  ${bosKey}: ${output[bosKey].models.length} models`)
  }

  // Preserve providers not tracked by models.dev
  for (const key of Object.keys(existing)) {
    if (!(key in output)) {
      output[key] = existing[key]
      console.log(`  ${key}: ${existing[key].models.length} models (preserved)`)
    }
  }

  writeFileSync(OUTPUT_PATH, JSON.stringify(output, null, 2) + '\n')
  console.log(`\nWrote ${OUTPUT_PATH}`)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
