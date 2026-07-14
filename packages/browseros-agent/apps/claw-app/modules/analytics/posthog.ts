/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Locked-down posthog-js wrapper for the cockpit UI.
 *
 * Privacy stance mirrors the server: measure the product, never the
 * user. This module is imported ONLY by the cockpit newtab surface,
 * never by the recorder content script or background worker, so
 * analytics never runs on the pages the user browses.
 *
 * posthog-js defaults are aggressively disabled: no autocapture (would
 * read DOM text), no session recording, no automatic pageviews, no
 * `/decide` calls, and no person profiles. Auto-captured location
 * properties (`$current_url` etc.) are stripped so even the cockpit's
 * own extension URL never leaves. Identity is the server's anonymous
 * install UUID, set via `bootstrap.distinctID` (no `identify`, no PII).
 *
 * Gated on a build-time project write key (`VITE_CLAW_POSTHOG_KEY`) and
 * the user's consent. With no key, or before consent, nothing is
 * initialised and every capture no-ops.
 */

import posthog from 'posthog-js'

const KEY = import.meta.env.VITE_CLAW_POSTHOG_KEY as string | undefined
const HOST =
  (import.meta.env.VITE_CLAW_POSTHOG_HOST as string | undefined) ??
  'https://us.i.posthog.com'

/** Auto-added properties that could carry a url/referrer; always removed. */
const STRIPPED_PROPS = [
  '$current_url',
  '$pathname',
  '$host',
  '$referrer',
  '$referring_domain',
  '$initial_current_url',
  '$initial_pathname',
  '$initial_referrer',
  '$initial_referring_domain',
]

let initialised = false

function init(distinctId: string): void {
  initialised = true
  posthog.init(KEY as string, {
    api_host: HOST,
    autocapture: false,
    capture_pageview: false,
    capture_pageleave: false,
    disable_session_recording: true,
    disable_surveys: true,
    // No feature-flag / /decide round-trips: we only send events.
    advanced_disable_decide: true,
    // We never call identify(), so never create a person profile.
    person_profiles: 'never',
    persistence: 'localStorage',
    // Share the server's anonymous install id so both surfaces map to
    // one install, without identify().
    bootstrap: { distinctID: distinctId },
    sanitize_properties: (props) => {
      const cleaned: Record<string, unknown> = { ...props }
      for (const key of STRIPPED_PROPS) delete cleaned[key]
      return cleaned
    },
  })
}

/**
 * Reconciles the posthog client with the server's EFFECTIVE telemetry
 * state. `enabled` already folds in the user's consent, the operator
 * kill-switch, and the server key, so the cockpit respects all three by
 * gating on it. Initialises on first enable, opts in/out on later
 * changes, no-ops without a Vite key. Safe to call repeatedly.
 */
export function applyTelemetry(input: {
  distinctId: string
  enabled: boolean
}): void {
  if (!KEY || !input.distinctId) return
  if (input.enabled) {
    if (!initialised) init(input.distinctId)
    else posthog.opt_in_capturing()
  } else if (initialised) {
    posthog.opt_out_capturing()
  }
}

/** Whether posthog is initialised AND currently opted in to capturing. */
export function isCapturing(): boolean {
  return initialised && !posthog.has_opted_out_capturing()
}

/** Fire-and-forget event. No-ops until capturing. */
export function capture(
  event: string,
  properties?: Record<string, unknown>,
): void {
  if (!isCapturing()) return
  posthog.capture(event, properties)
}
