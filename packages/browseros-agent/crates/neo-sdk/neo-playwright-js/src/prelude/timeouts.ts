/**
 * Playwright's timeout chain, resolved in the prelude so Rust holds no per-page timeout state:
 * every op carries the result as `call.timeoutMs`.
 *
 * Order, as playwright-core's client `TimeoutSettings`: the call's `timeout` option, then the
 * page's `setDefaultTimeout` / `setDefaultNavigationTimeout`, then the context's, then the
 * Playwright defaults: 30 s for actions, reads, snapshots, event waits and navigation;
 * 5 s for `expect`, which has its own chain in expect.ts. Navigation falls back to a plain
 * default before the parent's, as upstream: `page.setDefaultTimeout(10_000)` also bounds that
 * page's `goto`. 0 means no timeout.
 */
import { clientError } from './errors'

export const DEFAULT_TIMEOUT = 30_000
export const DEFAULT_EXPECT_TIMEOUT = 5_000
const DEFAULT_NAVIGATION_TIMEOUT = 30_000

export type TimeoutOption = { timeout?: number }

/**
 * A timeout the prelude resolved for one op. `builtIn` only when nothing chose it: no timeout on
 * the call, none set on the page, none on the context, so Playwright's own 30 s applies.
 */
export type Resolved = { timeout: number; builtIn?: boolean }

/** A whole number of milliseconds, as the header carries; anything else is refused. */
export function timeoutMs(api: string, timeout: number): number {
  if (!Number.isFinite(timeout) || timeout < 0)
    throw clientError(
      api,
      `timeout: expected a non-negative number, got ${timeout}`,
    )
  return Math.ceil(timeout)
}

/**
 * The header's timeout part, built in this one place so no op carries the milliseconds without
 * the stamp: `timeoutMs`, and `timeoutBuiltIn` only when the prelude found no timeout on the
 * call, the page or the context (covered-click B stops a pointer action early only under it).
 * A default that anyone but Neo's own prelude set is a choice, a settings key included if Neo
 * ever gets one. The stamp is never derived from the number: a chosen 30000 carries none.
 */
export function headerTimeout(
  api: string,
  resolved: number | Resolved,
): { timeoutMs: number; timeoutBuiltIn?: true } {
  if (typeof resolved === 'number')
    return { timeoutMs: timeoutMs(api, resolved) }
  return {
    timeoutMs: timeoutMs(api, resolved.timeout),
    ...(resolved.builtIn ? { timeoutBuiltIn: true } : {}),
  }
}

export class TimeoutSettings {
  private readonly parent: TimeoutSettings | undefined
  private defaultTimeout: number | undefined
  private defaultNavigationTimeout: number | undefined

  constructor(parent?: TimeoutSettings) {
    this.parent = parent
  }

  setDefaultTimeout(timeout: number | undefined): void {
    this.defaultTimeout = timeout
  }

  setDefaultNavigationTimeout(timeout: number | undefined): void {
    this.defaultNavigationTimeout = timeout
  }

  /** Actions, reads that wait, snapshots and event waits. */
  timeout(options: TimeoutOption | undefined): number {
    return this.resolved(options).timeout
  }

  /** `timeout`, with whether nothing chose it: the call, the page and the context set none. */
  resolved(options: TimeoutOption | undefined): Resolved {
    if (typeof options?.timeout === 'number')
      return { timeout: options.timeout }
    const set = this.set()
    return set === undefined
      ? { timeout: DEFAULT_TIMEOUT, builtIn: true }
      : { timeout: set }
  }

  /** goto, reload, history, waitForURL, waitForLoadState, setContent. */
  navigationTimeout(options: TimeoutOption | undefined): number {
    if (typeof options?.timeout === 'number') return options.timeout
    return this.inheritedNavigation()
  }

  /** The default this settings object or a parent set, if any. */
  private set(): number | undefined {
    return this.defaultTimeout ?? this.parent?.set()
  }

  private inheritedNavigation(): number {
    return (
      this.defaultNavigationTimeout ??
      this.defaultTimeout ??
      this.parent?.inheritedNavigation() ??
      DEFAULT_NAVIGATION_TIMEOUT
    )
  }
}
