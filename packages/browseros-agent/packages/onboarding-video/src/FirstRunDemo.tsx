/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Top-level composition for the cockpit first-run motion demo. Six
 * scenes sequenced against the timing constants in `timing.ts`.
 * Scene order tells the story: this dashboard, install the MCP,
 * prompt your agent, watch the run land, set it up below.
 */

import { AbsoluteFill, Sequence } from 'remotion'
import { fonts } from './fonts'
import { palette } from './palette'
import { SceneActivity } from './scenes/SceneActivity'
import { SceneCockpit } from './scenes/SceneCockpit'
import { SceneInstallMcp } from './scenes/SceneInstallMcp'
import { SceneLoop } from './scenes/SceneLoop'
import { ScenePan } from './scenes/ScenePan'
import { ScenePrompt } from './scenes/ScenePrompt'
import { SCENES } from './timing'

export function FirstRunDemo() {
  return (
    <AbsoluteFill
      style={{ background: palette.bgCanvas, fontFamily: fonts.sans }}
    >
      <Sequence
        from={SCENES.cockpit.from}
        durationInFrames={SCENES.cockpit.duration}
      >
        <SceneCockpit />
      </Sequence>
      <Sequence
        from={SCENES.installMcp.from}
        durationInFrames={SCENES.installMcp.duration}
      >
        <SceneInstallMcp />
      </Sequence>
      <Sequence from={SCENES.pan.from} durationInFrames={SCENES.pan.duration}>
        <ScenePan />
      </Sequence>
      <Sequence
        from={SCENES.prompt.from}
        durationInFrames={SCENES.prompt.duration}
      >
        <ScenePrompt />
      </Sequence>
      <Sequence
        from={SCENES.activity.from}
        durationInFrames={SCENES.activity.duration}
      >
        <SceneActivity />
      </Sequence>
      <Sequence from={SCENES.loop.from} durationInFrames={SCENES.loop.duration}>
        <SceneLoop />
      </Sequence>
    </AbsoluteFill>
  )
}
