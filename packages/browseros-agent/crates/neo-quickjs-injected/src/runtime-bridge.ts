/**
 * The only file that touches `__neo_runtime`, the native object neo-runtime installs in the
 * realm (crates/neo-runtime/src/quickjs_bridge.rs).
 *
 * - `get` blocks until Rust has read one value live from BrowserOS over CDP.
 * - `callWithEvents` sends one Playwright protocol message and resolves with Rust's Playwright reply,
 *   failures included; it rejects only when Rust cannot read the envelope. The call runs on its
 *   own Rust task, so it keeps going while the script spins or blocks in a getter, and a call
 *   the script never awaits still finishes and leaves its trace record. Its reply carries the
 *   event watermark W: `call` runs the listeners of every event up to W (`events.ts`) before it
 *   returns the reply text, so they run before the caller continues, Playwright's order.
 * - `abort` aborts one call in flight, from its `signal`'s `abort` listener: the call stops and
 *   replies `AbortError`.
 * - `subscribe` turns the run's listening to one event of a page or the context on or off.
 * - `drain` takes queued events; `nextEvents` resolves once one is queued (the idle wake).
 * - `get` also serves the pure platform reads QuickJS lacks: URL parses and setters, device
 *   descriptors, random bytes.
 */

import type {
  FrameInitializer,
  PageInitializer,
} from '@browseros/neo-runtime-spec/serve'
import { Buffer } from './buffer'
import type { PausedDetails } from './debugger'
import type { RawEvent } from './events'
import type { CreatedHandle } from './js-handle'
import type { NeoSerializedUrlMatch } from './neo-commands'
import type { CreatedChain, Timing } from './network'

declare const __neo_runtime: {
  get(request: string): string
  callWithEvents(message: string): Promise<{ reply: string; events: number }>
  abort(id: number, reason: string): void
  subscribe(request: string): void
  drain(until: number): string
  nextEvents(): Promise<void>
}

/** What runs a reply's events before its caller continues: the run's `EventPipe.deliver`. */
let deliverEvents: ((until: number) => void) | undefined

/** Sets where `call` delivers each reply's events (`index.ts` `createRun`). */
export function onReplyEvents(deliver: (until: number) => void): void {
  deliverEvents = deliver
}

/**
 * A page Rust hands the run, as Playwright's `__create__` messages for a `Page` and its main
 * `Frame` would carry it. Rust mints both guids (`page@<targetId>`, `frame@<targetId>`); the SDK
 * never derives one from another. `mainFrame.initializer.url` is the tab's URL when Rust read
 * the record.
 */
export interface CreatedPage {
  guid: string
  initializer: PageInitializer
  mainFrame: CreatedFrame
}

/** A frame record minted by Rust. Guids remain opaque to the SDK. */
export interface CreatedFrame {
  guid: string
  initializer: FrameInitializer
}

/**
 * What each getter returns, keyed by the field name Rust dispatches on. Rust answers only for
 * guids it handed the run (the run page, every `pages` record, every `newPage` result, every
 * request and response it recorded) and throws `Error` for any other.
 */
export interface GetterReplies {
  /** `null` once the page is closed or the frame is detached. */
  url: string | null
  isClosed: boolean
  /** On the context: its open tabs, in the order Rust first saw them, the run page first. */
  pages: CreatedPage[]
  /** On a page: its creation record, which `newPage` builds its `Page` from. */
  initializer: CreatedPage
  /** On a request or response: its hop and every hop before it, oldest first. */
  chain: CreatedChain
  /** On a request: the guid of the request a redirect made of it, once it started. */
  redirectedTo: string | null
  /** On a request: the guid of its response, once it arrived. */
  existingResponse: string | null
  /** On a request: `timing()` now, `responseEnd` included once the request ended. */
  timing: Timing
  /** On a request: its failure text, `null` (or empty) while it has not failed. */
  failure: string | null
  /** On a response: whether its request finished, is still loading, or its page closed. */
  finished: 'finished' | 'pending' | 'closed'
  /** On a page: its live frame tree in pre-order, main frame first. */
  frames: CreatedFrame[]
  /** On a frame: live values from its page's frame tree; detached metadata is null. */
  name: string | null
  parentFrame: CreatedFrame | null
  childFrames: CreatedFrame[]
  isDetached: boolean
  /** On the browser: its version, as Playwright reports it (`155.0.8318.26`). */
  version: string
  /** On the browser: whether the runtime's connection to it is up. */
  isConnected: boolean
  /** On a debugger: the call it holds, `null` while none. */
  pausedDetails: PausedDetails | null
  /** On a context: its debugger's guid (`debugger@<context>`). */
  debugger: string
  /** On a page: the open tab of the run's context that opened it, `null` for none. */
  opener: CreatedPage | null
  /** On a console message (`console@<targetId>:<n>`): its arguments as the run's handles. */
  args: CreatedHandle[]
  /** On a file chooser (`filechooser@<targetId>:<n>`): its input as the run's handle. */
  element: CreatedHandle
}

