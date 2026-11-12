import type { SiteCheck } from '../fixtures/catalog'
import type { SolveResult } from './solve-result'

export interface Timeouts {
  /** Widget readiness, submit and the site's verdict. */
  probeMs: number
  navigationMs: number
  solveMs: number
  /** claw-server only: the runner polls its probes. */
  pollMs: number
}

/** One catalog page in one tab, over one server's tools. */
export interface PageSession {
  open(url: string): Promise<void>
  /** False when the widget did not render in time. */
  ready(expression: string): Promise<boolean>
  /** Never throws: a failed call is a result with `error` set. */
  solve(): Promise<SolveResult>
  /** The page's own token and callback state; lengths only. */
  observe(tokenExpression: string): Promise<unknown>
  /** Submits the form; the site's verdict, or null when none appeared in time. */
  submit(site: SiteCheck): Promise<'accepted' | 'rejected' | null>
  close(): Promise<void>
}
