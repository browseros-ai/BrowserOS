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
import type { CreatedChain, Timing } from './network'

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
  mainFrame: { guid: string; initializer: FrameInitializer }
}

/**
 * What each getter returns, keyed by the field name Rust dispatches on. Rust answers only for
 * guids it handed the run (the run page, every `pages` record, every `newPage` result, every
 * request and response of a `goto` answer) and throws `Error` for any other.
 */
export interface GetterReplies {
  /** `null` once the page is closed: every CDP source errors then. */
  url: string | null
  isClosed: boolean
  /** On the context: its open tabs, in the order Rust first saw them, the run page first. */
  pages: CreatedPage[]
  /** On a page: its creation record, which `newPage` builds its `Page` from. */
  initializer: CreatedPage
  /** On a response: its request and every hop before it, oldest first. */
  chain: CreatedChain
  /** On a request: `timing()` now, `responseEnd` included once the request ended. */
  timing: Timing
  /** On a request: its failure text, `null` (or empty) while it has not failed. */
  failure: string | null
  /** On a response: whether its request finished, is still loading, or its page closed. */
  finished: 'finished' | 'pending' | 'closed'
}

export function get<Field extends keyof GetterReplies>(
  guid: string,
  field: Field,
): GetterReplies[Field] {
  return JSON.parse(__neo_runtime.get(JSON.stringify({ guid, field })))
}

/** One browser call: message JSON in, reply JSON out. Only `connection.ts` calls it. */
export function call(message: string): Promise<string> {
  return __neo_runtime.call(message)
}
