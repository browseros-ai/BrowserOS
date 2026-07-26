/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 04: the activity. Camera swings back to center the cockpit.
 * The pulsing dot in the recent-activity table morphs into a real
 * live task row with a scrubbing progress bar. Caption below reads
 * "you watch. your agent works." as the punchline of the sequence.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitFrame } from '../components/CockpitFrame'
import { fonts } from '../fonts'
import { palette } from '../palette'

const SETTLE_END = 45
const PROGRESS_END = 130

export function SceneActivity() {
  const frame = useCurrentFrame()
  // Cockpit swings back to centered + full size.
  const settle = interpolate(frame, [0, SETTLE_END], [0, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const scale = interpolate(settle, [0, 1], [0.78, 1])
  const translateX = interpolate(settle, [0, 1], [-340, 0])
  // Task progress fills over ~2.8s once we settle.
  const taskProgress = interpolate(
    frame,
    [SETTLE_END, PROGRESS_END],
    [0.05, 0.82],
    { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' },
  )
  const captionOpacity = interpolate(frame, [80, 110], [0, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      <div
        style={{
          position: 'absolute',
          left: 60,
          top: 90,
          width: 900,
          height: 640,
          scale,
          translate: `${translateX}px 0px`,
          transformOrigin: 'top left',
        }}
      >
        <CockpitFrame
          liveTask={{
            agent: 'claude-code',
            action: 'browsing sfo to nyc, morning',
            progress: taskProgress,
          }}
        />
      </div>
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          bottom: 80,
          textAlign: 'center',
          fontSize: 32,
          fontWeight: 700,
          color: palette.ink,
          letterSpacing: -0.8,
          opacity: captionOpacity,
        }}
      >
        You watch.{' '}
        <span
          style={{
            fontFamily: fonts.serif,
            fontStyle: 'italic',
            color: palette.accent,
            fontWeight: 500,
          }}
        >
          Your agent works.
        </span>
      </div>
    </AbsoluteFill>
  )
}
