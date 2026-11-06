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

declare const __neo_runtime: {
  get(request: string): string
  call(message: string): Promise<string>
}

/** What each getter returns, keyed by the field name Rust dispatches on. */
export interface GetterReplies {
  /** `null` once the page is closed: every CDP source errors then. */
  url: string | null
  isClosed: boolean
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
