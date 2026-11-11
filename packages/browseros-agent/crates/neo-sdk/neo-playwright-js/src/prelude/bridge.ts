/**
 * The prelude's only contact with claw-sdk's run bootstrap
 * (crates/neo-sdk/neo-server/src/runtime/js/bootstrap.js). claw-sdk evaluates the prelude in each run's
 * fresh QuickJS realm after its bootstrap, so the prelude's state lives exactly as long as one
 * run. What it uses, one function each:
 *
 * - `__neo.callSite()`: captures the synchronous script frame at public API entry. The
 *   required `site` argument carries it through prelude awaits without shared mutable state.
 * - `__neo.invoke(name, input, options)`: resolves with an op's `ok` value, its brands revived;
 *   rejects with the bootstrap's NeoError, a TimeoutError when the op timed out. `options.signal`
 *   makes the op abortable (signal.ts).
 * - `__neo.brands.register(brand, fromJson)`: the reviver for one brand in results.
 * - `__neo.onReply(listener)`: every reply envelope, before its value is revived. Its `pages`
 *   facts keep `page.url()`, `isClosed()` and `context.pages()` current between ops, and its
 *   `defaultPage` names the tab the door opened for the run's lazy page.
 * - `__neo.provide(name, build)`: one parameter of the agent's function. `build(init)` runs
 *   once, when the run starts; `init.page` is the run's page brand when it already has a tab,
 *   and `init.pages`, when the run service sends it, lists every tab as reply facts do. A
 *   bootstrap with `refuse` (sdk-skeleton's S3) awaits a build that returns a promise, before
 *   the agent's first statement (FR/RC C5).
 * - `__neo.refuse(api, code, reason)`: a refusal the door records, which errors.ts raises
 *   through host.ts, where the bootstrap's type lives.
 * - `__neo_sleep(ms)`: the clock native the bootstrap's own `setTimeout` rides on, capped by
 *   the run's deadline and cut short by its stop. The bootstrap hands `setTimeout` to the
 *   agent's function only, so `waitForTimeout`, polling waits and the deadline a
 *   `waitForEvent` predicate races read the native directly.
 *
 * Each is read as a property of `globalThis`, and the prelude defines no global of its own:
 * the bundle names no host identifier, which tests/prelude-globals.test.ts checks, and
 * claw-sdk's runtime/tests.rs pins that a run's realm gains only `__neo` and its natives.
 */

import type { CallSite } from './call-site'
import { readyChoosers } from './chooser-setup'
import { shapeBridgeError, unsupportedError, withAbortCause } from './errors'
import type { OpName } from './generated/ops'
import { host, refuser } from './host'
import { completedPublications } from './publication'
import type { SignalLike } from './signal'
import type {
  CallHeader,
  Observation,
  OpInputs,
  OpOutputs,
  PageBrand,
  PageFact,
  WorkerInfo,
} from './wire'

/** What a run starts with (claw-sdk's run_script.rs `init`). */
export type RunInit = {
  /** The task's tab when the run service knows one; null opens one on the first op. */
  page: PageBrand | null
  /**
   * Every tab in the browser, as facts like a reply's, when the run service lists them
   * (`init.pages`); empty when it does not, and replies alone then name the other tabs.
   */
  pages: PageFact[]
  observed: Observation | null
}

/** What one reply says about pages. */
export type Reply = {
  /** The host observation that produced these facts. */
  observed: Observation | null
  /** Fresh facts about the pages the op touched, its target first. */
  pages: PageFact[]
  /** The tab of the run's own page, on the reply of an op the door opened it for. */
  defaultPage: { tab: number } | null
}

/**
 * Runs one op using the site captured before the public method's first await. The site is
 * transport metadata, never part of the probe's wire input. Inputs are plain JSON built by
 * the classes (never class instances, which the bootstrap's codec refuses), and the result is trusted as `OpOutputs` declares: Rust decoded
 * the input and owns the result's shape. Every failure is shaped once, here. A `signal` aborts
 * the op (signal.ts). The method checked it when the script called it; one that fired since is
 * the call in flight, which the bootstrap aborts as soon as it is sent.
 */
export async function invoke<Op extends OpName>(
  op: Op,
  site: CallSite,
  call: CallHeader,
  args: OpInputs[Op],
  signal?: SignalLike,
): Promise<OpOutputs[Op]> {
  try {
    const ready = readyChoosers(op)
    if (ready) await ready
    countPumps()
    const value = (await host().invoke(
      op,
      { call, ...args },
      { signal, site: site.value },
    )) as OpOutputs[Op]
    const held = pumpsCaughtUp(op)
    if (held) await held
    return value
  } catch (thrown) {
    const held = pumpsCaughtUp(op)
    if (held) await held
    throw withAbortCause(shapeBridgeError(op, call.api, thrown), signal)
  }
}

/**
 * A page-state result and its ordering evidence. An optional consumer reserves delivery
 * synchronously, before any caller awaiting this reply can run. `signal` as for [`invoke`].
 */
