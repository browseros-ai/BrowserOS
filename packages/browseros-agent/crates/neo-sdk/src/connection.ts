/**
 * The client half of Playwright's wire inside the realm (Playwright's `client/connection.ts`
 * and `channelOwner.ts` `_wrapApiCall` at 26137face, cut to what a run needs). Every browser
 * call goes through a `Connection` method typed by the interface its guid names (`send` for a
 * `Frame`, `sendPage`, `sendContext`, `sendRequest`, `sendResponse`, and `sendNeo` for Neo's own
 * `neo:` frame commands, `neo-commands.ts`): it captures the caller's location synchronously,
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

import type {
  BrowserContextCommands,
  DialogCommands,
  FrameCommands,
  Metadata,
  PageCommands,
  RequestCommands,
  ResponseCommands,
} from '@browseros/neo-runtime-spec/serve'
import { type PlaywrightError, parseError } from './errors'
import type { EvalFrameCommands, JSHandleCommands } from './js-handle'
import type { NeoFrameCommands } from './neo-commands'
import type { Request, Response } from './network'
import { call } from './runtime-bridge'

/** The public methods that send a browser call, named as Playwright's client names them. */
export type ApiName =
  | 'page.frame'
  | 'locator.click'
  | 'locator.dblclick'
  | 'locator.hover'
  | 'locator.tap'
  | 'locator.fill'
  | 'locator.press'
  | 'locator.inputValue'
  | 'locator.isChecked'
  | 'locator.isVisible'
  | 'locator.check'
  | 'locator.uncheck'
  | 'locator.setChecked'
  | 'locator.selectOption'
  | 'locator.clear'
  | 'page.click'
  | 'page.dblclick'
  | 'page.hover'
  | 'page.tap'
  | 'page.fill'
  | 'page.press'
  | 'page.inputValue'
  | 'page.isChecked'
  | 'page.isVisible'
  | 'page.check'
  | 'page.uncheck'
  | 'page.setChecked'
  | 'page.selectOption'
  | 'frame.click'
  | 'frame.dblclick'
  | 'frame.hover'
  | 'frame.tap'
  | 'page.goto'
  | 'frame.goto'
  | 'page.title'
  | 'frame.title'
  | 'page.textContent'
  | 'frame.textContent'
  | 'locator.textContent'
  | 'page.innerText'
  | 'frame.innerText'
  | 'locator.innerText'
  | 'page.getAttribute'
  | 'frame.getAttribute'
  | 'locator.getAttribute'
  | 'locator.count'
  | 'locator.allTextContents'
  | 'page.ariaSnapshot'
  | 'locator.ariaSnapshot'
  | 'frame.fill'
  | 'frame.press'
  | 'frame.inputValue'
  | 'frame.isChecked'
  | 'frame.isVisible'
  | 'frame.check'
  | 'frame.uncheck'
  | 'frame.setChecked'
  | 'frame.selectOption'
  | 'browserContext.newPage'
  | 'page.close'
  | 'page.bringToFront'
  | 'locator.waitFor'
  | 'page.waitForURL'
  | 'frame.waitForURL'
  | 'page.waitForLoadState'
  | 'frame.waitForLoadState'
  | 'request.response'
  | 'request.allHeaders'
  | 'request.headersArray'
  | 'request.headerValue'
  | 'request.sizes'
  | 'response.allHeaders'
  | 'response.headersArray'
  | 'response.headerValue'
  | 'response.headerValues'
  | 'response.body'
  | 'response.text'
  | 'response.json'
  | 'response.serverAddr'
  | 'response.securityDetails'
  | 'response.httpVersion'
  | 'page.evaluate'
  | 'page.evaluateHandle'
  | 'frame.evaluate'
  | 'frame.evaluateHandle'
  | 'locator.evaluate'
  | 'jsHandle.evaluate'
  | 'jsHandle.evaluateHandle'
  | 'jsHandle.jsonValue'
  | 'jsHandle.dispose'
  | 'elementHandle.evaluate'
  | 'elementHandle.evaluateHandle'
  | 'elementHandle.jsonValue'
  | 'elementHandle.dispose'
  | 'jsHandle.getProperty'
  | 'jsHandle.getProperties'
  | 'elementHandle.getProperty'
  | 'elementHandle.getProperties'
  | 'elementHandle.$'
  | 'elementHandle.$$'
  | 'elementHandle.$eval'
  | 'elementHandle.$$eval'
  | 'elementHandle.dispatchEvent'
  | 'page.addScriptTag'
  | 'page.addStyleTag'
  | 'frame.addScriptTag'
  | 'frame.addStyleTag'
  | 'page.$'
  | 'page.$$'
  | 'page.$eval'
  | 'page.$$eval'
  | 'frame.$'
  | 'frame.$$'
  | 'frame.$eval'
  | 'frame.$$eval'
  | 'page.waitForFunction'
  | 'frame.waitForFunction'
  | 'locator.evaluateHandle'
  | 'locator.evaluateAll'
  | 'locator.elementHandle'
  | 'locator.elementHandles'
  | 'page.waitForEvent'
  | 'browserContext.waitForEvent'
  | 'page.consoleMessages'
  | 'page.pageErrors'
  | 'page.clearConsoleMessages'
  | 'page.clearPageErrors'
  | 'dialog.accept'
  | 'dialog.dismiss'
  // Element reads and states (B1).
  | 'locator.isEnabled'
  | 'page.isEnabled'
  | 'frame.isEnabled'
  | 'locator.isDisabled'
  | 'page.isDisabled'
  | 'frame.isDisabled'
  | 'locator.isEditable'
  | 'page.isEditable'
  | 'frame.isEditable'
  | 'locator.isHidden'
  | 'page.isHidden'
  | 'frame.isHidden'
  | 'locator.innerHTML'
  | 'page.innerHTML'
  | 'frame.innerHTML'
  | 'locator.allInnerTexts'
  | 'locator.all'

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

