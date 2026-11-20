/**
 * Neo's OpenTelemetry in the viewer, behind one function: `startTelemetry`. The only file of this
 * folder the rest of the server imports.
 *
 *   Neo ──OTLP/HTTP JSON──▶ /otlp/v1/{traces,metrics} ──▶ span store / push slot ─┐
 *   Neo ◀──GET /v1/metrics (host API, every 2 s)── poller ──▶ scrape slot ─────────┤
 *                                                                                 ▼
 *        records (spans → NeoRunRecord)   runTrace (waterfall)   health / metrics (Health view)
 *
 * Hidden here: OTLP JSON and Prometheus text parsing, the receiver's guard, the poller and its
 * silence on older images, the bounded span store, the per-door metric slots and counter resets,
 * and the span → record fold. What leaves is the page's domain types (`shared/api.ts`).
 */

import type { MetricsView, RunTrace, TelemetryHealth } from '../../shared/api'
import { hostFetch } from '../hostApi'
import type { RunRecordSource } from '../neo-runs'
import { buildTrace } from './fold-spans'
import {
  type Door,
  type HealthPoint,
  LIVE_WITHIN_MS,
  MetricStore,
} from './metric-store'
import { metricsView } from './metrics-view'
import { pollMetrics } from './neo-metrics'
import { receiverRoutes } from './otlp-receiver'
import { parsePrometheus } from './prometheus-text'
import { spanRecords } from './span-records'
import { type SpanLimits, SpanStore } from './span-store'

export type { HealthPoint } from './metric-store'

export interface TelemetryDeps {
  /** A host API is configured: scrape its `GET /v1/metrics`. */
  hostApi: boolean
  now: () => number
  /** Test seams. */
  scrape?: {
    fetch?: (path: string, init?: RequestInit) => Promise<Response>
    intervalMs?: number
    retryMs?: number
  }
  readWaitMs?: number
  firstSpanGraceMs?: number
  liveWithinMs?: number
  spanLimits?: SpanLimits
  /** Health events: at most one per this many ms. */
  emitEveryMs?: number
}

/** What export writes under `metrics/`. */
export interface TelemetryExport {
  /**
   * The last scrape's text, byte for byte; null when the exported numbers came by push (or the
   * scrape never answered), so the file always matches the summary beside it.
   */
  prometheus: string | null
  /** The door shown last (live, else the one heard from last). */
  metrics: MetricsView
  health: TelemetryHealth | null
  /** That door's history: one point a second, at most ten minutes, cumulative. */
  history: HealthPoint[]
}

export interface Telemetry {
  /** `POST /otlp/v1/traces` and `POST /otlp/v1/metrics`, for `Bun.serve({ routes })`. */
  readonly receiverRoutes: Record<
    string,
    { POST: (req: Request) => Promise<Response> }
  >
  /** Records from spans: `joinNeoRuns`' source when the viewer has no host API. */
  readonly records: RunRecordSource
  /** Null until the first metric sample or neo span. */
  health(): TelemetryHealth | null
  /** Each health change, at most once a second. Returns the unsubscribe. */
  subscribe(listener: (health: TelemetryHealth) => void): () => void
  /** The Health view's data from the live door; null when none is live. */
  metrics(): MetricsView | null
  /** A run's waterfall by its Neo run id, or by the span id the viewer minted for it. */
  runTrace(binding: {
    neoRunId: string | null
    mintedSpanId: string | null
  }): RunTrace | null
  /** Null when no metric ever arrived (an older image). */
  exportSnapshot(): TelemetryExport | null
  /** The log was cleared: `sinceClear` counts from here. */
  rebase(): void
  /** Stops the poller and timers, and wakes every waiting read with what it has. */
  stop(): Promise<void>
}

const EMIT_EVERY_MS = 1_000

