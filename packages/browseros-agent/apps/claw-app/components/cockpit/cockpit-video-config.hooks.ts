/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useState } from 'react'
import {
  getRemoteConfigPayload,
  onRemoteConfig,
} from '@/modules/analytics/posthog'
import {
  COCKPIT_VIDEOS_FLAG,
  type CockpitVideoLineup,
  parseCockpitVideoLineup,
} from './cockpit-video-config'

/**
 * The cockpit video lineup from PostHog remote config. Empty until the config
 * loads, and stays empty when telemetry is off or no PostHog key is set, so
 * callers hide the section on `videos.length === 0`.
 */
export function useCockpitVideos(): CockpitVideoLineup {
  const [lineup, setLineup] = useState<CockpitVideoLineup>(() =>
    parseCockpitVideoLineup(getRemoteConfigPayload(COCKPIT_VIDEOS_FLAG)),
  )
  useEffect(() => {
    const read = () =>
      setLineup(
        parseCockpitVideoLineup(getRemoteConfigPayload(COCKPIT_VIDEOS_FLAG)),
      )
    read()
    // External system: PostHog loads remote config asynchronously after init;
    // re-read when it arrives. No-op unsubscribe when posthog is not ready.
    return onRemoteConfig(read)
  }, [])
  return lineup
}
