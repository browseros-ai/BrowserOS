/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'

interface CaptureCall {
  distinctId: string
  event: string
  properties: Record<string, unknown>
}

const captureCalls: CaptureCall[] = []

class FakePostHog {
  // biome-ignore lint/complexity/noUselessConstructor: matches the real PostHog(apiKey, opts) signature so `new PostHog(...)` from the module under test type-checks against the mock
  constructor(_apiKey: string, _opts: { host: string }) {}
  capture(call: CaptureCall): void {
    captureCalls.push(call)
  }
  async shutdown(): Promise<void> {
    return Promise.resolve()
  }
}

mock.module('posthog-node', () => ({ PostHog: FakePostHog }))
mock.module('../../src/env', () => ({
  INLINED_ENV: { POSTHOG_API_KEY: 'test-key' },
}))

// Loaded after the module mocks so the metrics module picks them up.
const metricsModule = await import('../../src/lib/metrics')
const { metrics, __internal__ } = metricsModule
const {
  ROLLUP_INTERVAL_MS,
  MAX_TOOL_NAME_KEYS,
  sanitizeToolName,
  RollupBuffer,
} = __internal__

beforeEach(async () => {
  // Drain whatever's left in the singleton RollupBuffer + PostHog
  // client from a prior test so each test is hermetic. shutdown nulls
  // the client; the initialize calls below re-create it.
  await metrics.shutdown()
  captureCalls.length = 0
  metrics.initialize({ client_id: '__reset__' })
  metrics.initialize({ client_id: undefined, install_id: undefined })
})

describe('sanitizeToolName', () => {
  it('replaces dots with underscores so HogQL prop access is safe', () => {
    expect(sanitizeToolName('foo.bar.baz')).toBe('foo_bar_baz')
  })

  it('caps length so a pathological name cannot blow up the payload', () => {
    const long = `${'x'.repeat(200)}`
    expect(sanitizeToolName(long).length).toBe(80)
  })
})

describe('RollupBuffer', () => {
  it('returns null on drain when nothing was recorded', () => {
    const buf = new RollupBuffer()
    expect(buf.drain(new Date().toISOString())).toBeNull()
  })

  it('accumulates mcp_requests and tool_executions across calls', () => {
    const buf = new RollupBuffer()
    buf.recordMcpRequest()
    buf.recordMcpRequest()
    buf.recordToolExecuted({
      tool_name: 'navigate_page',
      source: 'chat',
      success: true,
    })
    buf.recordToolExecuted({
      tool_name: 'navigate_page',
      source: 'chat',
      success: false,
    })
    buf.recordToolExecuted({
      tool_name: 'fill',
      source: 'mcp',
      success: true,
    })

    const bucket = buf.drain(new Date().toISOString())
    expect(bucket).not.toBeNull()
    if (!bucket) return
    expect(bucket.mcp_requests).toBe(2)
    expect(bucket.tool_executions.total).toBe(3)
    expect(bucket.tool_executions.failed).toBe(1)
    expect(bucket.tool_executions.by_tool).toEqual({
      navigate_page: 2,
      fill: 1,
    })
    expect(bucket.tool_executions.by_source).toEqual({ chat: 2, mcp: 1 })
  })

  it('resets internal state after drain so the next interval is clean', () => {
    const buf = new RollupBuffer()
    buf.recordMcpRequest()
    buf.drain(new Date().toISOString())
    expect(buf.drain(new Date().toISOString())).toBeNull()
  })

  it('rolls overflow tool names into __other__ once the cap is reached', () => {
    const buf = new RollupBuffer()
    for (let i = 0; i < MAX_TOOL_NAME_KEYS; i++) {
      buf.recordToolExecuted({ tool_name: `tool_${i}`, source: 'chat' })
    }
    // The (cap + 1)th distinct name should land in __other__.
    buf.recordToolExecuted({ tool_name: 'overflow_one', source: 'chat' })
    buf.recordToolExecuted({ tool_name: 'overflow_two', source: 'chat' })
    // A name already in the bucket continues to increment, even after the cap.
    buf.recordToolExecuted({ tool_name: 'tool_0', source: 'chat' })

    const bucket = buf.drain(new Date().toISOString())
    if (!bucket) throw new Error('expected bucket')
    expect(Object.keys(bucket.tool_executions.by_tool).length).toBe(
      MAX_TOOL_NAME_KEYS + 1, // original cap + __other__
    )
    expect(bucket.tool_executions.by_tool.__other__).toBe(2)
    expect(bucket.tool_executions.by_tool.tool_0).toBe(2)
  })
})

