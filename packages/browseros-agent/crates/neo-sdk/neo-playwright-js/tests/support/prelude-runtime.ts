// Runs the built prelude asset the way claw-sdk does: a fresh realm per run, claw-sdk's own
// bootstrap first (crates/neo-sdk/neo-server/src/runtime/js/bootstrap.js, read as it is, so a drift
// between the two fails here), then the prelude, then the run. The realm gets the bootstrap's
// three natives: `__neo_invoke` answers each op from the test's `answer` as one reply
// envelope, `__neo_log` drops console lines, and `__neo_sleep` is Node's timer.
//
// claw-sdk starts a realm's run once, through `__neo.run`, which builds the agent function's
// parameters from the run's init before that function's first await. The harness starts it
// with a function that keeps them, and `run(code)` then calls each script body with them, so
// a test sees the script's own value or error rather than the run's settled JSON. `settle(code)`
// is the other view: the body goes through `__neo.run` itself, and the test sees how the run
// ended, as claw-sdk's run_script.rs `settle` reports it.
import { readFileSync } from 'node:fs'
import { createContext, runInContext } from 'node:vm'
import type { Observation } from '../../src/prelude/generated/envelope'

export type Call = {
  name: string
  input: Record<string, unknown>
  /**
   * For a call the script can abort (`signal`): the realm's key for it, and the reason once the
   * bootstrap aborted it, as bridge.rs registers the key and answers `$neo.abort`.
   */
  abort?: { key: string; aborted: Promise<string> }
}

/** An op's answer: a value, or an envelope (`{ ok }`, `{ err }`, with `pages`, `defaultPage`). */
type Envelope = {
  ok?: unknown
  err?: Record<string, unknown>
  pages?: unknown[]
  defaultPage?: { tab: number }
  observed?: Observation | null
}
export type Answer = (call: Call) => unknown | Promise<unknown>

/** The unmodified strings crossing the bootstrap's native engine boundary. */
export type WireExchange = { op: string; args: string; reply: string }

/** How a run ended, as claw-sdk's run_script.rs `settle` reports it. */
export type Settled =
  | { ok: unknown }
  /** An uncaught NeoError of an op: the failure the door answered that op with, or null. */
  | { operation: Record<string, unknown> | null }
  /** Anything else the script threw: claw-sdk's `script_error`. */
  | { error: { name: string; message: string } }

const agentRoot = new URL('../../../../../', import.meta.url)

/** The committed prelude asset, read when a runtime is built without prelude text. */
const committedPrelude = (): string =>
  readFileSync(
    new URL('crates/neo-sdk/neo-playwright/assets/prelude.js', agentRoot),
    'utf8',
  )

const bootstrap = (): string =>
  readFileSync(
    new URL('crates/neo-sdk/neo-server/src/runtime/js/bootstrap.js', agentRoot),
    'utf8',
  )

/** claw-sdk evaluates the bootstrap, the preludes and the agent's function as strict code. */
const strict = (source: string): string => `'use strict';${source}`

export type Runtime = {
  /**
   * Every op the prelude sent once the run started, decoded from the JSON the bridge carried:
   * the script's own, and the refusals the door recorded for it.
   */
  calls: Call[]
  /** What a run sent before the agent's first statement: the run page's frame read (FR-1). */
  startCalls: Call[]
  /** Runs a script body (`return await page.title()`) as claw-sdk runs `run(code)`. */
  run(code: string): Promise<unknown>
  /**
   * Runs a script body through `__neo.run` with the run's init, and says how the run ended.
   * A second `__neo.run` in the realm: the providers give a seeded run the same objects.
   */
  settle(code: string): Promise<Settled>
  /**
   * Resolves once the run's parameters arrived: a bootstrap that awaits each parameter's build
   * (sdk-skeleton's S3, FR/RC C5) calls the agent's function only after a turn of the realm.
   */
  ready: Promise<void>
  /**
   * Evaluates an expression where the script's parameters (`page`, `context`) are in scope,
   * once `ready` resolved.
   */
  evaluate(code: string): unknown
  /** The ops named `name`, in order. */
  sent(name: string): Call[]
  /** Every `$neo.abort` the bootstrap sent, which the bridge answers and never records. */
  aborts: { key: string; reason: string }[]
}

