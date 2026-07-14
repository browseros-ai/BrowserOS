/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import path from 'node:path'
import { Config } from '@remotion/cli/config'
import { enableTailwind } from '@remotion/tailwind-v4'

Config.setVideoImageFormat('jpeg')
Config.setPixelFormat('yuv420p')
Config.setCodec('h264')

// Wire Tailwind v4 into Remotion's webpack so composition components
// can consume the same utility classes as the shipped extension.
// Design tokens (colours, fonts, radii, shadows) come from the app's
// `@theme inline` block via
// `apps/claw-app/entrypoints/newtab/tokens.css`, imported at the top
// of `onboarding-video/src/index.css`. Also teaches webpack the
// `@/*` alias so composition scenes can import claw-app components
// (e.g. `@/components/cockpit/CockpitOnboarding`) the same way any
// other claw-app file does. TypeScript already resolves the alias
// via `tsconfig.json` paths; webpack needs it separately for the
// actual module bundle.
// Remotion invokes the render command from the claw-app root
// (`packages/browseros-agent/apps/claw-app`) via the `video:*` npm
// scripts, so `process.cwd()` is the correct anchor for the alias
// target. `import.meta.url` is not preserved by Remotion's config
// loader, so we cannot derive the dirname from the config file
// itself.
const clawAppRoot = process.cwd()

// Composition-only module swaps. Keys are absolute paths to the
// module that would normally be imported; values are absolute paths
// to the composition-local stub. Webpack alias matches on the
// resolved request, so relative imports inside claw-app files
// (e.g. `./FirstRunVideo` from CockpitOnboarding.tsx) are covered.
const compositionOverrides: Record<string, string> = {
  [path.join(clawAppRoot, 'components/cockpit/FirstRunVideo')]: path.join(
    clawAppRoot,
    'onboarding-video/src/components/FirstRunVideoStatic',
  ),
}

Config.overrideWebpackConfig((current) => {
  const withTailwind = enableTailwind(current)
  return {
    ...withTailwind,
    resolve: {
      ...withTailwind.resolve,
      alias: {
        ...withTailwind.resolve?.alias,
        '@': clawAppRoot,
        ...compositionOverrides,
      },
    },
  }
})
