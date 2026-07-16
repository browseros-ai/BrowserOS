/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { existsSync } from 'node:fs'
import path from 'node:path'
import { Config } from '@remotion/cli/config'
import { enableTailwind } from '@remotion/tailwind-v4'

Config.setVideoImageFormat('jpeg')
Config.setPixelFormat('yuv420p')
Config.setCodec('h264')

/**
 * Locate the claw-app root regardless of where Remotion was invoked
 * from. `import.meta.url` is undefined under Remotion's config loader
 * and `__dirname` resolves to the CLI's install directory, so we
 * walk up from `process.cwd()` looking for the WXT config file that
 * marks the claw-app root. Throws a loud error rather than silently
 * pointing the alias at the wrong tree.
 */
function findClawAppRoot(): string {
  let dir = process.cwd()
  const root = path.parse(dir).root
  while (dir !== root) {
    if (existsSync(path.join(dir, 'wxt.config.ts'))) {
      return dir
    }
    dir = path.dirname(dir)
  }
  throw new Error(
    `remotion.config.ts: could not locate the claw-app root by walking up from ${process.cwd()}. Invoke via the video:* npm scripts in apps/claw-app.`,
  )
}

const clawAppRoot = findClawAppRoot()

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

/**
 * Composition-only module swaps. Keys are absolute paths to the
 * module that would normally be imported; values are absolute paths
 * to the composition-local stub. Webpack alias matches on the
 * resolved request, so relative imports inside claw-app files
 * (e.g. `./FirstRunVideo` from CockpitOnboarding.tsx) are covered.
 *
 * Registers both extensionless and extensioned variants of each
 * key so alias matching succeeds regardless of when webpack
 * normalises the request path against the extension list.
 */
function overrideAlias(from: string, to: string): Record<string, string> {
  return {
    [from]: to,
    [`${from}.tsx`]: to,
    [`${from}.ts`]: to,
  }
}

const compositionOverrides: Record<string, string> = {
  ...overrideAlias(
    path.join(clawAppRoot, 'components/cockpit/FirstRunVideo'),
    path.join(
      clawAppRoot,
      'onboarding-video/src/components/FirstRunVideoStatic',
    ),
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
