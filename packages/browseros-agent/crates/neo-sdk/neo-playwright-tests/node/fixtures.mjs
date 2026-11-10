// biome-ignore-all lint/correctness/noEmptyPattern: Playwright parses fixture dependencies from this parameter.
/** Neo: Replaces browser ownership while preserving upstream's runner and context factory. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { source } from '../js/prepare.mjs'
import { launchBrowser } from './browser.mjs'
import { openSdk } from './loader.mjs'

const require = createRequire(path.join(source, 'package.json'))
export const { expect } = require('@playwright/test')
const upstreamRequire = createRequire(
  path.join(process.env.NEO_NODE_OVERLAY, 'tests/config/browserTest.ts'),
)
const { browserTest } = upstreamRequire('./__neoOriginalBrowserTest.ts')
// Reuse the existing helper's source without changing its CDP installer or the production SDK.
const builtinsFile = fs.readFileSync(
  new URL('../js/test-builtins.mjs', import.meta.url),
  'utf8',
)
const builtins = builtinsFile.slice(
  builtinsFile.indexOf('function exposeBuiltins'),
  builtinsFile.indexOf('/** Neo: One source'),
)
const builtinsSource = `(${builtins.trim()})(globalThis)`

function unsupported(name) {
  throw new Error(
    `Neo Node harness does not provide Playwright internal fixture ${name}`,
  )
}

export const test = browserTest.extend({
  __neo: [
    async ({}, use, workerInfo) => {
      const output = path.join(
        process.env.NEO_NODE_OUTPUT,
        `worker-${workerInfo.workerIndex}`,
      )
      const owned = await launchBrowser(process.env.NEO_E2E_BROWSER, output)
      let sdk
      try {
        sdk = await openSdk(owned.port, {
          timeoutMs: 1_800_000,
          artifactsDir: path.join(output, 'artifacts'),
        })
        await use(sdk)
      } finally {
        try {
          if (sdk)
            fs.writeFileSync(
              path.join(output, 'outcome.json'),
              await sdk.finish(),
            )
        } finally {
          await owned.close()
        }
      }
    },
    { scope: 'worker', timeout: 60_000 },
  ],
  playwright: [
    async ({ __neo }, use) => {
      await use(__neo.playwright)
    },
    { scope: 'worker' },
  ],
  browser: [
    async ({ __neo }, use) => {
      await use(__neo.context.browser())
    },
    { scope: 'worker' },
  ],
  context: async (
    {
      _contextFactory,
      _combinedContextOptions,
      actionTimeout,
      navigationTimeout,
    },
    use,
  ) => {
    const options = { ..._combinedContextOptions }
    // The pinned file never requests these options. Neo refuses even their upstream defaults;
    // record this fixture-only omission in overlay.json instead of changing product behavior.
    if (options.offline === false) delete options.offline
    if (options.acceptDownloads === true) delete options.acceptDownloads
    const { context, close } = await _contextFactory(options)
    context.setDefaultTimeout(actionTimeout || 0)
    context.setDefaultNavigationTimeout(navigationTimeout || 0)
    await context.addInitScript(builtinsSource)
    await use(context)
    await close()
  },
  _setupArtifacts: [
    async ({}, use) => {
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _setupContextOptions: [
    async ({ playwright, testIdAttribute }, use) => {
      if (testIdAttribute)
        playwright.selectors.setTestIdAttribute(testIdAttribute)
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _browserOptions: [
    async ({}, use) => {
      await use({})
    },
    { scope: 'worker', auto: true },
  ],
  _reuseContext: [false, { scope: 'worker' }],
  toImplInWorkerScope: [
    async ({}, use) => {
      await use(() => unsupported('toImpl'))
    },
    { scope: 'worker' },
  ],
  launchPersistent: async ({}, use) => {
    await use(() => unsupported('launchPersistent'))
  },
  startRemoteServer: async ({}, use) => {
    await use(() => unsupported('startRemoteServer'))
  },
})
