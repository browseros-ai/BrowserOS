/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it } from 'bun:test'
import { createAcpxProbeRoutes } from '../../../src/api/routes/acpx-probe'
import type {
  ServerAcpxProbeInput,
  ServerAcpxProbeResult,
} from '../../../src/api/services/acpx-probe/probeAgent'

// Injection beats mock.module here: a sibling test (`probeAgent.test.ts`)
// imports the real probeAcpAgent and reads `acp-probe` directly, and bun's
// mock.module leaks across files in the same test process. Threading the
// stub through createAcpxProbeRoutes keeps both tests isolated.
let nextProbeResult: ServerAcpxProbeResult | null = null
let nextProbeError: Error | null = null
let lastInput: ServerAcpxProbeInput | null = null

const probeStub = async (
  input: ServerAcpxProbeInput,
): Promise<ServerAcpxProbeResult> => {
  lastInput = input
  if (nextProbeError) throw nextProbeError
  if (!nextProbeResult) throw new Error('probe stub: no nextProbeResult set')
  return nextProbeResult
}

beforeEach(() => {
  nextProbeResult = null
  nextProbeError = null
  lastInput = null
})

async function call(body: unknown) {
  const app = createAcpxProbeRoutes({ probe: probeStub })
  return app.request('/', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
}

describe('POST /acpx/probe', () => {
  it('returns 200 with the normalised probe result on success', async () => {
    nextProbeResult = {
      models: [{ id: 'sonnet' }],
      reasoning: { values: ['low', 'medium'], defaultValue: 'medium' },
      supportsConfigOption: true,
      agentInfo: { name: 'claude' },
      protocolVersion: 1,
    }
    const res = await call({ agentId: 'claude' })
    expect(res.status).toBe(200)
    const payload = (await res.json()) as { models: { id: string }[] }
    expect(payload.models[0]?.id).toBe('sonnet')
    expect(lastInput).toEqual({ agentId: 'claude' })
  })

  it('returns 200 with a populated error field when the probe reports a probe-level failure', async () => {
    nextProbeResult = {
      models: [],
      reasoning: null,
      supportsConfigOption: false,
      agentInfo: null,
      protocolVersion: 0,
      error: { code: 'spawn_failed', message: 'binary not found' },
    }
    const res = await call({ agentId: 'codex' })
    expect(res.status).toBe(200)
    const payload = (await res.json()) as { error?: { code: string } }
    expect(payload.error?.code).toBe('spawn_failed')
  })

  it('returns 500 when the wrapper throws (unrecoverable)', async () => {
    nextProbeError = new Error('boom')
    const res = await call({ agentId: 'claude' })
    expect(res.status).toBe(500)
    const payload = (await res.json()) as {
      error?: { code: string; message: string }
    }
    expect(payload.error?.code).toBe('wrapper_error')
    expect(payload.error?.message).toContain('boom')
  })

  it('rejects an empty body where neither agentId nor command is set', async () => {
    const res = await call({})
    expect(res.status).toBe(400)
  })

  it('accepts a command-only request for acp-custom', async () => {
    nextProbeResult = {
      models: [{ id: 'default' }],
      reasoning: null,
      supportsConfigOption: false,
      agentInfo: null,
      protocolVersion: 1,
    }
    const res = await call({ command: 'my-bin acp', cwd: '/tmp/x' })
    expect(res.status).toBe(200)
    expect(lastInput).toEqual({ command: 'my-bin acp', cwd: '/tmp/x' })
  })

  it('rejects an out-of-range timeout', async () => {
    const res = await call({ agentId: 'claude', timeoutMs: 100 })
    expect(res.status).toBe(400)
  })
})
