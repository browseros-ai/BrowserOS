/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run how-to bento: a mixed-size grid of poster tiles that open
 * a shared lightbox player. Replaces the single setup demo now that
 * agents auto-connect at launch, so onboarding leads with how to put
 * BrowserClaw to work instead of how to install it.
 *
 * Desktop is a 3-column bento: one featured tile (2x2), a stacked
 * rail, and a wide tile. Mobile collapses to a single column of
 * 16:9 tiles. Fixed row height plus object-cover keeps every cell
 * aligned regardless of poster aspect.
 */

import { useState } from 'react'
import { ONBOARDING_VIDEOS, type OnboardingVideo } from './cockpit-videos'
import { VideoLightbox } from './VideoLightbox'
import { VideoTile } from './VideoTile'

export function VideoBento() {
  const [active, setActive] = useState<OnboardingVideo | null>(null)
  return (
    <>
      <div className="grid grid-cols-1 gap-3 md:auto-rows-[13.5rem] md:grid-cols-3">
        {ONBOARDING_VIDEOS.map((video) => (
          <VideoTile key={video.id} video={video} onPlay={setActive} />
        ))}
      </div>
      <VideoLightbox video={active} onClose={() => setActive(null)} />
    </>
  )
}
