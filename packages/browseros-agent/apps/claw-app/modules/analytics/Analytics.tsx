/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Headless analytics driver. Rendered once inside the router: it
 * reconciles posthog-js with the shared telemetry state, fires
 * `app_opened` once, and emits a view event on each cockpit route
 * change. Renders nothing.
 */

import { useEffect, useRef } from 'react'
import { useLocation } from 'react-router'
import { AnalyticsEvent, screenEventForPath, track } from './events'
import { applyTelemetry } from './posthog'
import { useTelemetryState } from './telemetry.hooks'

export function Analytics() {
  const { data } = useTelemetryState()
  const location = useLocation()
  const appOpened = useRef(false)

  // External-system integration: init/opt-in/opt-out posthog-js from the
  // server-owned consent, then record the first open once analytics is
  // live (capture no-ops until then, so ordering is safe).
  useEffect(() => {
    if (!data) return
    applyTelemetry({ distinctId: data.distinctId, consent: data.consent })
    if (!appOpened.current) {
      appOpened.current = true
      track(AnalyticsEvent.AppOpened)
    }
  }, [data])

  // Route changes are the only source of screen-view events; there is no
  // handler to lift this into, so it lives in an effect keyed on the path.
  useEffect(() => {
    const event = screenEventForPath(location.pathname)
    if (event) track(event)
  }, [location.pathname])

  return null
}
