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
 * read DOM text), no automatic pageviews, no feature-flag polling, no
 * console recording, and no person profiles. A 20% sample of consenting
 * cockpit sessions may be recorded with inputs masked, task-bearing DOM
 * blocked, and replay URLs replaced by a fixed token. Auto-captured location properties
 * (`$current_url` etc.) are stripped so even the cockpit's own extension
 * URL never leaves. Identity is the server's anonymous install UUID, set
 * via `bootstrap.distinctID` (no `identify`, no PII).
 *
 * Gated on a build-time project write key (`VITE_CLAW_POSTHOG_KEY`) and
 * the user's consent. With no key, or before consent, nothing is
 * initialised and every capture no-ops.
 */

import posthog, { type PostHogConfig } from 'posthog-js'
import 'posthog-js/dist/posthog-recorder'

const KEY = import.meta.env.VITE_CLAW_POSTHOG_KEY as string | undefined
const HOST =
  (import.meta.env.VITE_CLAW_POSTHOG_HOST as string | undefined) ??
  'https://us.i.posthog.com'
const REDACTED_REPLAY_URL = 'browserclaw://redacted'

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

export function sanitizeProperties(
  properties: Record<string, unknown>,
): Record<string, unknown> {
  const cleaned = { ...properties }
  for (const key of STRIPPED_PROPS) delete cleaned[key]
  return cleaned
}

/** Removes route, session, and local API identifiers from replay URLs. */
export function maskCapturedReplayRequest<T extends { name: string }>(
  request: T,
): T {
  return { ...request, name: REDACTED_REPLAY_URL }
}

export function createPostHogConfig(
  distinctId: string,
): Partial<PostHogConfig> {
  return {
    api_host: HOST,
    autocapture: false,
    capture_pageview: false,
    capture_pageleave: false,
    disable_external_dependency_loading: true,
    disable_capture_url_hashes: true,
    enable_recording_console_log: false,
    // Reconciled explicitly after effective consent is known.
    disable_session_recording: true,
    disable_surveys: true,
    // Session replay needs PostHog's remote config, but BrowserClaw does not use
    // feature flags. Fetch config once, without evaluating or polling flags.
    advanced_disable_feature_flags_on_first_load: true,
    remote_config_refresh_interval_ms: 0,
    // Do not persist browser location metadata outside the event sanitizer.
    save_campaign_params: false,
    save_referrer: false,
    // We never call identify(), so never create a person profile.
    person_profiles: 'never',
    persistence: 'localStorage',
    // Share the server's anonymous install id so both surfaces map to
    // one install, without identify().
    bootstrap: { distinctID: distinctId },
    sanitize_properties: sanitizeProperties,
    session_recording: {
      blockClass: 'ph-no-capture',
      collectFonts: false,
      maskAllInputs: true,
      maskCapturedNetworkRequestFn: maskCapturedReplayRequest,
      recordBody: false,
      recordCrossOriginIframes: false,
      recordHeaders: false,
      sampleRate: 0.2,
    },
  }
}

interface RecordingConsentClient {
  opt_in_capturing(options?: { captureEventName?: string | null | false }): void
  opt_out_capturing(): void
  startSessionRecording(): void
  stopSessionRecording(): void
}

/**
 * Keeps capture consent and session recording in lockstep. Starting without
 * an override is important: it lets session_recording.sampleRate decide.
 */
export function reconcileSessionRecording(
  client: RecordingConsentClient,
  enabled: boolean,
  isInitialised: boolean,
): void {
  if (enabled) {
    // The server state is authoritative, so clear any stale SDK-local opt-out.
    // Suppress PostHog's own opt-in event; BrowserClaw sends its allowlisted one.
    client.opt_in_capturing({ captureEventName: false })
    client.startSessionRecording()
  } else if (isInitialised) {
    client.stopSessionRecording()
    client.opt_out_capturing()
  }
}

function init(distinctId: string): void {
  initialised = true
  posthog.init(KEY as string, createPostHogConfig(distinctId))
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
    const wasInitialised = initialised
    if (!wasInitialised) init(input.distinctId)
    reconcileSessionRecording(posthog, true, wasInitialised)
  } else {
    reconcileSessionRecording(posthog, false, initialised)
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
