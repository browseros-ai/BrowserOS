// biome-ignore-all lint/correctness/noEmptyPattern: Playwright parses fixture dependencies from this parameter.
/** Neo: Preserves upstream fixtures and exposes only real SDK or real Playwright browser objects. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { source } from '../js/prepare.mjs'
import { launchBrowser } from './browser.mjs'
import { contextOptions } from './context-options.mjs'
import { skipReason } from './curation.mjs'
import { createExpect } from './expect-loop.mjs'
import { openSdk } from './loader.mjs'

const require = createRequire(path.join(source, 'package.json'))
const { expect: realExpect } = require('@playwright/test')
let neoPage
let neoLocator

export function wrapExpect(original) {
  if (
    process.env.NEO_NODE_MODE !== 'neo' ||
    process.env.NEO_NODE_EXPECT_LOOP === '0'
  )
    return original
  return createExpect(original, {
    isNeo: (value) =>
      Boolean(
        neoPage && (value instanceof neoPage || value instanceof neoLocator),
      ),
    info: () => test.info(),
  })
}

export const expect = wrapExpect(realExpect)
const upstreamRequire = createRequire(
  path.join(process.env.NEO_NODE_OVERLAY, 'tests/config/browserTest.ts'),
)
const { browserTest } = upstreamRequire('./__neoOriginalBrowserTest.ts')
const builtinsFile = fs.readFileSync(
  new URL('../js/test-builtins.mjs', import.meta.url),
  'utf8',
)
const builtins = builtinsFile.slice(
  builtinsFile.indexOf('function exposeBuiltins'),
  builtinsFile.indexOf('/** Neo: One source'),
)
const builtinsSource = `(${builtins.trim()})(globalThis)`
const implicitOptions = new WeakMap()
const curationEntries = process.env.NEO_NODE_CURATION
  ? JSON.parse(fs.readFileSync(process.env.NEO_NODE_CURATION, 'utf8')).entries
  : []

function unsupported(name) {
  throw new Error(`needs Playwright internals: ${name}`)
}

function markImplicit(testInfo, name) {
  if (!implicitOptions.has(testInfo)) implicitOptions.set(testInfo, new Set())
  implicitOptions.get(testInfo).add(name)
}

const ownedTest = browserTest.extend({
  __neoCuration: [
    async ({}, use, testInfo) => {
      const file = path
        .relative(
          fs.realpathSync(process.env.NEO_NODE_OVERLAY),
          fs.realpathSync(testInfo.file),
        )
        .split(path.sep)
        .join('/')
      const reason = skipReason(
        curationEntries,
        file,
        // Worker title paths begin with the file; runner paths also have root/project suites.
        testInfo.titlePath.slice(1),
      )
      if (reason) testInfo.skip(true, reason)
      await use()
    },
    { auto: true },
  ],
  _reuseContext: [false, { scope: 'worker' }],
  __neoWorker: [
    async ({}, use, workerInfo) => {
      const output = path.join(
        process.env.NEO_NODE_OUTPUT,
        `worker-${workerInfo.workerIndex}`,
      )
      const owned = await launchBrowser(process.env.NEO_E2E_BROWSER, output, {
        slot: workerInfo.parallelIndex,
        launcherPid: Number(process.env.NEO_NODE_LAUNCHER_PID),
      })
      let closing = false
      owned.exited.then((exit) => {
        if (!closing)
          setImmediate(() => {
            throw new Error(`Worker BrowserOS exited: ${JSON.stringify(exit)}`)
          })
      })
      try {
        await use({ ...owned, output })
      } finally {
        closing = true
        await owned.close()
      }
    },
    { scope: 'worker', timeout: 60_000 },
  ],
})

const referenceTest = ownedTest.extend({
  browser: [
    async ({ playwright, __neoWorker }, use) => {
      const browser = await playwright.chromium.connectOverCDP(
        `http://127.0.0.1:${__neoWorker.port}`,
      )
      try {
        await use(browser)
      } finally {
        await browser.close()
      }
    },
    { scope: 'worker' },
  ],
})

const neoTest = ownedTest.extend({
  __neoExpectLoopSkip: [
    async ({}, use, testInfo) => {
      await use()
      // Upstream error-message tests can catch skip's exception and assert on it.
      // Preserve that requested skip, including its reason and any subsequent errors.
      if (
        testInfo.annotations.some(
          (annotation) =>
            annotation.type === 'skip' &&
            /^expectLoop: \w+ is not covered$/.test(
              annotation.description || '',
            ),
        )
      )
        testInfo.status = 'skipped'
    },
    { auto: true },
  ],
  __neo: [
    async ({ __neoWorker }, use) => {
      const sdk = await openSdk(__neoWorker.port, {
        // Playwright owns each test's timeout; this is only a worker lifetime fence.
        timeoutMs: 86_400_000,
        // Keep the real SDK's file sandbox aligned with upstream's cwd and test output paths.
        artifactsDir: fs.realpathSync(process.env.NEO_NODE_OVERLAY),
      })
      neoPage = sdk.page.constructor
      neoLocator = sdk.page.locator(':root').constructor
      try {
        await use(sdk)
      } finally {
        fs.writeFileSync(
          path.join(__neoWorker.output, 'outcome.json'),
          await sdk.finish(),
        )
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
  // A test.use override bypasses these initializers, so an explicit default remains explicit.
  offline: [
    async ({ contextOptions }, use, testInfo) => {
      if (contextOptions.offline == null) markImplicit(testInfo, 'offline')
      await use(contextOptions.offline ?? false)
    },
    { option: true },
  ],
  acceptDownloads: [
    async ({ contextOptions }, use, testInfo) => {
      if (contextOptions.acceptDownloads == null)
        markImplicit(testInfo, 'acceptDownloads')
      await use(contextOptions.acceptDownloads ?? true)
    },
    { option: true },
  ],
  _contextFactory: async (
    {
      _contextFactory,
      _combinedContextOptions,
      actionTimeout,
      navigationTimeout,
    },
    use,
    testInfo,
  ) => {
    if (process.env.PW_CLOCK) unsupported('clock fixture: context._wrapApiCall')
    await use(async (requested) => {
      const options = contextOptions(
        _combinedContextOptions,
        requested,
        implicitOptions.get(testInfo),
      )
      const result = await _contextFactory(options)
      result.context.setDefaultTimeout(actionTimeout || 0)
      result.context.setDefaultNavigationTimeout(navigationTimeout || 0)
      await result.context.addInitScript(builtinsSource)
      return result
    })
  },
  context: async ({ _contextFactory }, use) => {
    const { context, close } = await _contextFactory()
    try {
      await use(context)
    } finally {
      await close()
    }
  },
  _setupArtifacts: [
    async ({ trace, screenshot }, use) => {
      if ((typeof trace === 'string' ? trace : trace.mode) !== 'off')
        unsupported('trace fixture: playwright._instrumentation')
      if (
        (typeof screenshot === 'string' ? screenshot : screenshot.mode) !==
        'off'
      )
        unsupported('screenshot fixture: playwright._instrumentation')
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _setupContextOptions: [
    async ({ playwright, testIdAttribute }, use, testInfo) => {
      if (testIdAttribute)
        playwright.selectors.setTestIdAttribute(testIdAttribute)
      testInfo.snapshotSuffix = process.platform
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _browserOptions: [
    async ({ mode, headless, channel, launchOptions, connectOptions }, use) => {
      if (mode !== 'default') unsupported(`mode=${mode}`)
      if (
        !headless ||
        channel !== 'chromium' ||
        Object.keys(launchOptions).length ||
        connectOptions
      )
        unsupported('custom browser launch/connect options')
      await use({})
    },
    { scope: 'worker', auto: true },
  ],
  toImplInWorkerScope: [
    async ({}, use) => {
      await use(() => unsupported('toImpl'))
    },
    { scope: 'worker' },
  ],
  startRemoteServer: async ({}, use) => {
    await use(() => unsupported('startRemoteServer'))
  },
})

export const test =
  process.env.NEO_NODE_MODE === 'reference' ? referenceTest : neoTest