export function get<Field extends keyof GetterReplies>(
  guid: string,
  field: Field,
): GetterReplies[Field] {
  return JSON.parse(__neo_runtime.get(JSON.stringify({ guid, field })))
}

/**
 * A pure synchronous getter over Rust's shared URL matcher. QuickJS has no WHATWG URL
 * parser, so compiling globs here would disagree with `waitForURL`'s URL normalization.
 */
export function matchesURL(
  url: string,
  matcher: NeoSerializedUrlMatch,
): boolean {
  return JSON.parse(
    __neo_runtime.get(JSON.stringify({ field: 'matchesURL', url, matcher })),
  )
}

/** The parts of a WHATWG URL, as Rust's parser reads them (`url_match.rs` `ParsedUrl`). */
export interface ParsedURL {
  href: string
  origin: string
  protocol: string
  username: string
  password: string
  host: string
  hostname: string
  port: string
  pathname: string
  search: string
  hash: string
  /** The query's `application/x-www-form-urlencoded` pairs, in order. */
  searchParams: [string, string][]
}

/**
 * A synchronous parse by Rust's WHATWG URL parser, which QuickJS lacks: `null` where
 * `new URL(url, base)` would throw.
 */
export function parseURL(url: string, base?: string): ParsedURL | null {
  return JSON.parse(
    __neo_runtime.get(JSON.stringify({ field: 'parseURL', url, base })),
  )
}

/** A WHATWG `URL` attribute with a setter (`url_match.rs` `UrlPart`). */
export type URLPart =
  | 'href'
  | 'protocol'
  | 'username'
  | 'password'
  | 'host'
  | 'hostname'
  | 'port'
  | 'pathname'
  | 'search'
  | 'hash'

/**
 * The URL `href` once Rust's WHATWG parser has run the `part` setter with `value`: unchanged
 * parts where the setter ignores `value`, `null` where it throws (only `href`'s does).
 */
export function setURL(
  href: string,
  part: URLPart,
  value: string,
): ParsedURL | null {
  return JSON.parse(
    __neo_runtime.get(JSON.stringify({ field: 'setURL', href, part, value })),
  )
}

/** `length` random bytes from Rust's OS-seeded CSPRNG, at most 65,536 (`quickjs_bridge.rs`). */
export function randomValues(length: number): Uint8Array {
  return Buffer.fromBinary(
    JSON.parse(
      __neo_runtime.get(JSON.stringify({ field: 'randomValues', length })),
    ),
  )
}

/**
 * Playwright's device descriptors, by name: the vendored data neo-runtime embeds
 * (`neo-quickjs-injected/vendor/playwright/isomorphic/deviceDescriptorsSource.json`), read once per
 * run.
 */
export function devices(): Record<string, unknown> {
  return JSON.parse(__neo_runtime.get(JSON.stringify({ field: 'devices' })))
}

/**
 * One browser call: message JSON in, reply JSON out. Only `connection.ts` calls it. The events
 * the reply's watermark covers run their listeners here, synchronously in the reaction to
 * Rust's answer, before the reply text reaches the caller.
 */
export async function call(message: string): Promise<string> {
  const { reply, events } = await __neo_runtime.callWithEvents(message)
  deliverEvents?.(events)
  return reply
}

/**
 * Aborts the call sent as message `id`, which stops at its next wait and replies `AbortError`
 * with `reason`; one that already replied is left alone. Only `connection.ts` calls it, from the
 * call's `signal` listener (Playwright's `__abort__ {id, reason}`).
 */
export function abort(id: number, reason: string): void {
  __neo_runtime.abort(id, reason)
}

/** Turns the run's listening to `event` on `guid` on or off; throws for one Neo cannot produce. */
export function subscribe(guid: string, event: string, enabled: boolean): void {
  __neo_runtime.subscribe(JSON.stringify({ guid, event, enabled }))
}

/** Takes every queued event with seq ≤ `until` (all of them for -1), in seq order. */
export function drain(until: number): RawEvent[] {
  return JSON.parse(__neo_runtime.drain(until))
}

/** Resolves once an undelivered event is queued. */
export function nextEvents(): Promise<void> {
  return __neo_runtime.nextEvents()
}
