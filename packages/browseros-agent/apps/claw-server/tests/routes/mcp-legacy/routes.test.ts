/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Locks the env-gated 404 behaviour on the legacy `/mcp/:slug` route.
 * With the flag unset, the route returns a structured 404 pointing at
 * the v2 endpoint. With the flag set, dispatch reaches the per-slug
 * manager (covered by the existing integration suite).
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import app from '../../../src/server'

describe('/mcp/:slug legacy gate', () => {
  let prev: string | undefined

  beforeEach(() => {
    // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
    prev = process.env.COCKPIT_LEGACY_PER_AGENT_MCP
    // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
    delete process.env.COCKPIT_LEGACY_PER_AGENT_MCP
  })
  afterEach(() => {
    if (prev === undefined) {
      // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
      delete process.env.COCKPIT_LEGACY_PER_AGENT_MCP
    } else {
      // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
      process.env.COCKPIT_LEGACY_PER_AGENT_MCP = prev
    }
  })

  test('returns 404 with a structured hint when the flag is unset', async () => {
    const res = await app.fetch(
      new Request('http://localhost/mcp/anything', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'initialize',
          params: {
            protocolVersion: '2024-11-05',
            capabilities: {},
            clientInfo: { name: 'test', version: '0' },
          },
        }),
      }),
    )
    expect(res.status).toBe(404)
    const body = (await res.json()) as { error?: string; hint?: string }
    expect(body.error).toBe('legacy per-agent MCP endpoint is disabled')
    expect(body.hint).toContain('/cockpit/mcp')
    expect(body.hint).toContain('COCKPIT_LEGACY_PER_AGENT_MCP')
  })

  test('also 404s for GET and DELETE methods', async () => {
    for (const method of ['GET', 'DELETE'] as const) {
      const res = await app.fetch(
        new Request('http://localhost/mcp/anything', { method }),
      )
      expect(res.status).toBe(404)
    }
  })

  test('treats COCKPIT_LEGACY_PER_AGENT_MCP=true the same as =1', async () => {
    // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
    process.env.COCKPIT_LEGACY_PER_AGENT_MCP = 'true'
    const res = await app.fetch(
      new Request('http://localhost/mcp/anything', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'initialize',
          params: {
            protocolVersion: '2024-11-05',
            capabilities: {},
            clientInfo: { name: 'test', version: '0' },
          },
        }),
      }),
    )
    // The gate let this through to the manager. The manager may
    // still 404 for an unknown slug, but the disabled-gate hint is
    // gone from the body.
    const body = (await res.json()) as { hint?: string }
    expect(body.hint ?? '').not.toContain('COCKPIT_LEGACY_PER_AGENT_MCP')
  })
})
