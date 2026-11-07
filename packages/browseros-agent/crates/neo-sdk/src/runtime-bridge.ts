/**
 * The only file that touches `__neo_runtime`, the native object neo-runtime installs in the
 * realm (crates/neo-runtime/src/bridge.rs).
 *
 * - `get` blocks until Rust has read one value live from BrowserOS over CDP.
 * - `call` sends one Playwright protocol message and resolves with Rust's Playwright reply,
 *   failures included; it rejects only when Rust cannot read the envelope. The call runs on its
 *   own Rust task, so it keeps going while the script spins or blocks in a getter, and a call
 *   the script never awaits still finishes and leaves its trace record.
 */

import type {
  FrameInitializer,
  PageInitializer,
} from '@browseros/neo-runtime-spec/serve'
import type { NeoSerializedUrlMatch } from './neo-commands'

declare const __neo_runtime: {
  get(request: string): string
  call(message: string): Promise<string>
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
 * guids it handed the run (the run page, every `pages` record, every `newPage` result) and
 * throws `Error` for any other.
 */
export interface GetterReplies {
  /** `null` once the page is closed or the frame is detached. */
  url: string | null
  isClosed: boolean
  /** On the context: its open tabs, in the order Rust first saw them, the run page first. */
  pages: CreatedPage[]
  /** On a page: its creation record, which `newPage` builds its `Page` from. */
  initializer: CreatedPage
  /** On a page: its live frame tree in pre-order, main frame first. */
  frames: CreatedFrame[]
  /** On a frame: live values from its page's frame tree; detached metadata is null. */
  name: string | null
  parentFrame: CreatedFrame | null
  childFrames: CreatedFrame[]
  isDetached: boolean
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

/** One browser call: message JSON in, reply JSON out. Only `connection.ts` calls it. */
export function call(message: string): Promise<string> {
  return __neo_runtime.call(message)
}
