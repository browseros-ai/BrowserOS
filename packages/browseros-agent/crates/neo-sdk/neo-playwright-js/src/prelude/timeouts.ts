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

/** A whole number of milliseconds, as the header carries; anything else is refused. */
export function timeoutMs(api: string, timeout: number): number {
  if (!Number.isFinite(timeout) || timeout < 0)
    throw clientError(
      api,
      `timeout: expected a non-negative number, got ${timeout}`,
    )
  return Math.ceil(timeout)
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
    if (typeof options?.timeout === 'number') return options.timeout
    return this.inherited()
  }

  /** goto, reload, history, waitForURL, waitForLoadState, setContent. */
  navigationTimeout(options: TimeoutOption | undefined): number {
    if (typeof options?.timeout === 'number') return options.timeout
    return this.inheritedNavigation()
  }

  private inherited(): number {
    return this.defaultTimeout ?? this.parent?.inherited() ?? DEFAULT_TIMEOUT
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
