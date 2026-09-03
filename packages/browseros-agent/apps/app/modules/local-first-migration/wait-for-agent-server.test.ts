import { describe, expect, it } from 'bun:test'
import { waitForAgentServer } from './wait-for-agent-server'

function harness(healthyAfter: number) {
  let calls = 0
  let clock = 0
  return {
    calls: () => calls,
    elapsed: () => clock,
    opts: {
      isHealthy: async () => {
        calls += 1
        return calls > healthyAfter
      },
      now: () => clock,
      sleep: async (ms: number) => {
        clock += ms
      },
      timeoutMs: 60_000,
      intervalMs: 1_000,
    },
  }
}

describe('waitForAgentServer', () => {
  it('returns immediately when the server is already up', async () => {
    const h = harness(0)

    expect(await waitForAgentServer(h.opts)).toBe(true)
    expect(h.calls()).toBe(1)
    expect(h.elapsed()).toBe(0)
  })

  // The case this exists for: the background starts at the same moment as the
  // server, so the first few probes find nothing listening.
  it('waits out a server that is still starting', async () => {
    const h = harness(6)

    expect(await waitForAgentServer(h.opts)).toBe(true)
    expect(h.calls()).toBe(7)
    expect(h.elapsed()).toBe(6_000)
  })

  // Six seconds is roughly what was observed between the browser launching and
  // the server answering, and the previous behaviour gave up inside two.
  it('outlasts the gap that made the import fail', async () => {
    const h = harness(6)
    await waitForAgentServer(h.opts)

    expect(h.elapsed()).toBeGreaterThan(1_500)
  })

  // Giving up rather than throwing is what lets the caller leave the markers
  // unset, so the next start tries again.
  it('reports failure rather than throwing when the server never answers', async () => {
    const h = harness(Number.POSITIVE_INFINITY)

    expect(await waitForAgentServer(h.opts)).toBe(false)
  })

  it('stops probing once the deadline passes', async () => {
    const h = harness(Number.POSITIVE_INFINITY)
    await waitForAgentServer(h.opts)

    expect(h.elapsed()).toBeLessThanOrEqual(60_000)
    expect(h.calls()).toBeLessThanOrEqual(62)
  })

  // Connection refused is the expected state early on, so a probe that throws
  // has to read as not reachable rather than end the wait.
  it('treats a throwing probe as not reachable and keeps waiting', async () => {
    let calls = 0
    let clock = 0

    const result = await waitForAgentServer({
      isHealthy: async () => {
        calls += 1
        if (calls < 3) throw new Error('connection refused')
        return true
      },
      now: () => clock,
      sleep: async (ms) => {
        clock += ms
      },
      timeoutMs: 60_000,
      intervalMs: 1_000,
    })

    expect(result).toBe(true)
    expect(calls).toBe(3)
    expect(clock).toBe(2_000)
  })
})