export async function invokeObserved<Op extends OpName>(
  op: Op,
  site: CallSite,
  call: CallHeader,
  args: OpInputs[Op],
  onResult?: (value: OpOutputs[Op], observed: Observation | null) => void,
  signal?: SignalLike,
): Promise<{ value: OpOutputs[Op]; observed: Observation | null }> {
  try {
    const ready = readyChoosers(op)
    if (ready) await ready
    countPumps()
    const reply = await host().invoke(
      op,
      { call, ...args },
      {
        site: site.value,
        observed: true,
        signal,
        onResult:
          onResult &&
          ((value, observed) =>
            onResult(value as OpOutputs[Op], readObservation(observed))),
      },
    )
    const held = pumpsCaughtUp(op)
    if (held) await held
    return {
      value: field(reply, 'value') as OpOutputs[Op],
      observed: readObservation(field(reply, 'observed')),
    }
  } catch (thrown) {
    const held = pumpsCaughtUp(op)
    if (held) await held
    throw withAbortCause(shapeBridgeError(op, call.api, thrown), signal)
  }
}

/**
 * The pump ledger. Rust holds an op that ends until the run's events pump has taken the events
 * that came before it (`Playwright::events_before_reply`), and every reply envelope says how many
 * pump replies Rust counted by then (`pumped`). The host settles each op's promise on its own, in
 * no fixed order, so an op's result could reach the script before the pump reply that carries its
 * events; the op's result waits here until the pump dispatched that many replies, as upstream's
 * listeners have run before the call that preceded them resolves.
 *
 * The prelude never counts pump replies itself: Rust counts only replies that carry events, and
 * a pump reply's own envelope (read here before its value) carries the count including it. One
 * pump runs at a time, so once one of its replies, a value or an error, is dispatched, so is
 * every reply counted so far. Every counted reply exists, so a hold ends; it is bounded anyway,
 * for a pump the host cut off at the run's end.
 */
const PUMP_HOLD_MS = 1000
let pumpsNeeded = 0
let pumpsDispatched = 0
let countingPumps = false
const pumpHolds: { needed: number; release: () => void }[] = []

/** Starts reading `pumped` off every reply, once, before the first op is sent. */
function countPumps(): void {
  if (countingPumps) return
  countingPumps = true
  host().onReply((envelope) => {
    completedPublications(field(envelope, 'completedNavigations'))
    const pumped = field(envelope, 'pumped')
    if (typeof pumped === 'number' && pumped > pumpsNeeded) pumpsNeeded = pumped
  })
}

/** The events pump dispatched a reply, a value or an error (events.ts): it caught up. */
export function pumpDispatched(): void {
  pumpsDispatched = Math.max(pumpsDispatched, pumpsNeeded)
  for (const hold of pumpHolds.filter((hold) => hold.needed <= pumpsDispatched))
    hold.release()
}

/**
 * Settles once the pump dispatched every reply the envelopes so far counted; at once when it
 * has, so an op with nothing to wait for keeps its timing. The event ops are the pump and the
 * waits themselves, which Rust does not hold either.
 */
function pumpsCaughtUp(op: string): Promise<void> | undefined {
  const needed = pumpsNeeded
  if (op.startsWith('events.') || pumpsDispatched >= needed) return undefined
  return new Promise((resolve) => {
    let cancel = () => {}
    const release = () => {
      cancel()
      const index = pumpHolds.indexOf(hold)
      if (index >= 0) pumpHolds.splice(index, 1)
      resolve()
    }
    const hold = { needed, release }
    pumpHolds.push(hold)
    cancel = startTimer(PUMP_HOLD_MS, release)
  })
}

/**
 * The reviver for `brand` in op results. `bytes` replaces the bootstrap's own reviver, whose
 * plain Uint8Array the prelude's Buffer subclasses.
 */
export function registerBrand(
  brand: 'page' | 'element' | 'bytes',
  fromJson: (json: object) => unknown,
): void {
  host().brands.register(brand, fromJson)
}

export function onReply(listener: (reply: Reply) => void): void {
  host().onReply((envelope) => {
    const observed = readObservation(field(envelope, 'observed'))
    const result = field(envelope, 'ok')
    if (
      observed &&
      (Array.isArray(field(result, 'events')) ||
        typeof field(result, 'event') === 'string')
    )
      observed.deferWorkers = true
    // A failed event read returns no records for the hub to deliver. Its closed facts
    // must settle now, even if the host copied a close before the operation failed.
    if (observed && field(envelope, 'err') !== undefined) observed.closes = []
    listener({
      observed,
      pages: readPageFacts(field(envelope, 'pages')),
      defaultPage: readTab(field(envelope, 'defaultPage')),
    })
  })
}

/**
 * Gives the agent's function its `name` parameter, built once from the run's init. A build may
 * return a promise only for a bootstrap that awaits it (`awaitsProviders`): an older one would
 * hand the script the promise itself.
 */
export function provide(name: string, build: (init: RunInit) => unknown): void {
  host().provide(name, (init) => {
    const page = readTab(field(init, 'page'))
    return build({
      page: page ? { $neo: 'page', ...page } : null,
      pages: readPageFacts(field(init, 'pages')),
      observed: readObservation(field(init, 'observed')),
    })
  })
}