describe('MetricsService — log dispatch', () => {
  it('aggregates mcp.request without immediately capturing', () => {
    metrics.initialize({ client_id: 'client-a' })
    metrics.log('mcp.request', { scopeId: 'ephemeral' })
    metrics.log('mcp.request')
    metrics.log('mcp.request')
    expect(captureCalls).toHaveLength(0)
  })

  it('aggregates tool_executed without immediately capturing', () => {
    metrics.initialize({ client_id: 'client-a' })
    metrics.log('tool_executed', {
      tool_name: 'navigate_page',
      source: 'chat',
      success: true,
    })
    metrics.log('tool_executed', {
      tool_name: 'fill',
      source: 'mcp',
      success: false,
    })
    expect(captureCalls).toHaveLength(0)
  })

  it('captures non-aggregated events immediately', () => {
    metrics.initialize({ client_id: 'client-a' })
    metrics.log('chat.request', { mode: 'agent' })
    expect(captureCalls).toHaveLength(1)
    expect(captureCalls[0]?.event).toBe('browseros.server.chat.request')
    expect(captureCalls[0]?.distinctId).toBe('client-a')
  })

  it('skips emit entirely when no identity is configured', () => {
    // No client_id, no install_id → captureNow should short-circuit.
    metrics.initialize({})
    metrics.log('chat.request', { mode: 'agent' })
    expect(captureCalls).toHaveLength(0)
  })

  it('still aggregates noisy events even without identity, but flush emits nothing', async () => {
    metrics.initialize({})
    metrics.log('mcp.request')
    metrics.log('tool_executed', { tool_name: 'fill', source: 'chat' })
    // No way to reach flush from outside, but shutdown drains + skip-on-no-identity
    // means the emit on the underlying captureNow is also skipped.
    await metrics.shutdown()
    expect(captureCalls).toHaveLength(0)
  })
})

describe('MetricsService — flush + shutdown', () => {
  it('emits a single usage_rollup on shutdown drain', async () => {
    metrics.initialize({
      client_id: 'client-a',
      install_id: 'install-a',
      browseros_version: '0.46.0.0',
      server_version: '0.0.99',
    })

    metrics.log('mcp.request', { scopeId: 'ephemeral' })
    metrics.log('mcp.request')
    metrics.log('tool_executed', {
      tool_name: 'navigate_page',
      source: 'chat',
      success: true,
    })
    metrics.log('tool_executed', {
      tool_name: 'navigate_page',
      source: 'chat',
      success: false,
    })
    metrics.log('tool_executed', {
      tool_name: 'fill',
      source: 'mcp',
      success: true,
    })

    await metrics.shutdown()

    expect(captureCalls).toHaveLength(1)
    const call = captureCalls[0]
    if (!call) throw new Error('expected one capture call')
    expect(call.event).toBe('browseros.server.usage_rollup')
    expect(call.distinctId).toBe('client-a')
    expect(call.properties.interval_seconds).toBe(ROLLUP_INTERVAL_MS / 1000)
    expect(call.properties.mcp_requests_count).toBe(2)
    expect(call.properties.tool_executions_count).toBe(3)
    expect(call.properties.tool_executions_failed).toBe(1)
    expect(call.properties.tool_executions_by_source).toEqual({
      chat: 2,
      mcp: 1,
    })
    expect(call.properties.tool_executions_by_tool).toEqual({
      navigate_page: 2,
      fill: 1,
    })
    expect(call.properties.browseros_version).toBe('0.46.0.0')
    expect(call.properties.server_version).toBe('0.0.99')
  })

  it('emits nothing on shutdown when no activity was recorded', async () => {
    metrics.initialize({ client_id: 'client-a' })
    await metrics.shutdown()
    expect(captureCalls).toHaveLength(0)
  })

  it('does not retain the dotted-name shape on aggregated tool names', async () => {
    metrics.initialize({ client_id: 'client-a' })
    metrics.log('tool_executed', { tool_name: 'foo.bar', source: 'chat' })
    await metrics.shutdown()
    const call = captureCalls[0]
    if (!call) throw new Error('expected one capture call')
    expect(
      (call.properties.tool_executions_by_tool as Record<string, number>)
        .foo_bar,
    ).toBe(1)
  })
})
