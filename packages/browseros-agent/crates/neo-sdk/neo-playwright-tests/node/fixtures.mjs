// biome-ignore-all lint/correctness/noEmptyPattern: Playwright parses fixture dependencies from this parameter.
/** Neo: Preserves upstream fixtures and exposes only real SDK or real Playwright browser objects. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { source } from '../js/prepare.mjs'
import { browserVersion, launchBrowser } from './browser.mjs'
import { clawContextOptions, clawDefaults } from './claw-context-options.mjs'
import {
  installFixtureBuiltins,
  openHarness,
  pageTargets,
  resetClawTest,
  serverOrigins,
} from './claw-harness.mjs'
import { contextOptions } from './context-options.mjs'
import { skipReason } from './curation.mjs'
import { createExpect } from './expect-loop.mjs'

const require = createRequire(path.join(source, 'package.json'))
const { expect: realExpect } = require('@playwright/test')
let neoPage
let neoLocator

export function wrapExpect(original) {
  if (
    !['neo', 'claw'].includes(process.env.NEO_NODE_MODE) ||
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

const sdkTest = ownedTest.extend({
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

const neoTest = sdkTest.extend({
  __neo: [
    async ({ __neoWorker }, use) => {
      const { openSdk } = await import('./loader.mjs')
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
})

const clawOptionFixtures = Object.fromEntries(
  Object.entries(clawDefaults).map(([name, value]) => [
    name,
    [
      async ({ contextOptions }, use, testInfo) => {
        // test.use({ option: default }) bypasses this initializer and stays explicit.
        if (!Object.hasOwn(contextOptions, name)) markImplicit(testInfo, name)
        await use(
          Object.hasOwn(contextOptions, name) ? contextOptions[name] : value,
        )
      },
      { option: true },
    ],
  ]),
)

const clawTest = sdkTest.extend({
  ...clawOptionFixtures,
  __claw: [
    async ({ __neoWorker }, use) => {
      const { openClaw } = await import('./claw-loader.mjs')
      const sdk = await openClaw(__neoWorker.port, {
        timeoutMs: 86_400_000,
        stateDir: path.join(__neoWorker.output, 'claw-state'),
        executionDir: path.join(__neoWorker.profile, '.browseros'),
        browserosVersion: browserVersion(process.env.NEO_E2E_BROWSER),
      })
      const initialPage = sdk.page || sdk.context.pages()[0]
      if (initialPage) {
        neoPage = initialPage.constructor
        neoLocator = initialPage.locator(':root').constructor
      }
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
  __clawHarness: [
    async ({ __neoWorker }, use) => {
      const cdp = await openHarness(__neoWorker.port)
      try {
        await use(cdp)
      } finally {
        await cdp.close()
      }
    },
    { scope: 'worker' },
  ],
  playwright: [
    async ({}, use) => {
      // v1 has no Playwright namespace. Missing members remain undefined.
      await use({})
    },
    { scope: 'worker' },
  ],
  browser: [
    async ({ __claw }, use) => {
      await use(__claw.browser)
    },
    { scope: 'worker' },
  ],
  __clawIsolation: [
    async (
      {
        __claw,
        __clawHarness,
        __neoWorker,
        loopback,
        actionTimeout,
        navigationTimeout,
      },
      use,
    ) => {
      const context = __claw.context
      const initialPages = new Set(context.pages())
      context.setDefaultTimeout(actionTimeout || 0)
      context.setDefaultNavigationTimeout(navigationTimeout || 0)
      try {
        await use()
      } finally {
        await resetClawTest({
          context,
          initialPages,
          cdp: __clawHarness,
          origins: serverOrigins(__neoWorker.serverPort, loopback),
        })
      }
    },
    { auto: true },
  ],
  _contextFactory: async (
    { __claw, _combinedContextOptions, video },
    use,
    testInfo,
  ) => {
    if (process.env.PW_CLOCK) unsupported('clock fixture: context._wrapApiCall')
    let calls = 0
    await use(async (requested) => {
      const explicit = clawContextOptions(
        _combinedContextOptions,
        requested,
        implicitOptions.get(testInfo),
      )
      if ((typeof video === 'string' ? video : video.mode) !== 'off')
        explicit.recordVideo = { dir: testInfo.outputPath('video') }
      calls++
      const forced = Object.keys(explicit).length
        ? `explicit options: ${JSON.stringify(explicit)}`
        : calls > 1
          ? 'second context in one test'
          : null
      if (forced) {
        testInfo.annotations.push({ type: 'claw-context', description: forced })
        const context = await __claw.browser.newContext(explicit)
        return { context, close: () => context.close() }
      }
      return { context: __claw.context, close: async () => {} }
    })
  },
  __clawPageTargets: async ({ context, __clawHarness }, use) => {
    // The dependency resolves before upstream's page fixture calls context.newPage().
    if (!context) throw new Error('harness-error: no fixture context')
    await use(await pageTargets(__clawHarness))
  },
  page: async ({ __clawPageTargets, page, __clawHarness }, use) => {
    await installFixtureBuiltins(__clawHarness, __clawPageTargets)
    neoPage = page.constructor
    neoLocator = page.locator(':root').constructor
    await use(page)
  },
})

export const test =
  process.env.NEO_NODE_MODE === 'reference'
    ? referenceTest
    : process.env.NEO_NODE_MODE === 'claw'
      ? clawTest
      : neoTest
