/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { createStatusRoute } from '../../../src/api/routes/status'
import { ServerActivity } from '../../../src/api/services/server-activity'
import { TurnRegistry } from '../../../src/lib/agents/turns/active-turn-registry'

function createActivity() {
  const registry = new TurnRegistry({
    retainAfterDoneMs: 1000,
    sweepIntervalMs: 60_000,
  })
  return {
    activity: new ServerActivity(registry),
    registry,
  }
}

describe('createStatusRoute', () => {
  it('returns status ok when no browser is provided', async () => {
    const route = createStatusRoute()
    const response = await route.request('/')

    assert.strictEqual(response.status, 200)
    const body = await response.json()
    assert.deepStrictEqual(body, { status: 'ok', can_update: true })
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
      can_update: true,
    })

    connected = true

    const secondResponse = await route.request('/')
    assert.deepStrictEqual(await secondResponse.json(), {
      status: 'ok',
      cdpConnected: true,
      can_update: true,
    })
  })

  it('reports can_update false while a chat stream is open', async () => {
    const { activity, registry } = createActivity()
    const route = createStatusRoute({ activity })

    activity.beginChatStream()
    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: false,
    })

    activity.endChatStream()
    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: true,
    })
    registry.stopSweeper()
  })

  it('reports can_update false while a registry turn is running', async () => {
    const { activity, registry } = createActivity()
    const route = createStatusRoute({ activity })
    const turn = registry.register('agent-1')

    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: false,
    })

    registry.pushEvent(turn.turnId, { type: 'done', stopReason: 'end_turn' })
    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: true,
    })
    registry.stopSweeper()
  })

  it('reports can_update false while an MCP browser tool is executing', async () => {
    const { activity, registry } = createActivity()
    const route = createStatusRoute({ activity })

    activity.beginMcpToolExecution()
    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: false,
    })

    activity.endMcpToolExecution()
    assert.deepStrictEqual(await (await route.request('/')).json(), {
      status: 'ok',
      can_update: true,
    })
    registry.stopSweeper()
  })
})
