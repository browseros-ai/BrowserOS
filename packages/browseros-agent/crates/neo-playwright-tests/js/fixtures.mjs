// biome-ignore-all lint/correctness/noEmptyPattern: Playwright parses fixture dependencies from this required parameter syntax.
/** Neo: Upstream fixtures own test semantics; this seam supplies only engine-backed browser objects. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { contextBootstrapPlan } from './browser.mjs'
import { source } from './prepare.mjs'
import { NativeProxy, ReferenceProxy } from './proxy.mjs'

const require = createRequire(path.join(source, 'package.json'))
const { expect } = require('@playwright/test')
const playwright = require('playwright-core')

export { expect }

// Playwright transforms this upstream TypeScript module in both collection and workers.
const overlayRequire = createRequire(
  path.join(process.env.NEO_TESTS_OVERLAY, 'tests/config/browserTest.ts'),
)
const { browserTest } = overlayRequire('./__neoOriginalBrowserTest.ts')

async function engine() {
  const settings = JSON.parse(
    fs.readFileSync(process.env.NEO_TESTS_ENGINE, 'utf8'),
  )
  const harnessLimitations = []
  let contextBootstrap
  const bookkeeping = {
    configure(options, fixtureOptions) {
      contextBootstrap = contextBootstrapPlan(options, fixtureOptions)
      harnessLimitations.push(...contextBootstrap.limitations)
      if (contextBootstrap.limitations.length)
        throw Object.assign(
          new Error(contextBootstrap.limitations.join('; ')),
          { name: 'HarnessLimitation' },
        )
    },
    limited(operation) {
      const reason = `Owned single-browser fixture does not support ${operation}`
      harnessLimitations.push(reason)
      throw Object.assign(new Error(reason), { name: 'HarnessLimitation' })
    },
    evidence() {
      return { harnessLimitations, contextBootstrap }
    },
  }
  if (settings.kind === 'neo') {
    const native = new NativeProxy(settings, { errorTypes: playwright.errors })
    await native.ready
    native.browser = native.absent('Browser')
    native.browserType = native.absent('BrowserType')
    return {
      ...bookkeeping,
      native,
      playwright: native.playwright,
      page: native.page,
      context: native.context,
      browser: native.browser,
      browserType: native.browserType,
      async finish() {
        return {
          ...bookkeeping.evidence(),
          calls: native.calls,
          roots: native.roots,
          censusGaps: native.censusGaps,
          correlationGaps: native.correlationGaps,
          missing: [...native.missing],
          transportErrors: native.transportErrors,
          capabilities: native.capabilities,
          nativeOutcome: await native.finish(),
        }
      },
    }
  }
  const browser = await playwright.chromium.connectOverCDP(settings.endpoint, {
    timeout: 10_000,
  })
  const context = browser.contexts()[0]
  const page = context.pages()[0]
  if (!page) throw new Error('Reference browser has no owned initial page')
  // CDP attachment does not inherit Playwright Test's context defaults. OwnedBrowser has
  // already established the same viewport in both processes; this real PW bootstrap also
  // updates its client-side viewport metadata before any upstream API is measured.
  await page.setViewportSize({ width: 1280, height: 720 })
  page.setDefaultTimeout(0)
  page.setDefaultNavigationTimeout(0)
  context.setDefaultTimeout(0)
  context.setDefaultNavigationTimeout(0)
  const proxy = new ReferenceProxy({
    connection: browser._connection,
    page,
    playwright,
  })
  // Bootstrap discovery is infrastructure, outside the measured upstream API calls.
  const wrappedPage = proxy.wrap(page)
  const wrappedContext = proxy.wrap(context)
  return {
    ...bookkeeping,
    playwright: proxy.playwright,
    page: wrappedPage,
    context: wrappedContext,
    browser: proxy.wrap(browser),
    browserType: proxy.wrap(playwright.chromium),
    async finish() {
      await browser.close()
      return {
        ...bookkeeping.evidence(),
        calls: proxy.calls,
        roots: proxy.roots,
        censusGaps: proxy.censusGaps,
        correlationGaps: proxy.correlationGaps,
        requirements: [...proxy.requirements],
        missing: [],
        transportErrors: [],
      }
    },
  }
}

export const test = browserTest.extend({
  server: async ({ server }, use) => {
    fs.writeFileSync(
      `${process.env.NEO_TESTS_JOURNAL}.server.json`,
      JSON.stringify({
        address: server._server.address(),
        advertised: server.EMPTY_PAGE,
      }),
    )
    await use(server)
  },
  __engine: [
    async ({}, use) => {
      const instance = await engine()
      try {
        await use(instance)
      } finally {
        const record = await instance.finish()
        fs.writeFileSync(
          process.env.NEO_TESTS_JOURNAL,
          JSON.stringify(record, null, 2),
        )
      }
    },
    { scope: 'worker' },
  ],
  playwright: [
    async ({ __engine }, use) => {
      await use(__engine.playwright)
    },
    { scope: 'worker' },
  ],
  browser: [
    async ({ __engine }, use) => {
      await use(__engine.browser)
    },
    { scope: 'worker' },
  ],
  browserType: [
    async ({ __engine }, use) => {
      await use(__engine.browserType)
    },
    { scope: 'worker' },
  ],
  context: async ({ __engine, _setupContextOptions }, use) => {
    await use(__engine.context)
  },
  page: async ({ __engine, _setupContextOptions }, use) => {
    await use(__engine.page)
  },
  // The preserved upstream contextFactory and pageWithHar depend on this seam. Apply their
  // context defaults and measure creation/cleanup without PW's private artifact machinery.
  _contextFactory: async ({ browser, _combinedContextOptions }, use) => {
    const contexts = []
    await use(async (options) => {
      const context = await browser.newContext({
        ..._combinedContextOptions,
        ...options,
      })
      const close = () => context.close()
      contexts.push(close)
      return { context, close }
    })
    await Promise.all(contexts.map((close) => close()))
  },
  // These inherited fixtures can bypass the engine namespace or launch additional browser
  // processes. Retain their names, but make the ownership boundary explicit on actual use.
  toImplInWorkerScope: [
    async ({ __engine }, use) => {
      await use(() => __engine.limited('toImpl'))
    },
    { scope: 'worker' },
  ],
  launchPersistent: async ({ __engine }, use) => {
    await use(() => __engine.limited('launchPersistent'))
  },
  startRemoteServer: async ({ __engine }, use) => {
    await use(() => __engine.limited('startRemoteServer'))
  },
  // Upstream's auto artifact/launch fixtures mutate its real control namespace. Disable those
  // internals while configuration below validates the original runner's combined options.
  _setupArtifacts: [
    async ({}, use) => {
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _setupContextOptions: [
    async (
      {
        __engine,
        _combinedContextOptions,
        actionTimeout,
        navigationTimeout,
        testIdAttribute,
      },
      use,
    ) => {
      __engine.configure(_combinedContextOptions, {
        actionTimeout,
        navigationTimeout,
        testIdAttribute,
      })
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
  browserVersion: [
    async ({}, use) => {
      const settings = JSON.parse(
        fs.readFileSync(process.env.NEO_TESTS_ENGINE, 'utf8'),
      )
      await use(settings.version?.product?.split('/').at(-1) || '0')
    },
    { scope: 'worker' },
  ],
  browserMajorVersion: [
    async ({ browserVersion }, use) => {
      await use(Number(browserVersion.split('.')[0]))
    },
    { scope: 'worker' },
  ],
  isAndroid: [false, { scope: 'worker' }],
  isElectron: [false, { scope: 'worker' }],
  isHeadlessShell: [false, { scope: 'worker' }],
  isFrozenWebkit: [false, { scope: 'worker' }],
  isBidi: [false, { scope: 'worker' }],
  electronMajorVersion: [0, { scope: 'worker' }],
  allowsThirdParty: [false, { scope: 'worker' }],
  defaultSameSiteCookieValue: ['Lax', { scope: 'worker' }],
})
