/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { logger } from '../../logger'

const CODEX_API_ENDPOINT = 'https://chatgpt.com/backend-api/codex/responses'

export function createCodexFetch(accountId?: string) {
  return (input: RequestInfo | URL, init?: RequestInit) => {
    let inputUrl: string
    if (typeof input === 'string') {
      inputUrl = input
    } else if (input instanceof URL) {
      inputUrl = input.toString()
    } else if (input instanceof Request) {
      inputUrl = input.url
    } else {
      inputUrl = String(input)
    }

    const parsed = new URL(inputUrl)
    const shouldRewrite =
      parsed.pathname.includes('/v1/responses') ||
      parsed.pathname.includes('/chat/completions')
    const url = shouldRewrite ? new URL(CODEX_API_ENDPOINT) : parsed

    const headers = new Headers(init?.headers as HeadersInit)
    if (accountId) {
      headers.set('ChatGPT-Account-Id', accountId)
    }
    headers.set('originator', 'browseros')
    headers.set('OpenAI-Beta', 'responses=experimental')

    let body = init?.body
    if (shouldRewrite && body && typeof body === 'string') {
      try {
        const json = JSON.parse(body)
        json.stream = true
        json.store = false
        delete json.previous_response_id
        delete json.temperature
        delete json.max_tokens
        delete json.max_output_tokens
        delete json.top_p
        if (!json.instructions) {
          json.instructions = 'You are a helpful assistant.'
        }
        // Strip item IDs — Codex doesn't persist items with store=false.
        // The SDK should already inline content (via providerOptions store=false),
        // but this is a safety net matching OpenCode's approach.
        if (Array.isArray(json.input)) {
          for (const item of json.input) {
            if ('id' in item) {
              delete item.id
            }
          }
        }
        body = JSON.stringify(json)
      } catch (err) {
        logger.warn(
          'Failed to inject Codex-required fields into request body',
          {
            error: err instanceof Error ? err.message : String(err),
          },
        )
      }
    }

    return fetch(url, { ...init, headers, body })
  }
}
