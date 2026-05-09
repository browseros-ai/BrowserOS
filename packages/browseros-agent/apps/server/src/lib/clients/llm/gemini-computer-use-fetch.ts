/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

const GEMINI_COMPUTER_USE_MODEL_PATTERN = /computer-use/i

const GEMINI_COMPUTER_USE_TOOL = {
  computerUse: {
    environment: 'ENVIRONMENT_BROWSER',
  },
} as const

type JsonObject = Record<string, unknown>

function isJsonObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function hasComputerUseTool(tool: unknown): boolean {
  return isJsonObject(tool) && 'computerUse' in tool
}

export function isGeminiComputerUseModel(modelId: string): boolean {
  return GEMINI_COMPUTER_USE_MODEL_PATTERN.test(modelId)
}

export function addGeminiComputerUseTool(body: unknown): unknown {
  if (!isJsonObject(body)) return body

  const existingTools = Array.isArray(body.tools) ? body.tools : []
  if (existingTools.some(hasComputerUseTool)) return body

  return {
    ...body,
    tools: [GEMINI_COMPUTER_USE_TOOL, ...existingTools],
  }
}

function injectComputerUseToolIntoBody(body: BodyInit | null | undefined) {
  if (typeof body !== 'string') return body

  try {
    return JSON.stringify(addGeminiComputerUseTool(JSON.parse(body)))
  } catch {
    return body
  }
}

export function createGeminiComputerUseFetch(
  modelId: string,
): typeof globalThis.fetch | undefined {
  if (!isGeminiComputerUseModel(modelId)) return undefined

  const fetchWithComputerUse = (async (input, init) => {
    return globalThis.fetch(input, {
      ...init,
      body: injectComputerUseToolIntoBody(init?.body),
    })
  }) as typeof globalThis.fetch

  fetchWithComputerUse.preconnect = globalThis.fetch.preconnect.bind(
    globalThis.fetch,
  )

  return fetchWithComputerUse
}
