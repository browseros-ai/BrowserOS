/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Cockpit-side hook that injects the rrweb recorder into every
 * agent-driven BrowserOS page. Called from `register.ts` after a
 * successful `tabs new` dispatch, immediately next to
 * `ensureAgentTabGroup`. The hook is fire-and-forget; recorder
 * injection never blocks the agent's tool dispatch response.
 *
 * Why per-page rather than per-session: rrweb runs in the page
 * context. Each tab the agent opens is a distinct CDP target with
 * its own document, so we have to inject the recorder script onto
 * every target individually. We dedupe per pageId per process
 * lifetime so a second `tabs new` for the same page (rare but
 * possible) does not double-install.
 *
 * The recorder script is built by `buildInitScript` from the
 * vendored rrweb UMD bundle. Each tab gets its own per-session blob
 * with sessionId + tabPageId + cockpit loopback origin baked in.
 *
 * `Page.addScriptToEvaluateOnNewDocument` runs the script before
 * any page script on EVERY future navigation of that target,
 * including the current document if it is already past navigation
 * start (covered by `runImmediately: true`). So SPA route changes
 * within the tab keep the recorder active without re-injection.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { env } from '../env'
import { logger } from '../lib/logger'
import { getLocalServerUrl } from '../local-server-url'
import { buildInitScript } from '../replay/recorder-source'

/**
 * Set of pageIds we have already injected for in this process. A
 * server restart resets this; the next `tabs new` after restart
 * re-injects.
 */
const injected = new Set<number>()

/**
 * Kill switch for the recorder. The current rrweb config emits
 * synchronously on every DOM mutation, which on heavy SPA pages
 * (HN /show, GitHub trending) backs up the main thread and locks
 * the page. While we throttle the emit handler (move serialisation
 * off the main thread, debounce flushes, drop low-information event
 * types), the default ships OFF. Set CLAW_REPLAY_ENABLED=1 to opt
 * in for dogfood.
 *
 * Read from the shared `env` snapshot at call time so tests can
 * flip `env.replayEnabled` without remounting the module.
 */
function isReplayEnabled(): boolean {
  return env.replayEnabled === true
}

export interface EnsureReplayRecorderInput {
  sessionId: string
  slug: string
  pageId: number
  session: BrowserSession
}

export async function ensureReplayRecorder(
  input: EnsureReplayRecorderInput,
): Promise<void> {
  if (!isReplayEnabled()) return
  if (injected.has(input.pageId)) return
  if (input.sessionId.length === 0) {
    // The dispatch path may invoke us with an empty sessionId when
    // the agent's MCP session has not been initialised yet. Without
    // a sessionId we have no target to POST events to; skip silently
    // and let the next dispatch retry.
    return
  }
  const cockpitOrigin = getLocalServerUrl()
  if (!cockpitOrigin) {
    logger.warn('replay injection skipped: cockpit URL not yet set', {
      sessionId: input.sessionId,
      pageId: input.pageId,
    })
    return
  }

  const source = buildInitScript({
    sessionId: input.sessionId,
    tabPageId: input.pageId,
    cockpitOrigin,
  })

  try {
    const { session: cdp } = await input.session.pages.getSession(input.pageId)
    await cdp.Page.addScriptToEvaluateOnNewDocument({
      source,
      // Run the script on the current document too if it has already
      // loaded by the time we attach. Without this the agent's first
      // `tabs new` could miss the initial DOM snapshot when the
      // about:blank navigation completes before injection.
      runImmediately: true,
    })
    injected.add(input.pageId)
  } catch (err) {
    logger.warn('replay injection failed', {
      sessionId: input.sessionId,
      pageId: input.pageId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

/** Test-only escape hatch so successive tests do not see stale state. */
export function _resetReplayInjectionForTesting(): void {
  injected.clear()
}