export function startTelemetry(deps: TelemetryDeps): Telemetry {
  const { now } = deps
  const liveWithinMs = deps.liveWithinMs ?? LIVE_WITHIN_MS
  const spans = new SpanStore(deps.spanLimits)
  const store = new MetricStore(liveWithinMs)
  const records = spanRecords(spans, {
    now,
    ...(deps.readWaitMs === undefined ? {} : { waitMs: deps.readWaitMs }),
    ...(deps.firstSpanGraceMs === undefined
      ? {}
      : { firstSpanGraceMs: deps.firstSpanGraceMs }),
  })
  const listeners = new Set<(health: TelemetryHealth) => void>()
  let rev = 0
  let shownDoor: Door | null = null
  let lastEmit = 0
  let emitTimer: ReturnType<typeof setTimeout> | null = null
  let staleTimer: ReturnType<typeof setTimeout> | null = null
  let stopped = false
  const emitEvery = deps.emitEveryMs ?? EMIT_EVERY_MS

  const health = (): TelemetryHealth | null => {
    if (!store.hasSamples() && !spans.spansSeen) return null
    const door = store.active(now())
    const latest = door ? store.latest(door) : null
    const totals = door ? store.totals(door) : null
    return {
      rev,
      observedAt: latest?.at ?? null,
      source: door,
      spansSeen: spans.spansSeen,
      runsInFlight: totals?.runsInFlight ?? null,
      tabsOpen: totals?.tabsOpen ?? null,
      sinceClear: door ? store.sinceClear(door) : null,
    }
  }

  const emit = () => {
    emitTimer = null
    lastEmit = now()
    const current = health()
    if (!current) return
    for (const listener of listeners) listener(current)
  }

  /** Something health shows changed: a new `rev`, sent at most once per `emitEvery`. */
  const changed = () => {
    rev++
    if (stopped || emitTimer) return
    emitTimer = setTimeout(emit, Math.max(0, lastEmit + emitEvery - now()))
  }

  /** The shown door can change with no sample at all (one goes quiet): look again when it would. */
  const watchDoor = () => {
    if (staleTimer) clearTimeout(staleTimer)
    staleTimer = null
    const door = store.active(now())
    if (door !== shownDoor) {
      shownDoor = door
      changed()
    }
    const latest = door ? store.latest(door) : null
    if (latest && !stopped)
      staleTimer = setTimeout(
        watchDoor,
        Math.max(0, latest.at + liveWithinMs - now()) + 5,
      )
  }

  const sample = () => {
    changed()
    watchDoor()
  }

  const receiver = receiverRoutes({
    spans(batch) {
      const { changed: runs, rejected } = spans.add(batch)
      if (runs.size > 0) {
        records.changed(runs)
        changed()
      }
      return { rejected }
    },
    metrics(families) {
      store.pushedFamilies(families, now())
      sample()
    },
  })

  const poller = deps.hostApi
    ? pollMetrics({
        fetch: deps.scrape?.fetch ?? hostFetch,
        now,
        onText(text, at) {
          store.scraped(parsePrometheus(text), text, at)
          sample()
        },
        ...(deps.scrape?.intervalMs === undefined
          ? {}
          : { intervalMs: deps.scrape.intervalMs }),
        ...(deps.scrape?.retryMs === undefined
          ? {}
          : { retryMs: deps.scrape.retryMs }),
      })
    : null

  return {
    receiverRoutes: receiver,
    records,
    health,
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    metrics() {
      const door = store.active(now())
      const latest = door ? store.latest(door) : null
      return door && latest
        ? metricsView(latest.families, door, latest.at)
        : null
    },
    runTrace({ neoRunId, mintedSpanId }) {
      const id =
        neoRunId ?? (mintedSpanId ? spans.runUnder(mintedSpanId) : null)
      const run = id ? spans.get(id) : null
      return run ? buildTrace(run) : null
    },
    exportSnapshot() {
      const door = store.newest(now())
      const latest = door ? store.latest(door) : null
      if (!door || !latest) return null
      return {
        // The raw text only beside a summary read from that same scrape, never a push's.
        prometheus: door === 'scrape' ? store.scrapeText() : null,
        metrics: metricsView(latest.families, door, latest.at),
        health: health(),
        history: store.points(door),
      }
    },
    rebase() {
      store.rebase()
      changed()
    },
    async stop() {
      stopped = true
      if (emitTimer) clearTimeout(emitTimer)
      if (staleTimer) clearTimeout(staleTimer)
      records.stop()
      await poller?.stop()
    },
  }
}
