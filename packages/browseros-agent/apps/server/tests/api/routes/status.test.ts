/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { createStatusRoute } from '../../../src/api/routes/status'

describe('createStatusRoute', () => {
  it('returns status ok when no browser is provided', async () => {
    const route = createStatusRoute()
    const response = await route.request('/')

    assert.strictEqual(response.status, 200)
    const body = await response.json()
    assert.deepStrictEqual(body, { status: 'ok' })
  })

  it('reads CDP connectivity on each request', async () => {
    let connected = false
    const route = createStatusRoute({
      browser: {
        isCdpConnected: () => connected,
      } as never,
    })

    const firstResponse = await route.request('/')
    assert.deepStrictEqual(await firstResponse.json(), {
      status: 'ok',
      cdpConnected: false,
    })

    connected = true

    const secondResponse = await route.request('/')
    assert.deepStrictEqual(await secondResponse.json(), {
      status: 'ok',
      cdpConnected: true,
    })
  })
})
