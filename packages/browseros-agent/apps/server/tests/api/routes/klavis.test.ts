/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, it } from 'bun:test'
import assert from 'node:assert'
import { createKlavisRoutes } from '../../../src/api/routes/klavis'
import { klavisStrataCache } from '../../../src/api/services/klavis/strata-cache'

const originalFetch = globalThis.fetch

beforeEach(() => {
  klavisStrataCache.clear()
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('createKlavisRoutes', () => {
  it('normalizes string integrations into authenticated entries', async () => {
    globalThis.fetch = (async () =>
      Response.json({
        integrations: ['Google Docs', 'Slack'],
      })) as typeof fetch

    const route = createKlavisRoutes({ browserosId: 'user-123' })
    const response = await route.request('/user-integrations')
    const body = await response.json()

    assert.strictEqual(response.status, 200)
    assert.deepStrictEqual(body, {
      integrations: [
        { name: 'Google Docs', is_authenticated: true },
        { name: 'Slack', is_authenticated: true },
      ],
      count: 2,
    })
  })

  it('supports object integrations with mixed auth flag formats', async () => {
    globalThis.fetch = (async () =>
      Response.json({
        integrations: [
          { name: 'Google Docs', isAuthenticated: false },
          { name: 'Slack', is_authenticated: true },
        ],
      })) as typeof fetch

    const route = createKlavisRoutes({ browserosId: 'user-123' })
    const response = await route.request('/user-integrations')
    const body = await response.json()

    assert.strictEqual(response.status, 200)
    assert.deepStrictEqual(body, {
      integrations: [
        { name: 'Google Docs', is_authenticated: false },
        { name: 'Slack', is_authenticated: true },
      ],
      count: 2,
    })
  })

  it('resolves auth URLs with normalized server name keys', async () => {
    globalThis.fetch = (async () =>
      Response.json({
        strataServerUrl: 'https://strata.example.com',
        strataId: 'strata-123',
        addedServers: ['Google Docs'],
        oauthUrls: { google_docs: 'https://oauth.example.com/google-docs' },
        apiKeyUrls: {
          google_docs: 'https://auth.example.com/setup?instance_id=abc123',
        },
      })) as typeof fetch

    const route = createKlavisRoutes({ browserosId: 'user-123' })
    const response = await route.request('/servers/add', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ serverName: 'Google Docs' }),
    })
    const body = await response.json()

    assert.strictEqual(response.status, 200)
    assert.deepStrictEqual(body, {
      success: true,
      serverName: 'Google Docs',
      strataId: 'strata-123',
      addedServers: ['Google Docs'],
      oauthUrl: 'https://oauth.example.com/google-docs',
      apiKeyUrl: 'https://auth.example.com/setup?instance_id=abc123',
    })
  })
})
