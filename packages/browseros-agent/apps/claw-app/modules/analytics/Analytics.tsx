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
import { usePlan } from '@/modules/api/plan.hooks'
import { planContextFor } from '@/modules/plan/plan.helpers'
import { AnalyticsEvent, screenEventForPath, track } from './events'
import { applyTelemetry, isCapturing, setPlanContext } from './posthog'
import { useTelemetryState } from './telemetry.hooks'

export function Analytics() {
  const { data } = useTelemetryState()
  const plan = usePlan()
  const location = useLocation()
  const appOpened = useRef(false)
  const lastViewedPath = useRef<string | null>(null)

  // External-system integration. Reconcile posthog with the server's
  // effective telemetry state, then emit events. The guards
  // (`appOpened`, `lastViewedPath`) are only consumed once posthog is
  // actually capturing, so:
  //   - a cold open on a deep link fires its view event once telemetry
  //     initialises rather than being dropped, and
  //   - opting in later still sends `app_opened` (the guard was not
  //     burned while capture was a no-op).
  // Runs on telemetry state changes and on navigation.
  // Tag events with the plan before any of them fires; posthog reads it at capture time.
  useEffect(() => {
    setPlanContext(planContextFor(plan.data))
  }, [plan.data])

  useEffect(() => {
    if (!data) return
    applyTelemetry({ distinctId: data.distinctId, enabled: data.enabled })
    // The first events wait for the plan, so they carry it; a failed plan read still lets them go.
    if (!isCapturing() || plan.isPending) return
    if (!appOpened.current) {
      appOpened.current = true
      track(AnalyticsEvent.AppOpened)
    }
    if (lastViewedPath.current !== location.pathname) {
      lastViewedPath.current = location.pathname
      const event = screenEventForPath(location.pathname)
      if (event) track(event)
    }
  }, [data, location.pathname, plan.isPending])

  return null
}
