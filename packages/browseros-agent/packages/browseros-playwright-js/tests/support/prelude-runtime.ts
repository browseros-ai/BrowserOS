// Runs the built prelude asset the way claw-sdk does: a fresh realm per run, claw-sdk's own
// bootstrap first (apps/claw-sdk/src/runtime/js/bootstrap.js, read as it is, so a drift
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

export type Call = { name: string; input: Record<string, unknown> }

/** An op's answer: a value, or an envelope (`{ ok }`, `{ err }`, with `pages`, `defaultPage`). */
type Envelope = {
  ok?: unknown
  err?: Record<string, unknown>
  pages?: unknown[]
  defaultPage?: { id: number; tab: number }
}
export type Answer = (call: Call) => unknown | Promise<unknown>

/** How a run ended, as claw-sdk's run_script.rs `settle` reports it. */
export type Settled =
  | { ok: unknown }
  /** An uncaught NeoError of an op: the failure the door answered that op with, or null. */
  | { operation: Record<string, unknown> | null }
  /** Anything else the script threw: claw-sdk's `script_error`. */
  | { error: { name: string; message: string } }

const agentRoot = new URL('../../../../', import.meta.url)

/** The committed prelude asset, read when a runtime is built without prelude text. */
const committedPrelude = (): string =>
  readFileSync(
    new URL('crates/browseros-playwright/assets/prelude.js', agentRoot),
    'utf8',
  )

const bootstrap = (): string =>
  readFileSync(
    new URL('apps/claw-sdk/src/runtime/js/bootstrap.js', agentRoot),
    'utf8',
  )

/** claw-sdk evaluates the bootstrap, the preludes and the agent's function as strict code. */
const strict = (source: string): string => `'use strict';${source}`

export type Runtime = {
  /** Every op the prelude sent, decoded from the JSON the bridge carried. */
  calls: Call[]
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
}

export function loadPrelude(
  options: {
    answer?: Answer
    /**
     * The run's init (run_script.rs): `{ page }`, the run's page brand, and `pages`, every tab's
     * facts, once the run service sends them. Absent: no tab yet.
     */
    seed?: { page: unknown; pages?: unknown[] }
    /** The prelude's source; the committed asset when absent (build.ts passes a fresh build). */
    prelude?: string
    /** A fixed `Date.now()` for the realm, so recorded `since` stamps are reproducible. */
    now?: number
  } = {},
): Runtime {
  const calls: Call[] = []
  // Each failed op's error, by op id, as bridge.rs remembers the failures it answered.
  const failures = new Map<unknown, Record<string, unknown>>()
  const answer = options.answer ?? (() => null)
  const realm = createContext({
    __neo_invoke: async (name: string, inputJson: string): Promise<string> => {
      const call: Call = { name, input: JSON.parse(inputJson) }
      calls.push(call)
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
      return JSON.stringify({ pages: [], ...envelope, ok: envelope.ok ?? null })
    },
    __neo_log: () => {},
    __neo_sleep: (ms: number) =>
      new Promise((resolve) => setTimeout(resolve, ms)),
  })
  if (options.now !== undefined)
    runInContext(`Date.now = () => ${options.now}`, realm)
  runInContext(strict(bootstrap()), realm, { filename: 'bootstrap.js' })
  runInContext(strict(options.prelude ?? committedPrelude()), realm, {
    filename: 'prelude.js',
  })
  const parameters = String(runInContext('__neo.parameters()', realm))
  const start = runInContext('__neo.run', realm) as (
    main: (...provided: unknown[]) => Promise<unknown>,
    init: string,
  ) => Promise<string>
  const init = JSON.stringify(options.seed ?? { page: null })
  // The parameters arrive when the bootstrap calls the agent's function; see `ready`.
  let started: (provided: unknown[]) => void = () => {}
  const provided = new Promise<unknown[]>((resolve) => {
    started = resolve
  })
  void start(async (...args) => {
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
      strict(`(async function (${parameters}) {\n${body}\n})`),
      realm,
      {
        filename: 'run',
      },
    ) as (...provided: unknown[]) => Promise<unknown>
  return {
    calls,
    ready,
    run: async (code) => compile(code)(...(await provided)),
    settle: async (code) => {
      const { ok, thrown } = JSON.parse(await start(compile(code), init)) as {
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

/** A run that starts on page 7, tab 1007, as for a task whose tab the run service knows. */
export const SEED = { page: { $neo: 'page', id: 7, tab: 1007 } }

/** JSON copy of a value from the run's realm, for assertions across realms. */
export function json(value: unknown): unknown {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value))
}
