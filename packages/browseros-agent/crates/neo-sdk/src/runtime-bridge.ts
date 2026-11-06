/**
 * The only file that touches `__neo_runtime`, the native object neo-runtime installs in the
 * realm (crates/neo-runtime/src/bridge.rs). Each getter is one synchronous call that blocks
 * until Rust has read the value live from BrowserOS over CDP.
 */

declare const __neo_runtime: {
  get(request: string): string
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
