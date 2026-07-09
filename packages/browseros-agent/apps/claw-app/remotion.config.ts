/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Config } from '@remotion/cli/config'
import { enableTailwind } from '@remotion/tailwind-v4'

Config.setVideoImageFormat('jpeg')
Config.setPixelFormat('yuv420p')
Config.setCodec('h264')

// Wire Tailwind v4 into Remotion's webpack so composition components
// can consume the same utility classes as the shipped extension.
// Design tokens (colours, fonts, radii, shadows) come from the app's
// `@theme inline` block via
// `apps/claw-app/entrypoints/newtab/styles.css`, imported at the top
// of `onboarding-video/src/index.css`. See docs/browserclaw or the
// PR body for the reasoning.
Config.overrideWebpackConfig((c) => enableTailwind(c))
