/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * CDP attach helper for the cockpit's standalone runtime.
 *
 * Mirrors how `apps/server` connects to the BrowserOS Chromium:
 * instantiate a `CdpBackend` against the configured port, dial, wrap
 * the connection in a `Browser`, and hand the resulting
 * `BrowserSession` back so `main.ts` can pin it onto the cockpit's
 * process-wide singleton.
 *
 * `exitOnReconnectFailure: false` is deliberate. apps/server runs as
 * a child of the BrowserOS browser shell, so exiting on a dropped
 * CDP connection is the right escalation; the parent restarts it.
 * The standalone cockpit is user-owned — a transient CDP drop must
 * degrade to "session not connected" until BrowserOS is back up,
 * not kill the process and lose the UI tab.
 *
 * Soft fail at boot: if BrowserOS is not running on the configured
 * port at startup, return `null` and log a warning. The cockpit
 * keeps serving the UI, the profile CRUD, the harness installs, and
 * `tools/list`; only `tools/call` short-circuits with the existing
 * "browser session not connected" wire shape. Restart the cockpit
 * after BrowserOS is up to reattach.
 */

import { CdpBackend } from '@browseros/browser-core/backends/cdp'
import { Browser } from '@browseros/browser-core/browser'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { env } from '../env'
import { logger } from './logger'

export interface BrowserBootstrap {
  session: BrowserSession
  disconnect(): Promise<void>
}

/**
 * Minimal seam every test needs: a CDP-like value with `connect` and
 * `disconnect` methods, so the unit test can supply a stub without
 * touching the network. The runtime path passes a real `CdpBackend`.
 */
export interface CdpClient {
  connect(): Promise<void>
  disconnect(): Promise<void>
}

/**
 * Test seam for swapping out the CDP attach machinery as a single
 * unit. `cdpFactory` and `buildSession` are deliberately bundled here
 * rather than exposed as two independent optional overrides: the
 * default `buildSession` casts its argument to a real `CdpBackend`
 * before instantiating `Browser`, so mixing a stub `cdpFactory` with
 * the default `buildSession` would compile but blow up at the first
 * `Browser` call. Callers either pass no `inject` (production) or
 * pass both factories (tests).
 */
export interface BrowserBootstrapInjection {
  cdpFactory: (port: number) => CdpClient
  buildSession: (cdp: CdpClient) => BrowserSession
}

export interface BrowserBootstrapDeps {
  /** Test-only: replace the entire CDP attach machinery with stubs. */
  inject?: BrowserBootstrapInjection
}

const defaultInjection: BrowserBootstrapInjection = {
  cdpFactory: (port) => new CdpBackend({ port, exitOnReconnectFailure: false }),
  buildSession: (cdp) => new Browser(cdp as unknown as CdpBackend).session,
}

export async function bootstrapBrowserosBrowser(
  deps: BrowserBootstrapDeps = {},
): Promise<BrowserBootstrap | null> {
  const { cdpFactory, buildSession } = deps.inject ?? defaultInjection
  const port = env.cdpPort
  const cdp = cdpFactory(port)
  try {
    await cdp.connect()
  } catch (err) {
    logger.warn(
      'browseros browser unreachable on cdp port; cockpit will boot without a session',
      {
        port,
        error: err instanceof Error ? err.message : String(err),
      },
    )
    return null
  }
  const session = buildSession(cdp)
  return {
    session,
    disconnect: async () => {
      try {
        await cdp.disconnect()
      } catch (err) {
        logger.warn('cdp disconnect failed', {
          error: err instanceof Error ? err.message : String(err),
        })
      }
    },
  }
}
