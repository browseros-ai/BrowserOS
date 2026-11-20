/**
 * Neo's metrics as they arrive through either door: the scrape of `GET /v1/metrics` and the OTLP
 * push. Each door keeps its own slot (its latest families), its own history and its own count
 * since the last Clear; which door the viewer shows is a pure function of the two slots' ages
 * (`active`): the scrape while it is at most 10 s old, else the push while it is, else none. The
 * doors are never blended: both read one meter provider, so either alone is the whole truth, and
 * a fixed order means the numbers never jump between them while both are live.
 */

import type { MetricFamily } from '../../shared/api'
import { type Totals, totalsOf } from './metrics-view'

export type Door = 'scrape' | 'push'

/** One sample of the numbers health is made of, cumulative (export's history). */
export interface HealthPoint extends Totals {
  at: number
}

/** Counts since the last Clear (`TelemetryHealth.sinceClear`). */
export type SinceClear = Pick<
  Totals,
  'runs' | 'runErrors' | 'ops' | 'opErrors' | 'crashes'
>

const COUNTERS = ['runs', 'runErrors', 'ops', 'opErrors', 'crashes'] as const

/** A door's samples stop counting as live past this age. */
export const LIVE_WITHIN_MS = 10_000
/** History: at most one point a second, ten minutes of them. */
const POINT_EVERY_MS = 1_000
const HISTORY_POINTS = 600
/** Bounds on what one door may hold (Neo has 11 families and a few hundred series). */
const MAX_FAMILIES = 512
const MAX_SERIES = 20_000

interface Slot {
  families: MetricFamily[]
  at: number
  totals: Totals
}

interface Counter {
  /** The totals counted from: the last sample (null: the next sample is the baseline). */
  last: Totals | null
  since: SinceClear
}

export class MetricStore {
  private readonly slots: Record<Door, Slot | null> = {
    scrape: null,
    push: null,
  }
  /**
   * Pushes may split families over requests: each push replaces the families it carries, and
   * each family remembers when it last came.
   */
  private readonly pushed = new Map<
    string,
    { family: MetricFamily; at: number }
  >()
  private readonly history: Record<Door, HealthPoint[]> = {
    scrape: [],
    push: [],
  }
  private readonly counters: Record<Door, Counter> = {
    scrape: { last: null, since: zero() },
    push: { last: null, since: zero() },
  }
  /** The last scrape's body, byte for byte (export's `metrics.prom`). */
  private lastScrapeText: string | null = null

  constructor(private readonly liveWithinMs = LIVE_WITHIN_MS) {}

  /** A scrape's families and the text they came from. */
  scraped(families: MetricFamily[], text: string, at: number): void {
    this.lastScrapeText = text
    this.put('scrape', bounded(families), at)
  }

  /**
   * A push's families, merged by name into what earlier pushes said. A pusher re-sends every
   * family it has recorded at each interval (cumulative), so one not heard for as long as a door
   * counts as live was not sent by the Neo pushing now: after a restart, the old process's
   * counters must not sit beside the new one's. The merged set is bounded as one push is.
   */
  pushedFamilies(families: MetricFamily[], at: number): void {
    for (const family of families) this.pushed.set(family.name, { family, at })
    for (const [name, entry] of this.pushed)
      if (at - entry.at > this.liveWithinMs) this.pushed.delete(name)
    const kept = bounded([...this.pushed.values()].map((entry) => entry.family))
    const keptNames = new Map(kept.map((family) => [family.name, family]))
    for (const [name, entry] of this.pushed) {
      const family = keptNames.get(name)
      if (family) entry.family = family
      else this.pushed.delete(name)
    }
    this.put('push', kept, at)
  }

  /** Which door the viewer shows now; null when neither is live. */
  active(now: number): Door | null {
    for (const door of ['scrape', 'push'] as const) {
      const slot = this.slots[door]
      if (slot && now - slot.at <= this.liveWithinMs) return door
    }
    return null
  }

  /** The door's latest families and their time. */
  latest(door: Door): { families: MetricFamily[]; at: number } | null {
    const slot = this.slots[door]
    return slot ? { families: slot.families, at: slot.at } : null
  }

  /** The door shown now, else the one heard from last (an export after Neo stopped). */
  newest(now: number): Door | null {
    const live = this.active(now)
    if (live) return live
    const { scrape, push } = this.slots
    if (scrape && push) return scrape.at >= push.at ? 'scrape' : 'push'
    return scrape ? 'scrape' : push ? 'push' : null
  }

  totals(door: Door): Totals | null {
    return this.slots[door]?.totals ?? null
  }

  sinceClear(door: Door): SinceClear {
    return { ...this.counters[door].since }
  }

  points(door: Door): HealthPoint[] {
    return [...this.history[door]]
  }

  scrapeText(): string | null {
    return this.lastScrapeText
  }

  /** Any sample has arrived through either door. */
  hasSamples(): boolean {
    return this.slots.scrape !== null || this.slots.push !== null
  }

  /**
   * The log was cleared: count from Neo's counters as they stand now (the last sample), or from
   * the next sample when there is none yet.
   */
  rebase(): void {
    for (const door of ['scrape', 'push'] as const)
      this.counters[door].since = zero()
  }

  private put(door: Door, families: MetricFamily[], at: number): void {
    const totals = totalsOf(families)
    this.slots[door] = { families, at, totals }
    count(this.counters[door], totals)
    const history = this.history[door]
    const last = history.at(-1)
    // One point per clock second: a later sample in the same second replaces it.
    if (
      last &&
      Math.floor(at / POINT_EVERY_MS) === Math.floor(last.at / POINT_EVERY_MS)
    )
      history[history.length - 1] = { ...totals, at }
    else history.push({ ...totals, at })
    if (history.length > HISTORY_POINTS)
      history.splice(0, history.length - HISTORY_POINTS)
  }
}

/** Adds what each counter rose by since the last sample; one that fell restarted at zero. */
function count(counter: Counter, totals: Totals): void {
  const last = counter.last
  if (last)
    for (const name of COUNTERS) {
      const now = totals[name]
      counter.since[name] += now >= last[name] ? now - last[name] : now
    }
  counter.last = totals
}

function zero(): SinceClear {
  return { runs: 0, runErrors: 0, ops: 0, opErrors: 0, crashes: 0 }
}

/** At most `MAX_FAMILIES` families and `MAX_SERIES` series in all; the rest are dropped. */
function bounded(families: MetricFamily[]): MetricFamily[] {
  let left = MAX_SERIES
  const kept: MetricFamily[] = []
  for (const family of families.slice(0, MAX_FAMILIES)) {
    if (left <= 0) break
    const series = family.series.slice(0, left)
    left -= series.length
    kept.push(
      series.length === family.series.length ? family : { ...family, series },
    )
  }
  return kept
}
