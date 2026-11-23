import { afterEach, describe, expect, spyOn, test } from 'bun:test'
import type { TelemetryHealth } from '../../shared/api'
import { startTelemetry, type Telemetry, type TelemetryDeps } from '.'
import { fixtureMetrics, fixturePrometheus } from './fixtures'
import { pollMetrics } from './neo-metrics'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

function start(deps: Partial<TelemetryDeps> = {}): Telemetry {
  const telemetry = startTelemetry({ hostApi: false, now: Date.now, ...deps })
  cleanups.push(() => telemetry.stop())
  return telemetry
}

async function until<T>(read: () => T | null | undefined | false, ms = 2_000) {
  const deadline = Date.now() + ms
  for (;;) {
    const value = read()
    if (value) return value
    if (Date.now() > deadline) throw new Error('timed out')
    await Bun.sleep(5)
  }
}

function push(telemetry: Telemetry, step: number) {
  return telemetry.receiverRoutes['/otlp/v1/metrics']?.POST(
    new Request('http://127.0.0.1/otlp/v1/metrics', {
      method: 'POST',
      headers: { 'content-type': 'application/json', host: '127.0.0.1' },
      body: JSON.stringify(fixtureMetrics(step)),
    }),
  )
}

/** A host API whose `/v1/metrics` answers with this; records the paths asked. */
function host(answer: () => Response) {
  const asked: string[] = []
  return {
    asked,
    fetch: async (path: string) => {
      asked.push(path)
      return answer()
    },
  }
}

describe('the scrape door', () => {
  test('an image older than telemetry (404) stops the poller for good, silently', async () => {
    const errors = spyOn(console, 'error')
    const warnings = spyOn(console, 'warn')
    const logs = spyOn(console, 'log')
    const neo = host(() =>
      Response.json({ code: 'not_found' }, { status: 404 }),
    )
    const telemetry = start({
      hostApi: true,
      scrape: { fetch: neo.fetch, intervalMs: 10, retryMs: 10 },
    })
    await Bun.sleep(80)
    expect(neo.asked).toEqual(['/v1/metrics'])
    expect(telemetry.health()).toBeNull()
    expect(telemetry.metrics()).toBeNull()
    expect(telemetry.exportSnapshot()).toBeNull()
    for (const spy of [errors, warnings, logs]) {
      expect(spy).not.toHaveBeenCalled()
      spy.mockRestore()
    }
  })

  test('a host API that is not answering yet (502, 401) is tried again, then read', async () => {
    let status = 502
    const neo = host(() =>
      status === 200
        ? new Response(fixturePrometheus(0), {
            headers: { 'content-type': 'text/plain; version=0.0.4' },
          })
        : Response.json({ code: 'host_api_unreachable' }, { status }),
    )
    const telemetry = start({
      hostApi: true,
      scrape: { fetch: neo.fetch, intervalMs: 10, retryMs: 20 },
    })
    await until(() => neo.asked.length >= 2)
    status = 401
    await until(() => neo.asked.length >= 3)
    status = 200
    const health = await until(() => telemetry.health())
    expect(health).toMatchObject({
      source: 'scrape',
      runsInFlight: 0,
      tabsOpen: 3,
    })
    expect(telemetry.exportSnapshot()?.prometheus).toBe(fixturePrometheus(0))
  })

  test('the poller reads the body as text and hands it on', async () => {
    const texts: string[] = []
    const poller = pollMetrics({
      fetch: async () => new Response('neo_run_active 2\n'),
      onText: (text) => texts.push(text),
      now: Date.now,
      intervalMs: 5,
    })
    await until(() => texts.length >= 2)
    await poller.stop()
    const heard = texts.length
    await Bun.sleep(30)
    // Stopped: no scrape after it.
    expect(texts).toHaveLength(heard)
    expect(texts[0]).toBe('neo_run_active 2\n')
  })
})

