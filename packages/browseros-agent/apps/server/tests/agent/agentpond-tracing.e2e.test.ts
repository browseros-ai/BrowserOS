/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterAll, describe, expect, it, mock } from 'bun:test'
import { mkdir, mkdtemp, readdir, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MockLanguageModelV3 } from 'ai/test'

const PROMPT_SENTINEL = 'browseros-agentpond-private-prompt'
const RESPONSE_SENTINEL = 'browseros-agentpond-private-response'
const originalEnvironment = {
  projectId: process.env.AGENTPOND_PROJECT_ID,
  provider: process.env.FILES_SDK_PROVIDER,
  root: process.env.FILES_SDK_ROOT,
}
const configuredRoot = process.env.AGENTPOND_E2E_ROOT
const traceRoot =
  configuredRoot ?? (await mkdtemp(join(tmpdir(), 'browseros-agentpond-')))

await mkdir(traceRoot, { recursive: true })
process.env.AGENTPOND_PROJECT_ID = 'default-project'
process.env.FILES_SDK_PROVIDER = 'fs'
process.env.FILES_SDK_ROOT = traceRoot

const model = new MockLanguageModelV3({
  provider: 'browseros-agentpond-e2e',
  modelId: 'browseros-agentpond-test-model',
  doGenerate: {
    content: [{ type: 'text', text: RESPONSE_SENTINEL }],
    finishReason: { raw: 'stop', unified: 'stop' },
    usage: {
      inputTokens: { cacheRead: 0, cacheWrite: 0, noCache: 7, total: 7 },
      outputTokens: { reasoning: 0, text: 4, total: 4 },
    },
    warnings: [],
  },
})

mock.module('../../src/agent/provider-factory', () => ({
  createLanguageModel: async () => ({ model }),
}))

const { AiSdkAgent } = await import('../../src/agent/ai-sdk-agent')
const { flushAgentPondTracing, shutdownAgentPondTracing } = await import(
  '../../src/agent/agentpond-tracing'
)

afterAll(async () => {
  mock.restore()
  for (const [name, value] of Object.entries({
    AGENTPOND_PROJECT_ID: originalEnvironment.projectId,
    FILES_SDK_PROVIDER: originalEnvironment.provider,
    FILES_SDK_ROOT: originalEnvironment.root,
  })) {
    if (value === undefined) delete process.env[name]
    else process.env[name] = value
  }
  if (!configuredRoot) await rm(traceRoot, { force: true, recursive: true })
})

describe('BrowserOS AgentPond tracing', () => {
  it('reads back a content-free trace from the production agent path', async () => {
    const agent = await AiSdkAgent.create({
      browserSession: {} as never,
      resolvedConfig: {
        apiKey: 'fixture',
        baseUrl: 'http://127.0.0.1',
        chatMode: true,
        conversationId: 'agentpond-e2e',
        model: 'browseros-agentpond-test-model',
        provider: 'openai-compatible',
      },
    })

    const result = await agent.toolLoopAgent.generate({
      prompt: PROMPT_SENTINEL,
    })
    expect(result.text).toBe(RESPONSE_SENTINEL)
    expect(result.totalUsage.totalTokens).toBe(11)

    await flushAgentPondTracing()
    const traceKeys = (await readdir(traceRoot, { recursive: true })).filter(
      (key) => key.endsWith('.json') && !key.endsWith('.meta.json'),
    )
    expect(traceKeys.length).toBeGreaterThan(0)
    const rawPayload = (
      await Promise.all(
        traceKeys.map((key) => readFile(join(traceRoot, key), 'utf8')),
      )
    ).join('\n')

    expect(rawPayload).toContain('browseros-agentpond-test-model')
    expect(rawPayload).toContain('llm.token_count.total')
    expect(rawPayload).not.toContain(PROMPT_SENTINEL)
    expect(rawPayload).not.toContain(RESPONSE_SENTINEL)

    await agent.dispose()
    await shutdownAgentPondTracing()
  })
})
