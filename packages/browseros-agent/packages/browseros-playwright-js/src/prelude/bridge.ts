/**
 * The prelude's only contact with claw-sdk's run bootstrap (sdk-skeleton D8). claw-sdk
 * evaluates the prelude in each run's fresh QuickJS runtime after its bootstrap, so the
 * prelude's state lives exactly as long as one run. What it relies on, one function each:
 *
 * - `__neo.invoke(name, input)` (documented): resolves with an op's `ok` value; rejects with the
 *   bootstrap's NeoError, a TimeoutError when the op timed out, holding the envelope's fields.
 * - `__neo.handles.register(brand, fromJson)` (documented): revives brands in results. Results
 *   are read as raw brands too, so a bootstrap that revives nothing still works.
 * - `__neo.onPageFacts(listener)` (assumed): every envelope's `pages` facts, which keep
 *   `page.url()`, `isClosed()` and `context.pages()` current between ops.
 * - `__neoPlaywrightSeed` (assumed): the run's default page and the tabs open at start, for the
 *   synchronous `context.pages()` and the `page` global.
 * - `setTimeout` and `clearTimeout` (the bootstrap's, over `__neo_sleep`): `waitForTimeout`,
 *   polling waits, and the deadline a `waitForEvent` predicate races.
 *
 * Each is read as a property of `globalThis`. The bundle names no host identifier, which
 * tests/prelude-globals.test.ts checks, so the bootstrap must define these as global object
 * properties rather than top-level `const` bindings.
 */
import { shapeBridgeError, unsupportedError } from './errors'
import type { OpName } from './ops.generated'
import type { CallHeader, OpInputs, OpOutputs, PageFact } from './wire'

type Bootstrap = {
  invoke(name: string, input?: unknown): Promise<unknown>
  handles?: {
    register(brand: string, fromJson: (json: object) => unknown): void
  }
  onPageFacts?(listener: (pages: unknown) => void): void
}

/** What the host knows when the run starts. */
export type Seed = {
  /** The run's default page (the task's last tab), or null to open one on first use. */
  page: number | null
  pages: PageFact[]
  browserVersion: string
}

function host(): Bootstrap {
  const neo: unknown = Reflect.get(globalThis, '__neo')
  if (typeof neo !== 'object' || neo === null)
    throw new Error(
      "The Playwright prelude needs claw-sdk's bootstrap (globalThis.__neo), which was not installed",
    )
  return neo as Bootstrap
}

/**
 * Runs one op. Inputs are plain JSON built by the classes (never class instances, which the
 * bootstrap's codec refuses), and the result is trusted as `OpOutputs` declares: Rust decoded
 * the input and owns the result's shape. Every failure is shaped once, here.
 */
export async function invoke<Op extends OpName>(
  op: Op,
  call: CallHeader,
  args: OpInputs[Op],
): Promise<OpOutputs[Op]> {
  try {
    return (await host().invoke(op, { call, ...args })) as OpOutputs[Op]
  } catch (thrown) {
    throw shapeBridgeError(op, call.api, thrown)
  }
}

export function registerBrand(
  brand: 'page' | 'element',
  fromJson: (json: object) => unknown,
): void {
  host().handles?.register(brand, fromJson)
}

export function onPageFacts(listener: (facts: PageFact[]) => void): void {
  host().onPageFacts?.((pages) => listener(readPageFacts(pages)))
}

/** The run-start seed, or an empty one: no known tabs, and a page opened on first use. */
export function readSeed(): Seed {
  const seed: unknown = Reflect.get(globalThis, '__neoPlaywrightSeed')
  if (typeof seed !== 'object' || seed === null)
    return { page: null, pages: [], browserVersion: '' }
  const page: unknown = Reflect.get(seed, 'page')
  const version: unknown = Reflect.get(seed, 'browserVersion')
  return {
    page: typeof page === 'number' ? page : null,
    pages: readPageFacts(Reflect.get(seed, 'pages')),
    browserVersion: typeof version === 'string' ? version : '',
  }
}

/** Page facts from the host, dropping anything without a numeric id. */
function readPageFacts(pages: unknown): PageFact[] {
  if (!Array.isArray(pages)) return []
  const facts: PageFact[] = []
  for (const page of pages) {
    if (typeof page !== 'object' || page === null) continue
    const id: unknown = Reflect.get(page, 'id')
    if (typeof id !== 'number') continue
    const fact: PageFact = { id }
    const tab: unknown = Reflect.get(page, 'tab')
    if (typeof tab === 'number') fact.tab = tab
    const url: unknown = Reflect.get(page, 'url')
    const title: unknown = Reflect.get(page, 'title')
    const closed: unknown = Reflect.get(page, 'closed')
    if (typeof url === 'string') fact.url = url
    if (typeof title === 'string') fact.title = title
    if (typeof closed === 'boolean') fact.closed = closed
    facts.push(fact)
  }
  return facts
}

/** Resolves after `ms` on the bootstrap's timer, which the run deadline caps. */
export function sleep(api: string, ms: number): Promise<void> {
  const setTimer: unknown = Reflect.get(globalThis, 'setTimeout')
  if (typeof setTimer !== 'function')
    return Promise.reject(
      unsupportedError(
        api,
        'this runtime has no timers; wait on the page instead',
      ),
    )
  return new Promise((resolve) => {
    Reflect.apply(setTimer, globalThis, [resolve, ms])
  })
}

/**
 * Calls `fire` after `ms` on the bootstrap's timer, and returns what cancels it. A runtime
 * without timers never fires it.
 */
export function startTimer(ms: number, fire: () => void): () => void {
  const setTimer: unknown = Reflect.get(globalThis, 'setTimeout')
  if (typeof setTimer !== 'function') return () => {}
  const id: unknown = Reflect.apply(setTimer, globalThis, [fire, ms])
  return () => {
    const clearTimer: unknown = Reflect.get(globalThis, 'clearTimeout')
    if (typeof clearTimer === 'function')
      Reflect.apply(clearTimer, globalThis, [id])
  }
}