describe('health', () => {
  test('push only (no host API, as with --upstream): the health bar and Health view read OpenTelemetry names', async () => {
    const telemetry = start({ emitEveryMs: 10 })
    await push(telemetry, 1)
    const health = await until(() => telemetry.health())
    expect(health).toMatchObject({
      source: 'push',
      runsInFlight: 1,
      tabsOpen: 3,
    })
    const view = telemetry.metrics()
    expect(view?.source).toBe('push')
    expect(view?.runs).toMatchObject({
      total: 5,
      inFlight: 1,
      byStatus: { ok: 4, error: 1 },
      duration: { count: 5 },
    })
    expect(
      view?.operations.find((row) => row.op === 'locator.click'),
    ).toMatchObject({ total: 9, errors: 5, duration: { count: 9 } })
    expect(view?.errors).toEqual([
      { owner: 'script', code: 'no_match', count: 5 },
      { owner: 'website', code: 'navigation_failed', count: 1 },
    ])
    expect(view?.tabs).toMatchObject({
      openByOrigin: { agent: 2, found: 1 },
      openedByOrigin: { agent: 4, found: 1 },
      closedByReason: { script: 2, janitor: 0 },
      lifetime: { count: 2 },
    })
    expect(view?.families.map((family) => family.name)).toContain(
      'neo_tab_duration_seconds',
    )
    // The status arrives as the registry's int and the spend as a double, under their OTLP
    // names and keys, and read as the scrape's.
    expect(view?.navigations).toEqual({
      total: 6,
      byStatus: { '200': 4, '404': 1, none: 1 },
      errors: { name_not_resolved: 1 },
    })
    expect(view?.captchas).toMatchObject({
      total: 3,
      solves: [
        { family: 'turnstile', verdict: 'solved', count: 2 },
        { family: 'perimeterx', verdict: 'unsupported', count: 1 },
      ],
      spendUsd: { capsolver: 0.0012 },
    })
  })

  test('the scrape wins while live; when it goes quiet the push takes over, and with both quiet the numbers go', async () => {
    let scraping = true
    const neo = host(() =>
      scraping
        ? new Response(fixturePrometheus(0))
        : Response.json({ code: 'host_api_unreachable' }, { status: 502 }),
    )
    const telemetry = start({
      hostApi: true,
      scrape: { fetch: neo.fetch, intervalMs: 10, retryMs: 10 },
      liveWithinMs: 150,
      emitEveryMs: 10,
    })
    await until(() => telemetry.health()?.source === 'scrape')
    await push(telemetry, 4)
    // Both live: still the scrape (its numbers: 3 ok runs; the push says 7).
    expect(telemetry.metrics()).toMatchObject({
      source: 'scrape',
      runs: { total: 4 },
    })
    // The scrape stops answering while Neo keeps pushing.
    scraping = false
    let pushing = true
    const pushes = (async () => {
      while (pushing) {
        await push(telemetry, 4)
        await Bun.sleep(20)
      }
    })()
    await until(() => telemetry.health()?.source === 'push')
    pushing = false
    await pushes
    expect(telemetry.metrics()).toMatchObject({
      source: 'push',
      runs: { total: 8 },
    })
    const quiet = await until(() => {
      const health = telemetry.health()
      return health?.source === null ? health : null
    }, 1_000)
    expect(quiet).toMatchObject({
      observedAt: null,
      runsInFlight: null,
      tabsOpen: null,
      sinceClear: null,
    })
    expect(telemetry.metrics()).toBeNull()
    // An export still has the last numbers heard: the push's, so not the scrape's text beside them.
    expect(telemetry.exportSnapshot()?.metrics.source).toBe('push')
    expect(telemetry.exportSnapshot()?.prometheus).toBeNull()
  })

  test('changes reach subscribers at most once per interval, with a rising rev; sinceClear counts from the Clear', async () => {
    const telemetry = start({ emitEveryMs: 60 })
    const heard: TelemetryHealth[] = []
    telemetry.subscribe((health) => heard.push(health))
    await push(telemetry, 0)
    await push(telemetry, 1)
    await push(telemetry, 2)
    await until(() => heard.length >= 1)
    await Bun.sleep(100)
    // Three samples, coalesced: the first change, then the rest together.
    expect(heard.length).toBeLessThanOrEqual(2)
    const last = heard.at(-1)
    expect(last).toMatchObject({
      source: 'push',
      // From the first sample (the baseline): two more runs, each with its operations.
      sinceClear: { runs: 2, runErrors: 0, ops: 16, opErrors: 2, crashes: 0 },
    })
    expect(heard.map((health) => health.rev)).toEqual(
      [...heard.map((health) => health.rev)].sort((a, b) => a - b),
    )
    telemetry.rebase()
    expect(telemetry.health()?.sinceClear).toMatchObject({ runs: 0, ops: 0 })
    expect(telemetry.health()?.rev).toBeGreaterThan(last?.rev ?? 0)
    await push(telemetry, 3)
    expect(telemetry.health()?.sinceClear).toMatchObject({
      runs: 1,
      opErrors: 1,
    })
  })
})
