/**
 * Neo's run records made from its spans: the viewer's record source when it has no host API
 * (`joinNeoRuns(store, telemetry.records)`). A read finds the run's spans by `neo.run.id` (the
 * reply's `neoRunId`), or, for a run with no reply, by the `neo.run` span whose parent is the
 * span id the viewer minted for it, then waits until `neo.run` arrives (the run is whole) and
 * returns `foldSpans` of what it has.
 *
 * Spans trail the reply by up to Neo's batch delay (500 ms for the viewer's container), so a read
 * waits; but an image that never pushes would make every read wait in vain, slowing Copy and
 * Export. So until any neo span has arrived in this process, a read waits at most 2 s, not 10 s.
 */

import type { NeoRunRecord } from '../../shared/api'
import type { RunKey, RunRecordSource } from '../neo-runs'
import { foldSpans } from './fold-spans'
import type { RunSpans, SpanStore } from './span-store'

export interface SpanRecordOptions {
  now: () => number
  /** The longest a read waits for a run's `neo.run`. */
  waitMs?: number
  /** The same while no neo span has arrived at all. */
  firstSpanGraceMs?: number
}

export interface SpanRecords extends RunRecordSource {
  /** Spans arrived for these runs: wakes the reads waiting on them. */
  changed(neoRunIds: Set<string>): void
  /** Wakes every read with what it has; reads after this do not wait. */
  stop(): void
}

const WAIT_MS = 10_000
const FIRST_SPAN_GRACE_MS = 2_000
/**
 * Records handed out, and reads that ran out with nothing, remembered so a late span can announce
 * the run (at most this many of each).
 */
const REMEMBERED = 1_000

export function spanRecords(
  spans: SpanStore,
  options: SpanRecordOptions,
): SpanRecords {
  const waitMs = options.waitMs ?? WAIT_MS
  const graceMs = options.firstSpanGraceMs ?? FIRST_SPAN_GRACE_MS
  const wakers = new Set<() => void>()
  const lateListeners = new Set<(key: RunKey) => void>()
  /** neoRunId → the key it was read with and the size of the record given. */
  const delivered = new Map<
    string,
    { key: RunKey; size: string; operations: number }
  >()
  /**
   * Reads that found no span before their wait ran out (the first spans of a viewer's life can
   * trail a reply by more than the first-span grace), by Neo run id or minted span id: if the
   * run's spans come after all, it is read again.
   */
  const missed = new Map<string, RunKey>()
  const waiting = new Set<string>()
  let stopped = false

  const neoRunIdOf = (key: RunKey): string | null =>
    key.neoRunId ??
    (key.trace?.injected ? spans.runUnder(key.trace.spanId) : null)

  const deliver = (key: RunKey, neoRunId: string, record: NeoRunRecord) => {
    delivered.delete(neoRunId)
    delivered.set(neoRunId, {
      key,
      size: sizeOf(record),
      operations: record.operations.length,
    })
    if (delivered.size > REMEMBERED)
      delivered.delete(delivered.keys().next().value as string)
  }

  const miss = (key: RunKey) => {
    missed.set(key.neoRunId ?? `span:${key.trace?.spanId ?? ''}`, key)
    if (missed.size > REMEMBERED)
      missed.delete(missed.keys().next().value as string)
  }

  const announce = (key: RunKey) => {
    for (const listener of lateListeners) listener(key)
  }

  /** The record of what a run's spans say now, remembered as handed out. */
  const give = (key: RunKey, run: RunSpans): NeoRunRecord => {
    const record = foldSpans(run)
    deliver(key, run.neoRunId, record)
    return record
  }

  /**
   * A read's hold on its run: found by its key now, or once its `neo.run` names it (a minted
   * parent), and kept from eviction while the read waits on it.
   */
  const bind = (key: RunKey) => {
    let neoRunId: string | null = null
    let unpin = () => {}
    return {
      current(): RunSpans | null {
        if (!neoRunId) {
          neoRunId = neoRunIdOf(key)
          if (!neoRunId) return null
          unpin = spans.pin(neoRunId)
          waiting.add(neoRunId)
        }
        return spans.get(neoRunId)
      },
      release() {
        if (neoRunId) waiting.delete(neoRunId)
        unpin()
      },
    }
  }

  return {
    canRead: (key) => key.neoRunId !== null || key.trace?.injected === true,

    async read(key, signal) {
      const started = options.now()
      const bound = bind(key)
      try {
        for (;;) {
          const run = bound.current()
          if (run?.run) return give(key, run)
          const left =
            started + (spans.spansSeen ? waitMs : graceMs) - options.now()
          if (stopped || signal.aborted || left <= 0) {
            if (run && run.ops.size > 0) return give(key, run)
            miss(key)
            return null
          }
          await wake(wakers, left, signal)
        }
      } finally {
        bound.release()
      }
    },

    onLateCompletion(listener) {
      lateListeners.add(listener)
      return () => lateListeners.delete(listener)
    },

    changed(neoRunIds) {
      for (const waker of [...wakers]) waker()
      for (const neoRunId of neoRunIds) {
        const given = delivered.get(neoRunId)
        const run = spans.get(neoRunId)
        if (!given || !run || waiting.has(neoRunId)) continue
        const record = foldSpans(run)
        // A run the store dropped and a late span refiled holds less than was handed out: a
        // record only grows, so that one is never announced.
        if (
          record.operations.length < given.operations ||
          sizeOf(record) === given.size
        )
          continue
        announce(given.key)
      }
      for (const [id, key] of missed) {
        const neoRunId = neoRunIdOf(key)
        if (!neoRunId || !neoRunIds.has(neoRunId)) continue
        missed.delete(id)
        announce(key)
      }
    },

    stop() {
      stopped = true
      for (const waker of [...wakers]) waker()
    },
  }
}

/** How much a record holds: it only grows, so a change in this is a change in the record. */
function sizeOf(record: NeoRunRecord): string {
  return `${record.operations.length}/${record.complete}/${record.status}`
}

/** Resolves at the next `changed`/`stop`, after `ms`, or on abort. */
function wake(
  wakers: Set<() => void>,
  ms: number,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer)
      wakers.delete(done)
      signal.removeEventListener('abort', done)
      resolve()
    }
    const timer = setTimeout(done, ms)
    wakers.add(done)
    signal.addEventListener('abort', done, { once: true })
  })
}
