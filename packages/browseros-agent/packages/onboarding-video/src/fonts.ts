/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Loads the claw-app type system into the render and exposes each
 * family as a ready-to-use `fontFamily` stack. `loadFont()` blocks the
 * render until the font is ready on its own, so importing this module
 * once from the composition root is all the wiring needed — no manual
 * `delayRender`/`continueRender`. Fonts are fetched only at render time
 * (the shipped artefact is a baked MP4), never in the extension runtime.
 *
 * Families and fallbacks mirror apps/claw-app/entrypoints/newtab/styles.css:
 *   --font-sans:  "Schibsted Grotesk Variable", system-ui, sans-serif
 *   --font-serif: "Newsreader", Georgia, serif   (italic accent only)
 *   --font-mono:  "JetBrains Mono Variable", ui-monospace, monospace
 * Google Fonts serves Schibsted Grotesk / JetBrains Mono as the same
 * typefaces the app self-hosts via fontsource; at the weights used the
 * rendered output matches.
 */

import { loadFont as loadMono } from '@remotion/google-fonts/JetBrainsMono'
import { loadFont as loadSerif } from '@remotion/google-fonts/Newsreader'
import { loadFont as loadSans } from '@remotion/google-fonts/SchibstedGrotesk'

const { fontFamily: sans } = loadSans('normal', {
  weights: ['400', '500', '600', '700', '800', '900'],
  subsets: ['latin'],
})

// Newsreader is used only for the italic serif accent ("Your agent works.").
const { fontFamily: serif } = loadSerif('italic', {
  weights: ['500'],
  subsets: ['latin'],
})

const { fontFamily: mono } = loadMono('normal', {
  weights: ['400', '800'],
  subsets: ['latin'],
})

export const fonts = {
  sans: `"${sans}", system-ui, sans-serif`,
  serif: `"${serif}", Georgia, serif`,
  mono: `"${mono}", ui-monospace, monospace`,
} as const
