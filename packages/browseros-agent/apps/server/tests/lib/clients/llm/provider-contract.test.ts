/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Contract test: every provider declared in LLM_PROVIDERS must be registered in
 * BOTH factory registries (server `createLLMProvider` and agent
 * `createLanguageModel`). This catches a "half-added" provider — one wired into
 * the schema/UI but missing from a factory map, which otherwise only surfaces
 * at runtime as "Unknown provider".
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { createLanguageModel } from '../../../../src/agent/provider-factory'
import { createLLMProvider } from '../../../../src/lib/clients/llm/provider'

// A config populated with every field any provider might require, so the only
// remaining failure mode is "Unknown provider" (missing factory registration).
function kitchenSinkConfig(provider: string) {
  return {
    provider,
    model: 'test-model',
    apiKey: 'test-key',
    baseUrl: 'https://example.com/v1',
    resourceName: 'test-resource',
    region: 'us-east-1',
    accessKeyId: 'test-access-key',
    secretAccessKey: 'test-secret-key',
    sessionToken: 'test-session-token',
  }
}

const PROVIDERS = Object.values(LLM_PROVIDERS)

describe('provider registration contract', () => {
  it('covers every provider in the server factory registry', () => {
    for (const provider of PROVIDERS) {
      const model = createLLMProvider(kitchenSinkConfig(provider))
      assert.ok(model, `server factory returned no model for "${provider}"`)
    }
  })

  it('covers every provider in the agent factory registry', () => {
    for (const provider of PROVIDERS) {
      const model = createLanguageModel({
        conversationId: 'contract-test',
        ...kitchenSinkConfig(provider),
      })
      assert.ok(model, `agent factory returned no model for "${provider}"`)
    }
  })

  it('throws a clear error for an unregistered provider', () => {
    assert.throws(
      () => createLLMProvider(kitchenSinkConfig('made-up-provider')),
      /Unknown provider/,
    )
  })
})
