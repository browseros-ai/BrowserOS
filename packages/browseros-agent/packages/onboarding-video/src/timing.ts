/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Named frame ranges for the FirstRunDemo composition. Every scene
 * consumes `SCENES[<name>]` so a timing change stays a one-line
 * diff. Total duration is TOTAL_FRAMES.
 *
 * Ordering names the story the video tells:
 *   cockpit    "you are here" (this is the dashboard)
 *   installMcp "first: install the MCP"
 *   pan        "then: prompt your agent"
 *   prompt     the operator types into their agent terminal
 *   activity   the run lands back in the cockpit
 *   loop       "set it up below" + fade back to cockpit
 */

export const FPS = 30

const SECONDS = (n: number) => Math.round(n * FPS)

export const SCENES = {
  cockpit: { from: 0, duration: SECONDS(2.5) },
  installMcp: { from: SECONDS(2.5), duration: SECONDS(3.5) },
  pan: { from: SECONDS(6), duration: SECONDS(3) },
  prompt: { from: SECONDS(9), duration: SECONDS(4) },
  activity: { from: SECONDS(13), duration: SECONDS(4) },
  loop: { from: SECONDS(17), duration: SECONDS(3) },
} as const

export const TOTAL_FRAMES = SECONDS(20)
