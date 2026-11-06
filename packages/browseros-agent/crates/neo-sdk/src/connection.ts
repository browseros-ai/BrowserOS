/**
 * The client half of Playwright's wire inside the realm (Playwright's `client/connection.ts`
 * and `channelOwner.ts` `_wrapApiCall` at 26137face, cut to what a run needs). Every browser
 * call goes through {@link Connection.send}: it captures the caller's location synchronously,
 * builds Playwright's `{id, guid, method, params, metadata}`, hands it to `runtime-bridge.ts`,
 * and turns the reply into a value or a Playwright-shaped error.
 *
 * ```text
 *  locator.click() → frame._click('locator.click', …) → send(): location, message
 *    → __neo_runtime.call(json)  ── Rust: dispatch.rs serves it on its own task ──▶
 *    ◀ '{"id":1}'  |  '{"error":{"error":{"message","name"}},"id":1,"log":[…]}'
 *    → undefined   |  throw rebuildError(apiName, error, log)
 * ```
 *
 * Each public method passes its `apiName` (the `locator.click: ` prefix of an error)
 * explicitly. Playwright derives it from V8 stack frames, which name the class; QuickJS frames
 * name only the function. It never goes on the wire: Playwright's `Metadata` has no such field.
 */

import type { FrameCommands, Metadata } from '@browseros/neo-runtime-spec/serve'
import { type PlaywrightError, parseError } from './errors'
import { call } from './runtime-bridge'

/** The public methods that send a browser call, named as Playwright's client names them. */
export type ApiName = 'locator.click' | 'page.click' | 'frame.click'

/** Playwright's wire error, `{error: {name, message}}`'s inner object. */
export type WireError = { name: string; message: string }

/** Playwright's reply. Rust writes it (`dispatch.rs` `encode_reply`), and
 * `neo-runtime/tests/fixtures/replies.json` pins the shape for both sides. */
type Reply = {
  id: number
  result?: unknown
  error?: { error: WireError }
  log?: string[]
}

/** The file every location names: the agent's script is `run.js` everywhere. */
const SCRIPT_FILE = 'run.js'

/**
 * A QuickJS stack frame in the agent's script. Only code built by the realm's `AsyncFunction`
 * constructor is named `<input>`; this bundle and the realm's host globals are `eval_script`
 * (probe P4, pinned in `neo-runtime/tests/realm_limits.rs`). Columns are 1-based, as in V8.
 */
const AGENT_FRAME = /^ {4}at .* \(<input>:(\d+):(\d+)\)$/

export class Connection {
  #nextId = 1
  readonly #lineOffset: number

  /** `lineOffset`: lines the realm's `AsyncFunction` wrapper puts before the agent's line 1. */
  constructor(lineOffset: number) {
    this.#lineOffset = lineOffset
  }

  /**
   * Sends one browser call and resolves with its result, `undefined` for a void method such
   * as `click`, whose reply has no `result`. `timeout` is the resolved action timeout in ms,
   * `0` for none; it travels in `metadata`, never in params.
   *
   * The location is captured before the first `await`: QuickJS keeps no caller frames across
   * an `await`, and every public method reaches this line synchronously, as Playwright
   * captures its stack before its first `await` (`channelOwner.ts` `_wrapApiCall`).
   */
  async send<M extends keyof FrameCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: FrameCommands[M]['params'],
    timeout: number,
  ): Promise<FrameCommands[M]['result'] | undefined> {
    const metadata: Metadata = {
      location: captureLocation(this.#lineOffset),
      timeout,
    }
    const message = { id: this.#nextId++, guid, method, params, metadata }
    const reply: Reply = JSON.parse(await call(JSON.stringify(message)))
    if (reply.error) throw rebuildError(apiName, reply.error.error, reply.log)
    return reply.result as FrameCommands[M]['result'] | undefined
  }
}

/**
 * `metadata.location` of the innermost agent frame on the current stack. Raises
 * `Error.stackTraceLimit` around the capture, as Playwright's `captureRawStack` does, because
 * QuickJS keeps 10 frames by default and an agent may have lowered it.
 */
function captureLocation(lineOffset: number): Metadata['location'] {
  const stackTraceLimit = Error.stackTraceLimit
  Error.stackTraceLimit = 50
  const stack = new Error().stack ?? ''
  Error.stackTraceLimit = stackTraceLimit
  return agentLocation(stack, lineOffset)
}

/** The first agent frame in QuickJS stack text, with `lineOffset` subtracted, or just the file
 * when no frame is the agent's (`line` and `column` are optional in Playwright's `Metadata`). */
export function agentLocation(
  stack: string,
  lineOffset: number,
): NonNullable<Metadata['location']> {
  for (const frame of stack.split('\n')) {
    const match = AGENT_FRAME.exec(frame)
    if (match)
      return {
        file: SCRIPT_FILE,
        line: Number(match[1]) - lineOffset,
        column: Number(match[2]),
      }
  }
  return { file: SCRIPT_FILE }
}

/**
 * Playwright's client error rebuild (`connection.ts` `dispatch` and `channelOwner.ts`
 * `_wrapApiCall`): the class by wire name, `e.log` set to the call log, the formatted call log
 * appended to the message, then the `<apiName>: ` prefix.
 */
export function rebuildError(
  apiName: ApiName,
  error: WireError,
  log: string[] | undefined,
): PlaywrightError {
  const rebuilt = parseError(error)
  rebuilt.log = log || []
  rebuilt.message = `${apiName}: ${rebuilt.message}${formatCallLog(log)}`
  return rebuilt
}

/** Playwright's `formatCallLog` with colours off, as the goldens recorded it. */
export function formatCallLog(log: string[] | undefined): string {
  if (!log?.some((line) => !!line)) return ''
  return `\nCall log:\n${log.join('\n')}\n`
}
