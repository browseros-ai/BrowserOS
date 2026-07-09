/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Remotion Studio + CLI entry. Registers the FirstRunDemo
 * composition at 1600x900 / 30fps / 20s.
 *
 * Render to WebM (VP9) locally with:
 *   bunx remotion render src/Root.tsx FirstRunDemo out/first-run-demo.mp4
 *
 * Render a still for the poster with:
 *   bunx remotion still src/Root.tsx FirstRunDemo out/first-run-demo-poster.png --frame 0
 */

import { Composition } from 'remotion'
import './index.css'
import { FirstRunDemo } from './FirstRunDemo'
import { FPS, TOTAL_FRAMES } from './timing'

const WIDTH = 1600
const HEIGHT = 900

export function RemotionRoot() {
  return (
    <Composition
      id="FirstRunDemo"
      component={FirstRunDemo}
      durationInFrames={TOTAL_FRAMES}
      fps={FPS}
      width={WIDTH}
      height={HEIGHT}
    />
  )
}
