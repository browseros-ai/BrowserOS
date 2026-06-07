/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Contract test: the agent UI provider surface must stay in sync with the
 * shared LLM_PROVIDERS source of truth. Catches a provider added to the schema
 * but missing a base URL, a dropdown option, or a setup template.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import {
  DEFAULT_BASE_URLS,
  providerTemplates,
  providerTypeOptions,
} from './providerTemplates'

const PROVIDERS = Object.values(LLM_PROVIDERS)

// `browseros` is the hosted provider — configured via onboarding, not via the
// manual "Add Provider" template list.
const PROVIDERS_WITHOUT_TEMPLATE = new Set(['browseros'])

describe('agent provider UI contract', () => {
  it('every LLM provider has a default base URL', () => {
    for (const provider of PROVIDERS) {
      assert.ok(
        provider in DEFAULT_BASE_URLS,
        `DEFAULT_BASE_URLS is missing "${provider}"`,
      )
    }
  })

  it('every LLM provider appears in the dropdown options', () => {
    const optionValues = new Set(providerTypeOptions.map((o) => o.value))
    for (const provider of PROVIDERS) {
      assert.ok(
        optionValues.has(provider),
        `providerTypeOptions is missing "${provider}"`,
      )
    }
  })

  it('every LLM provider (except hosted) has a setup template', () => {
    const templateIds = new Set(providerTemplates.map((t) => t.id))
    for (const provider of PROVIDERS) {
      if (PROVIDERS_WITHOUT_TEMPLATE.has(provider)) continue
      assert.ok(
        templateIds.has(provider),
        `providerTemplates is missing "${provider}"`,
      )
    }
  })
})
