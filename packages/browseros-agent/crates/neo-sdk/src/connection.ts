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
 *  signal aborts meanwhile → __neo_runtime.abort(1, reason) → the reply is AbortError(reason)
 * ```
 *
 * Each public method passes its `apiName` (the `locator.click: ` prefix of an error)
 * explicitly. Playwright derives it from V8 stack frames, which name the class; QuickJS frames
 * name only the function. It never goes on the wire: Playwright's `Metadata` has no such field.
 */

import type {
  ArtifactCommands,
  BrowserContextCommands,
  DialogCommands,
  FrameCommands,
  Metadata,
  PageCommands,
  RequestCommands,
  ResponseCommands,
} from '@browseros/neo-runtime-spec/serve'
import {
  AbortError,
  isTargetClosedError,
  type PlaywrightError,
  parseError,
  targetClosedError,
} from './errors'
import type { EvalFrameCommands, JSHandleCommands } from './js-handle'
import type { NeoFrameCommands } from './neo-commands'
import type { Request, Response } from './network'
import { abort, call } from './runtime-bridge'
import type { SignalLike, TimeoutOptions } from './timeout-settings'

/** The public methods that send a browser call, named as Playwright's client names them. */
export type ApiName =
  | 'page.frame'
  | 'keyboard.down'
  | 'keyboard.up'
  | 'keyboard.insertText'
  | 'keyboard.type'
  | 'keyboard.press'
  | 'mouse.move'
  | 'mouse.down'
  | 'mouse.up'
  | 'mouse.click'
  | 'mouse.dblclick'
  | 'mouse.wheel'
  | 'touchscreen.tap'
  | 'locator.click'
  | 'locator.dblclick'
  | 'locator.hover'
  | 'locator.tap'
  | 'locator.fill'
  | 'locator.press'
  | 'locator.pressSequentially'
  | 'locator.dragTo'
  | 'locator.type'
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
  | 'page.type'
  | 'page.dragAndDrop'
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
  // Document content and timing (B3).
  | 'frame.content'
  | 'page.content'
  | 'frame.setContent'
  | 'page.setContent'
  | 'frame.waitForTimeout'
  | 'page.waitForTimeout'
  | 'locator.normalize'
  | 'locator.ariaSnapshotJSON'
  | 'page.ariaSnapshotJSON'
  | 'frame.fill'
  | 'frame.press'
  | 'frame.type'
  | 'frame.dragAndDrop'
  | 'frame.inputValue'
  | 'frame.isChecked'
  | 'frame.isVisible'
  | 'frame.check'
  | 'frame.uncheck'
  | 'frame.setChecked'
  | 'frame.selectOption'
  | 'browserContext.newPage'
  | 'browserContext.close'
  | 'page.close'
  | 'page.bringToFront'
  | 'page.addLocatorHandler'
  | 'page.waitForSelector'
  | 'frame.waitForSelector'
  | 'frame.frameElement'
  | 'page.removeLocatorHandler'
  | 'locator.waitFor'
  | 'page.waitForURL'
  | 'frame.waitForURL'
  | 'page.waitForLoadState'
  | 'frame.waitForLoadState'
  | 'page.waitForNavigation'
  | 'frame.waitForNavigation'
  | 'page.reload'
  | 'page.goBack'
  | 'page.goForward'
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
  | 'page.screenshot'
  | 'page.pdf'
  | 'locator.screenshot'
  | 'elementHandle.screenshot'
  | 'page.waitForEvent'
  | 'browserContext.waitForEvent'
  | 'page.consoleMessages'
  | 'page.waitForRequest'
  | 'page.waitForResponse'
  | 'page.requests'
  | 'page.pageErrors'
  | 'page.clearConsoleMessages'
  | 'page.clearPageErrors'
  | 'dialog.accept'
  | 'dialog.dismiss'
  | 'download.path'
  | 'download.saveAs'
  | 'download.failure'
  | 'download.cancel'
  | 'download.delete'
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
  // Element operations (B2).
  | 'locator.focus'
  | 'page.focus'
  | 'frame.focus'
  | 'locator.blur'
  | 'locator.dispatchEvent'
  | 'page.dispatchEvent'
  | 'frame.dispatchEvent'
  | 'locator.scrollIntoViewIfNeeded'
  | 'locator.selectText'
  | 'locator.boundingBox'
  | 'locator.highlight'
  | 'locator.hideHighlight'
  | 'page.hideHighlight'

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
  /** Set once the run's context closed (`BrowserContext.close`): every later call fails as
   * closed before it reaches Rust. */
  #closed = false
  /** What rejects each call still waiting for its reply, with the context's close reason. */
  readonly #inFlight = new Set<(reason: string | undefined) => void>()
  /** The close reason a call's page gave (`page.close({ reason })`), by the call's guid, and
   * whether that page already reads closed: only the calls its close interrupted get the reason. */
  _closeReasonFor: (guid: string) => string | undefined = () => undefined
  _closedFor: (guid: string) => boolean = () => false
  /** Every `Request` and `Response` built so far, by guid, as Playwright's connection keeps its
   * objects: one guid is always one object (`network.ts` `responseFor`). */
  readonly _objects = new Map<string, Request | Response>()

  /** `lineOffset`: lines the realm's `AsyncFunction` wrapper puts before the agent's line 1. */
  constructor(lineOffset: number) {
    this.#lineOffset = lineOffset
  }

  /**
   * Sends one `Frame` call and resolves with its result, `undefined` for a void method such
   * as `click`, whose reply has no `result`. `timeout` is what `TimeoutSettings` resolved, the
   * timeout in ms (`0` for none) and the `signal` that aborts the call, or a bare number for a
   * call Playwright sends without a signal (`kNoTimeout`). Neither travels in params.
   */
  async send<M extends keyof FrameCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: FrameCommands[M]['params'],
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
  ): Promise<Resolved<DialogCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<DialogCommands[M]['result']>
  }

  /** {@link Connection.send} for an `Artifact` command, to a `download@…` guid. */
  async sendArtifact<M extends keyof ArtifactCommands>(
    apiName: ApiName,
    guid: string,
    method: M,
    params: ArtifactCommands[M]['params'],
    timeout: number,
  ): Promise<Resolved<ArtifactCommands[M]['result']>> {
    return (await this.#send(
      apiName,
      guid,
      method,
      params,
      timeout,
    )) as Resolved<ArtifactCommands[M]['result']>
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    timeout: number | TimeoutOptions,
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
    options: number | TimeoutOptions,
  ): Promise<unknown> {
    const { timeout, signal } =
      typeof options === 'number'
        ? { timeout: options, signal: undefined }
        : options
    if (this.#closed) throw closedError(apiName)
    // Playwright's client checks before it sends anything, so the browser sees nothing.
    if (signal?.aborted)
      throw prefixed(
        apiName,
        new AbortError(undefined, { cause: signal.reason }),
      )
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
    const closedBefore = this._closedFor(guid)
    const id = this.#nextId++
    const message = { id, guid, method, params, metadata }
    const pending = new Promise<string>((resolve, reject) => {
      const abandon = (reason: string | undefined) =>
        reject(closedError(apiName, reason))
      this.#inFlight.add(abandon)
      call(JSON.stringify(message))
        .then(resolve, reject)
        .finally(() => this.#inFlight.delete(abandon))
    })
    // Rust has registered the call by the time `call` returns, so an abort from now on reaches it.
    let listener: (() => void) | undefined
    try {
      listener = signal ? listenForAbort(apiName, id, signal) : undefined
    } catch (error) {
      pending.catch(() => {})
      throw error
    }
    let text: string
    try {
      text = await pending
    } finally {
      if (signal && listener) signal.removeEventListener('abort', listener)
    }
    const reply: Reply = JSON.parse(text)
    if (reply.error) {
      const error = rebuildError(apiName, reply.error.error, reply.log, signal)
      // As in Playwright, whose server closes a page's open calls with its close reason, the
      // reason a page's close gave reaches the calls it interrupted.
      const reason =
        isTargetClosedError(error) && !closedBefore
          ? this._closeReasonFor(guid)
          : undefined
      if (reason)
        error.message = `${apiName}: ${reason}${formatCallLog(reply.log)}`
      throw error
    }
    return reply.result
  }

  /**
   * The run's context closed: calls still waiting for a reply reject at once, with `reason`
   * when the close gave one, and every later call fails as closed. A call Rust was serving
   * finishes there on its own; its record in the run's outcome says what it did.
   */
  _close(reason: string | undefined): void {
    this.#closed = true
    for (const abandon of this.#inFlight) abandon(reason)
    this.#inFlight.clear()
  }
}

/** Playwright's `TargetClosedError` for a call on a closed context, `<apiName>: ` first. */
function closedError(apiName: ApiName, reason?: string): PlaywrightError {
  const error = targetClosedError(reason)
  error.message = `${apiName}: ${error.message}`
  return error
}

/**
 * Playwright's `sendMessageToServer` abort listener, added after the call is sent: once `signal`
 * aborts, Rust aborts the call (`__abort__ {id, reason}`). A reason that is an `Error` sends its
 * message, anything else its `String(…)`. A `signal` without `addEventListener` throws its
 * `TypeError` here, after the call went out, as Playwright's does; the call still runs.
 */
function listenForAbort(
  apiName: ApiName,
  id: number,
  signal: SignalLike,
): () => void {
  const listener = () => {
    const reason =
      signal.reason instanceof Error
        ? signal.reason.message
        : String(signal.reason)
    abort(id, reason)
  }
  try {
    signal.addEventListener('abort', listener, { once: true })
  } catch (error) {
    throw prefixed(apiName, error)
  }
  return listener
}

/** Playwright's `_wrapApiCall` prefix on an error raised inside the call. */
function prefixed(apiName: ApiName, error: unknown): unknown {
  if (error instanceof Error) error.message = `${apiName}: ${error.message}`
  return error
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
  signal?: SignalLike | null,
): PlaywrightError {
  const rebuilt = parseError(error)
  // An aborted call's error carries what the script aborted with, as Playwright's `dispatch` sets.
  if (signal?.aborted && rebuilt instanceof AbortError)
    rebuilt.cause = signal.reason
  rebuilt.log = log || []
  rebuilt.message = `${apiName}: ${rebuilt.message}${formatCallLog(log)}`
  return rebuilt
}

/** Playwright's `formatCallLog` with colours off. */
export function formatCallLog(log: string[] | undefined): string {
  if (!log?.some((line) => !!line)) return ''
  return `\nCall log:\n${log.join('\n')}\n`
}