/**
 * Whether the bootstrap awaits a parameter's build before the agent's first statement (FR/RC
 * C5). It landed with `refuse` (C1), in sdk-skeleton's S3, so the one tells of the other.
 */
export function awaitsProviders(): boolean {
  return refuser() !== null
}

function field(value: unknown, name: string): unknown {
  return typeof value === 'object' && value !== null
    ? Reflect.get(value, name)
    : undefined
}

/** A tab-only identity from a page brand or a default-page reply. */
function readTab(value: unknown): { tab: number } | null {
  const tab = field(value, 'tab')
  return typeof tab === 'number' ? { tab } : null
}

/** Invalid or absent stamps confer identity only, without changing cached lifecycle. */
function readObservation(value: unknown): Observation | null {
  const seq = field(value, 'seq')
  const events = field(value, 'events')
  if (
    typeof seq !== 'number' ||
    !Number.isFinite(seq) ||
    typeof events !== 'number' ||
    !Number.isFinite(events)
  )
    return null
  const result: Observation = { seq, events }
  const payloadCurrent = field(value, 'payloadCurrent')
  if (typeof payloadCurrent === 'boolean')
    result.payloadCurrent = payloadCurrent
  const closes = field(value, 'closes')
  if (Array.isArray(closes)) {
    result.closes = []
    for (const close of closes) {
      const tab = field(close, 'tab')
      const at = field(close, 'seq')
      if (
        typeof tab === 'number' &&
        Number.isFinite(tab) &&
        typeof at === 'number' &&
        Number.isFinite(at)
      )
        result.closes.push({ tab, seq: at })
    }
  }
  return result
}

/** Page facts from the host, dropping anything without a numeric tab. */
function readPageFacts(pages: unknown): PageFact[] {
  if (!Array.isArray(pages)) return []
  const facts: PageFact[] = []
  for (const page of pages) {
    if (typeof page !== 'object' || page === null) continue
    const tab: unknown = Reflect.get(page, 'tab')
    if (typeof tab !== 'number') continue
    const fact: PageFact = { tab }
    const taskId: unknown = Reflect.get(page, 'taskId')
    const url: unknown = Reflect.get(page, 'url')
    const title: unknown = Reflect.get(page, 'title')
    const closed: unknown = Reflect.get(page, 'closed')
    const viewport: unknown = Reflect.get(page, 'viewportSize')
    if (typeof url === 'string') fact.url = url
    if (taskId === null || typeof taskId === 'string') fact.taskId = taskId
    if (typeof title === 'string') fact.title = title
    if (typeof closed === 'boolean') fact.closed = closed
    if (viewport === null) fact.viewportSize = null
    else if (typeof viewport === 'object' && viewport !== null) {
      const width: unknown = Reflect.get(viewport, 'width')
      const height: unknown = Reflect.get(viewport, 'height')
      if (typeof width === 'number' && typeof height === 'number')
        fact.viewportSize = { width, height }
    }
    const workers = readWorkers(field(page, 'workers'))
    if (workers) fact.workers = workers
    facts.push(fact)
  }
  return facts
}

/** Worker metadata is validated once at the reply boundary, independently of page facts. */
function readWorkers(value: unknown): WorkerInfo[] | null {
  if (!Array.isArray(value)) return null
  const workers: WorkerInfo[] = []
  for (const worker of value) {
    const id = field(worker, 'id')
    const url = field(worker, 'url')
    if (typeof id === 'string' && typeof url === 'string')
      workers.push({ id, url })
  }
  return workers
}

/**
 * One `error` line in the run's log, where the script's own `console.error` writes and the
 * caller reads it: the bootstrap's `__neo_log` native, read as `__neo_sleep` is. A realm
 * without it drops the line.
 */
export function logError(text: string): void {
  const log: unknown = Reflect.get(globalThis, '__neo_log')
  if (typeof log === 'function') Reflect.apply(log, globalThis, ['error', text])
}

/** The bootstrap's clock, or null in a realm without one. */
function clock(): ((ms: number) => Promise<unknown>) | null {
  const sleep: unknown = Reflect.get(globalThis, '__neo_sleep')
  return typeof sleep === 'function'
    ? (ms) => Promise.resolve(Reflect.apply(sleep, globalThis, [ms]))
    : null
}

/** Resolves after `ms` on the bootstrap's clock, which the run deadline caps. */
export function sleep(api: string, ms: number): Promise<void> {
  const after = clock()
  if (!after)
    return Promise.reject(
      unsupportedError(
        api,
        'this runtime has no timers; wait on the page instead',
      ),
    )
  return after(ms).then(() => undefined)
}

/**
 * Calls `fire` after `ms` on the bootstrap's clock, and returns what cancels it. A runtime
 * without a clock never fires it.
 */
export function startTimer(ms: number, fire: () => void): () => void {
  const after = clock()
  if (!after) return () => {}
  let live = true
  void after(ms).then(() => {
    if (live) fire()
  })
  return () => {
    live = false
  }
}
