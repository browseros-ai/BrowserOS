/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useSyncExternalStore } from 'react'
import { isCapturing, subscribeToCaptureState } from './posthog'

/**
 * Whether PostHog can send events right now. PostHog starts after the first
 * render and can be switched off, so "shown" events re-run on this to send
 * once it is ready instead of being lost.
 */
export function useCaptureReady(): boolean {
  return useSyncExternalStore(subscribeToCaptureState, isCapturing, () => false)
}