/** What a call resolves with: the command's result, or `undefined` for a void command, whose
 * reply carries no `result`. */
export type Resolved<Result> =
  Result extends Record<string, never> ? undefined : Result

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
  /** Every `Request` and `Response` built so far, by guid, as Playwright's connection keeps its
   * objects: one guid is always one object (`network.ts` `responseFor`). */
  readonly _objects = new Map<string, Request | Response>()

  /** `lineOffset`: lines the realm's `AsyncFunction` wrapper puts before the agent's line 1. */
  constructor(lineOffset: number) {
    this.#lineOffset = lineOffset
  }

  /**
   * Sends one `Frame` call and resolves with its result, `undefined` for a void method such
   * as `click`, whose reply has no `result`. `timeout` is the resolved action timeout in ms,
   * `0` for none; it travels in `metadata`, never in params.
   */
  async send<M extends keyof FrameCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: FrameCommands[M]['params'],
    timeout: number,
  ): Promise<FrameCommands[M]['result'] | undefined> {
    return (await this.#send(apiName, guid, method, params, timeout)) as
      | FrameCommands[M]['result']
      | undefined
  }

  /** {@link Connection.send} for a `Page` command, to the page's `page@<targetId>` guid. */
  async sendPage<M extends keyof PageCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: PageCommands[M]['params'],
    timeout: number,
  ): Promise<Resolved<PageCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<PageCommands[M]['result']>
  }

  /** {@link Connection.send} for a `BrowserContext` command, to the context's guid. */
  async sendContext<M extends keyof BrowserContextCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: BrowserContextCommands[M]['params'],
    timeout: number,
  ): Promise<Resolved<BrowserContextCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<BrowserContextCommands[M]['result']>
  }

  /** {@link Connection.send} for a `Request` command, to a hop's `request@…` guid. */
  async sendRequest<M extends keyof RequestCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: RequestCommands[M]['params'],
    timeout: number,
  ): Promise<RequestCommands[M]['result']> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as RequestCommands[M]['result']
  }

  /** {@link Connection.send} for a `Response` command, to a hop's `response@…` guid. */
  async sendResponse<M extends keyof ResponseCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: ResponseCommands[M]['params'],
    timeout: number,
  ): Promise<ResponseCommands[M]['result']> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as ResponseCommands[M]['result']
  }

  /** {@link Connection.send} for a `Dialog` command, to a `dialog@…` guid. */
  async sendDialog<M extends keyof DialogCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: DialogCommands[M]['params'],
    timeout: number,
  ): Promise<Resolved<DialogCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<DialogCommands[M]['result']>
  }

  /**
   * {@link Connection.send} for one of Neo's own commands on a frame guid (`neo-commands.ts`):
   * not Playwright protocol, but the same message, reply and error rebuild. `timeout` bounds the
   * whole wait Rust runs for it.
   */
  async sendNeo<M extends keyof NeoFrameCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: NeoFrameCommands[M]['params'],
    timeout: number,
  ): Promise<Resolved<NeoFrameCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<NeoFrameCommands[M]['result']>
  }

  /** {@link Connection.send} for an evaluation on a frame guid (`js-handle.ts`): Playwright's
   * `Frame.evaluateExpression(Handle)`, whose `arg` is already in the page's format. */
  async sendEval<M extends keyof EvalFrameCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: EvalFrameCommands[M]['params'],
    timeout: number,
  ): Promise<EvalFrameCommands[M]['result']> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as EvalFrameCommands[M]['result']
  }

  /** {@link Connection.send} for a `JSHandle` command, to a handle's `handle@<n>` guid. A void
   * command (`dispose`) resolves `undefined`. */
  async sendHandle<M extends keyof JSHandleCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: JSHandleCommands[M]['params'],
    timeout: number,
  ): Promise<JSHandleCommands[M]['result']> {
    return ((await this.#send(apiName, guid, method, params, timeout)) ??
      {}) as JSHandleCommands[M]['result']
  }

  /**
   * The message, the reply and the error rebuild every `send*` shares.
   *
   * The location is captured before the first `await`: QuickJS keeps no caller frames across
   * an `await`, and every public method reaches this line synchronously, as Playwright
   * captures its stack before its first `await` (`channelOwner.ts` `_wrapApiCall`).
   */
  async #send(
    apiName: ApiName,
    guid: string,
    method: string,
    params: unknown,
    timeout: number,
  ): Promise<unknown> {
    // Playwright's server validates every call's metadata (`dispatcher.ts`, `tOptional(tFloat)`):
    // `setDefaultTimeout(null)` resolves a timeout to `null`, which rejects the call before it
    // runs. Rust's serde would read it as no timeout and run the call.
    if (typeof timeout !== 'number')
      throw new Error(
        `timeout: expected float, got ${timeout === null ? 'object' : typeof timeout}`,
      )
    const metadata: Metadata = {
      location: captureLocation(this.#lineOffset),
      timeout,
    }
    const message = { id: this.#nextId++, guid, method, params, metadata }
    const reply: Reply = JSON.parse(await call(JSON.stringify(message)))
    if (reply.error) throw rebuildError(apiName, reply.error.error, reply.log)
    return reply.result
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

/** Playwright's `formatCallLog` with colours off. */
export function formatCallLog(log: string[] | undefined): string {
  if (!log?.some((line) => !!line)) return ''
  return `\nCall log:\n${log.join('\n')}\n`
}