export function loadPrelude(
  options: {
    answer?: Answer
    /**
     * The run's init (run_script.rs): `{ page }`, the run's page brand, and `pages`, every tab's
     * facts, once the run service sends them. Absent: no tab yet.
     */
    seed?: {
      page: unknown
      pages?: unknown[]
      observed?: Observation | null
    }
    /** The prelude's source; the committed asset when absent (build.ts passes a fresh build). */
    prelude?: string
    /** A fixed `Date.now()` for the realm, so recorded `since` stamps are reproducible. */
    now?: number
    /** Observe transport bytes before any test-side parsing or result normalization. */
    onWire?: (exchange: WireExchange) => void
    /**
     * The bootstrap as it was before sdk-skeleton's FR/RC: its `__neo` without `refuse`, which
     * the prelude takes for an older bootstrap.
     */
    withoutRefuse?: boolean
    /**
     * Sets the conformance switch the spec harness sets (`__neo_upstream_snapshots`), before
     * the prelude starts: snapshots keep Playwright's modes and text.
     */
    upstreamSnapshots?: boolean
  } = {},
): Runtime {
  const calls: Call[] = []
  const startCalls: Call[] = []
  const aborts: { key: string; reason: string }[] = []
  // Each abortable call's waiter, by the realm's key, as bridge.rs keeps its abort switches.
  const abortable = new Map<string, (reason: string) => void>()
  let refusals = 0
  let observations = options.seed?.observed?.seq ?? 0
  let newestEventSeq = options.seed?.observed?.events ?? 0
  // Each failed op's error, by op id, as bridge.rs remembers the failures it answered.
  const failures = new Map<unknown, Record<string, unknown>>()
  const answer =
    options.answer ??
    ((call: Call) => (call.name === 'page.evaluate' ? { value: null } : null))
  // The vm rig supplies neo-sdk's realm contract. Production installation and semantics are
  // tested through the real QuickJS run_script, rather than proved by these host implementations.
  const realm = createContext({
    ...(options.upstreamSnapshots ? { __neo_upstream_snapshots: true } : {}),
    URL,
    URLSearchParams,
    atob,
    btoa,
    DOMException,
    __neo_invoke: async (name: string, inputJson: string): Promise<string> => {
      const recordReply = (value: unknown): string => {
        const reply = JSON.stringify(value)
        options.onWire?.({ op: name, args: inputJson, reply })
        return reply
      }
      const input = JSON.parse(inputJson)
      if (name === '$neo.abort') {
        aborts.push(input)
        abortable.get(input.key)?.(input.reason)
        return recordReply({ ok: null, pages: [] })
      }
      const call: Call =
        input?.$neo === 'abortable'
          ? {
              name,
              input: input.input,
              abort: {
                key: input.key,
                aborted: new Promise((resolve) =>
                  abortable.set(input.key, resolve),
                ),
              },
            }
          : { name, input }
      calls.push(call)
      const refused = refusalReply(
        name,
        call.input,
        () => `refused.${++refusals}`,
      )
      if (refused) {
        failures.set(refused.err.operationId, refused.err)
        return recordReply(refused)
      }
      let reply: unknown
      try {
        reply = await answer(call)
      } catch (error) {
        reply = { err: error }
      }
      const envelope: Envelope =
        typeof reply === 'object' &&
        reply !== null &&
        ('ok' in reply || 'err' in reply)
          ? (reply as Envelope)
          : { ok: reply }
      if (envelope.err) failures.set(envelope.err.operationId, envelope.err)
      newestEventSeq = Math.max(newestEventSeq, eventHead(name, envelope.ok))
      if (envelope.observed === undefined) {
        envelope.observed = {
          seq: ++observations,
          events: newestEventSeq,
          payloadCurrent: true,
        }
        // A page payload always has a fact in a real host reply. Simple mocked values
        // inherit that identity so their payload and fact share the same observation.
        const target = call.input.page as { tab?: unknown } | undefined
        if (
          envelope.pages === undefined &&
          typeof target?.tab === 'number' &&
          [
            'page.frames',
            'page.goto',
            'page.reload',
            'page.goBack',
            'page.goForward',
            'page.waitForURL',
            'page.waitForLoadState',
            'page.setContent',
          ].includes(name)
        )
          envelope.pages = [{ tab: target.tab }]
      } else if (envelope.observed !== null)
        observations = Math.max(observations, envelope.observed.seq)
      return recordReply({ pages: [], ...envelope, ok: envelope.ok ?? null })
    },
    __neo_log: () => {},
    __neo_sleep: (ms: number) =>
      new Promise((resolve) => setTimeout(resolve, ms)),
  })
  if (options.now !== undefined)
    runInContext(`Date.now = () => ${options.now}`, realm)
  runInContext(strict(bootstrap()), realm, { filename: 'bootstrap.js' })
  if (options.withoutRefuse)
    runInContext(
      'globalThis.__neo = Object.freeze({ ...globalThis.__neo, refuse: undefined })',
      realm,
    )
  runInContext(strict(options.prelude ?? committedPrelude()), realm, {
    filename: 'prelude.js',
  })
  const parameters = String(runInContext('__neo.parameters()', realm))
  const start = runInContext('__neo.run', realm) as (
    main: (...provided: unknown[]) => Promise<unknown>,
    init: string,
  ) => Promise<string>
  const init = JSON.stringify({
    page: null,
    observed: { seq: 0, events: 0 },
    ...options.seed,
  })
  // The parameters arrive when the bootstrap calls the agent's function; see `ready`.
  let started: (provided: unknown[]) => void = () => {}
  const provided = new Promise<unknown[]>((resolve) => {
    started = resolve
  })
  void start(async (...args) => {
    startCalls.push(...calls.splice(0))
    started(args)
  }, init)
  let evaluator: ((code: string) => unknown) | null = null
  const ready = provided.then((args) => {
    evaluator = runInContext(
      strict(`(function (${parameters}) { return (code) => eval(code) })`),
      realm,
    )(...args) as (code: string) => unknown
  })
  const compile = (body: string) =>
    runInContext(
      strict(`(async function (${parameters}) {\n{\n${body}\n}\n})`),
      realm,
      {
        filename: 'run',
      },
    ) as (...provided: unknown[]) => Promise<unknown>
  return {
    calls,
    startCalls,
    aborts,
    ready,
    run: async (code) => compile(code)(...(await provided)),
    settle: async (code) => {
      // A second run in the realm builds its parameters again: what it sends before the body
      // starts is that run's start.
      const before = calls.length
      const main = compile(code)
      const settled = await start(async (...args) => {
        startCalls.push(...calls.splice(before))
        return main(...args)
      }, init)
      const { ok, thrown } = JSON.parse(settled) as {
        ok?: unknown
        thrown?: {
          kind: string
          operationId?: unknown
          name: string
          message: string
        }
      }
      if (thrown === undefined) return { ok }
      if (thrown.kind === 'operation')
        return { operation: failures.get(thrown.operationId) ?? null }
      return { error: { name: thrown.name, message: thrown.message } }
    },
    evaluate: (code) => {
      if (evaluator === null)
        throw new Error(
          "the run's parameters have not arrived; await runtime.ready",
        )
      return evaluator(code)
    },
    sent: (name) => calls.filter((call) => call.name === name),
  }
}

