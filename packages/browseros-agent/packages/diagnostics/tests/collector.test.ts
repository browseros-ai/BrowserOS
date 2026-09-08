import { describe, expect, it } from 'bun:test'
import { createDiagnosticsCollector } from '../src/collector'
import { formatDiagnostics, parseDiagnostics } from '../src/contract'
import fixture from './fixture.json'

const snapshot = parseDiagnostics(fixture)
if (!snapshot) throw new Error('Invalid fixture')

describe('diagnostics collection ownership', () => {
  it('coalesces concurrent refreshes and persists a serializable snapshot', async () => {
    let reads = 0
    let saved: unknown
    const collector = createDiagnosticsCollector({
      read: async () => {
        reads++
        await Promise.resolve()
        return snapshot
      },
      load: async () => null,
      save: async (value) => {
        saved = value
      },
    })
    const [first, second] = await Promise.all([
      collector.get(0),
      collector.get(0),
    ])
    expect(reads).toBe(1)
    expect(first).toEqual(second)
    expect(saved).toEqual(snapshot)
  })

  it('honors Refresh when it overlaps a cache-accepting Reporter request', async () => {
    let reads = 0
    const current = { ...snapshot, collectedAt: new Date().toISOString() }
    const collector = createDiagnosticsCollector({
      read: async () => {
        reads++
        return current
      },
      load: async () => current,
      save: async () => {},
    })
    const reporter = collector.get(60_000)
    const refresh = collector.get(0)
    await Promise.all([reporter, refresh])
    expect(reads).toBe(1)
    expect((await refresh).cached).toBe(false)
  })

  it('preserves the timestamp and labels a fallback after refresh failure', async () => {
    const collector = createDiagnosticsCollector({
      read: async () => {
        throw new Error('unavailable')
      },
      load: async () => snapshot,
      save: async () => {},
    })
    expect(await collector.get(0)).toEqual({ snapshot, cached: true })
  })

  it('returns current data even when storage fails', async () => {
    const collector = createDiagnosticsCollector({
      read: async () => snapshot,
      load: async () => {
        throw new Error('storage failed')
      },
      save: async () => {
        throw new Error('storage full')
      },
    })
    expect(await collector.get(0)).toEqual({ snapshot, cached: false })
  })

  it('formats exactly the snapshot and rejects invalid memory without leaking fields', () => {
    expect(formatDiagnostics(snapshot)).toContain(fixture.collectedAt)
    expect(formatDiagnostics(snapshot)).toContain('36.0 GiB')
    expect(
      parseDiagnostics({ ...fixture, secret: 'hidden' }),
    ).not.toHaveProperty('secret')
    expect(
      parseDiagnostics({
        ...fixture,
        memory: { totalBytes: 1, availableBytes: 2 },
      }),
    ).toBeNull()
  })
})
