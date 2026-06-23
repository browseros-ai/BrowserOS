/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  createNudgeMcpRoute,
  NUDGE_AGENT_ID_HEADER,
  NUDGE_SESSION_ID_HEADER,
} from '../../../src/api/routes/nudge-mcp'
import type { AgentStreamEvent } from '../../../src/lib/agents/types'

interface StubTurnInfo {
  turnId: string
}

interface StubRegistry {
  getActiveFor(agentId: string, sessionId: string): StubTurnInfo | undefined
  pushEvent(turnId: string, event: AgentStreamEvent): unknown
  events: Array<{ turnId: string; event: AgentStreamEvent }>
}

function makeStubRegistry(
  active?: {
    agentId: string
    sessionId: string
    turnId: string
  },
  options: { pushReturnsNull?: boolean } = {},
): StubRegistry {
  const events: Array<{ turnId: string; event: AgentStreamEvent }> = []
  return {
    events,
    getActiveFor(agentId, sessionId) {
      if (
        active &&
        active.agentId === agentId &&
        active.sessionId === sessionId
      ) {
        return { turnId: active.turnId }
      }
      return undefined
    },
    pushEvent(turnId, event) {
      events.push({ turnId, event })
      // Real TurnRegistry returns a TurnFrame on success and null when
      // the turn flipped to a terminal status mid-call. Tests use this
      // sentinel to exercise the deadlock-guard branch.
      return options.pushReturnsNull ? null : { id: turnId, seq: 0 }
    },
  }
}

function jsonRpcCallTool(args: { appName: string; reason: string }): string {
  return JSON.stringify({
    jsonrpc: '2.0',
    id: 1,
    method: 'tools/call',
    params: {
      name: 'suggest_app_connection',
      arguments: args,
    },
  })
}

async function callTool(
  registry: StubRegistry,
  headers: Record<string, string> = {},
  args: { appName: string; reason: string } = {
    appName: 'Linear',
    reason: 'to read issues',
  },
): Promise<{ status: number; body: string }> {
  // Cast is safe: the route only calls the three TurnRegistry methods
  // covered by the stub above. Avoids importing the heavyweight real
  // TurnRegistry class into the test.
  const route = createNudgeMcpRoute({
    // biome-ignore lint/suspicious/noExplicitAny: stub is intentional
    turnRegistry: registry as any,
  })
  const response = await route.request('/', {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      ...headers,
    },
    body: jsonRpcCallTool(args),
  })
  return { status: response.status, body: await response.text() }
}

describe('createNudgeMcpRoute', () => {
  it('pushes app_connection_request when the header identifies an active turn', async () => {
    const registry = makeStubRegistry({
      agentId: 'agent-A',
      sessionId: 'main',
      turnId: 'turn-1',
    })

    const { status, body } = await callTool(registry, {
      [NUDGE_AGENT_ID_HEADER]: 'agent-A',
      [NUDGE_SESSION_ID_HEADER]: 'main',
    })

    expect(status).toBe(200)
    expect(registry.events).toHaveLength(1)
    expect(registry.events[0].turnId).toBe('turn-1')
    expect(registry.events[0].event).toMatchObject({
      type: 'app_connection_request',
      appName: 'Linear',
      reason: 'to read issues',
    })
    expect(body).toContain('Connect card for Linear shown to the user')
  })

  it('falls back to MAIN session id when no session header is present', async () => {
    const registry = makeStubRegistry({
      agentId: 'agent-A',
      sessionId: 'main',
      turnId: 'turn-main',
    })

    await callTool(registry, { [NUDGE_AGENT_ID_HEADER]: 'agent-A' })

    expect(registry.events).toHaveLength(1)
    expect(registry.events[0].turnId).toBe('turn-main')
  })

  it('returns an isError response when the agent id header is missing', async () => {
    const registry = makeStubRegistry({
      agentId: 'agent-A',
      sessionId: 'main',
      turnId: 'turn-1',
    })

    const { body } = await callTool(registry, {})

    expect(registry.events).toHaveLength(0)
    expect(body).toContain('X-BrowserOS-Agent-Id')
    expect(body).toContain('isError')
  })

  it('returns an isError response when no active turn is registered', async () => {
    const registry = makeStubRegistry()

    const { body } = await callTool(registry, {
      [NUDGE_AGENT_ID_HEADER]: 'agent-A',
      [NUDGE_SESSION_ID_HEADER]: 'main',
    })

    expect(registry.events).toHaveLength(0)
    expect(body).toContain('No in-flight turn')
    expect(body).toContain('isError')
  })

  it('returns an isError response when the turn ends mid-call (pushEvent returns null)', async () => {
    const registry = makeStubRegistry(
      { agentId: 'agent-A', sessionId: 'main', turnId: 'turn-1' },
      { pushReturnsNull: true },
    )

    const { body } = await callTool(registry, {
      [NUDGE_AGENT_ID_HEADER]: 'agent-A',
      [NUDGE_SESSION_ID_HEADER]: 'main',
    })

    expect(registry.events).toHaveLength(1)
    expect(body).toContain('turn ended before the connect card')
    expect(body).toContain('isError')
  })
})