/** Latest event already delivered by the fake host when it takes its cache observation. */
function eventHead(name: string, value: unknown): number {
  const events =
    name === 'events.wait'
      ? [value]
      : name === 'events.next' && typeof value === 'object' && value !== null
        ? Reflect.get(value, 'events')
        : []
  if (!Array.isArray(events)) return 0
  let head = 0
  for (const event of events) {
    const seq =
      typeof event === 'object' && event !== null
        ? Reflect.get(event, 'seq')
        : undefined
    if (typeof seq === 'number') head = Math.max(head, seq)
  }
  return head
}

/**
 * The door's answer to a refusal brand (`__neo.refuse`, claw-sdk's dispatch/refusal_brand.rs):
 * refused under `name` before anything is sent, script-owned, its code one a script
 * may claim and any other `invalid_input`. Null for any other call. `operationId` names the
 * refusal and is drawn only for one, so the run's frames read at its start (FR-1) or the
 * script's other ops do not shift the ids a test expects.
 */
function refusalReply(
  name: string,
  input: unknown,
  operationId: () => string,
): { err: Record<string, unknown>; pages: [] } | null {
  if (typeof input !== 'object' || input === null) return null
  if (Reflect.get(input, '$neo') !== 'refused') return null
  const code: unknown = Reflect.get(input, 'code')
  const reason: unknown = Reflect.get(input, 'reason')
  const claimed =
    (code === 'method_unavailable' ||
      code === 'invalid_input' ||
      code === 'task_not_found' ||
      code === 'task_page_missing') &&
    typeof reason === 'string'
  return {
    err: {
      code: claimed ? code : 'invalid_input',
      owner: 'script',
      delivery: 'not_sent',
      retryable: true,
      operationId: operationId(),
      timeout: false,
      message: claimed
        ? reason
        : `\`${name}\`: a refusal may name only method_unavailable, invalid_input, task_not_found or task_page_missing`,
    },
    pages: [],
  }
}

/** Whether `call` is a refusal the prelude raised through `__neo.refuse`, not an op. */
export function isRefusal(call: Call): boolean {
  return call.input.$neo === 'refused'
}

/** A refusal as the door receives it: under `name`, with its code and the whole message. */
export function refusalCall(
  name: string,
  code: 'method_unavailable' | 'invalid_input',
  reason: string,
): Call {
  return { name, input: { $neo: 'refused', code, reason } }
}

/** A run that starts on page 7, tab 1007, as for a task whose tab the run service knows. */
export const SEED = { page: { $neo: 'page', tab: 1007 } }

/** JSON copy of a value from the run's realm, for assertions across realms. */
export function json(value: unknown): unknown {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value))
}
