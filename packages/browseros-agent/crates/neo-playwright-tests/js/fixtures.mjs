// biome-ignore-all lint/correctness/noEmptyPattern: Playwright parses fixture dependencies from this required parameter syntax.
/** Upstream fixtures own test semantics; this seam supplies only engine-backed browser objects. */
import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { source } from './prepare.mjs'
import { NativeProxy, ReferenceProxy } from './proxy.mjs'

const require = createRequire(path.join(source, 'package.json'))
const { expect } = require('@playwright/test')

export { expect }

// Playwright transforms this upstream TypeScript module in both collection and workers.
const overlayRequire = createRequire(
  path.join(process.env.NEO_TESTS_OVERLAY, 'tests/config/browserTest.ts'),
)
const { baseTest } = overlayRequire('./baseTest.ts')

async function engine() {
  const settings = JSON.parse(
    fs.readFileSync(process.env.NEO_TESTS_ENGINE, 'utf8'),
  )
  if (settings.kind === 'neo') {
    const native = new NativeProxy(settings)
    await native.ready
    native.browser = native.absent('Browser')
    native.browserType = native.absent('BrowserType')
    return {
      ...native,
      native,
      page: native.page,
      context: native.context,
      browser: native.browser,
      browserType: native.browserType,
      async finish() {
        return {
          calls: native.calls,
          missing: [...native.missing],
          transportErrors: native.transportErrors,
          capabilities: native.capabilities,
          nativeOutcome: await native.finish(),
        }
      },
    }
  }
  const playwright = require('playwright-core')
  const browser = await playwright.chromium.connectOverCDP(settings.endpoint, {
    timeout: 10_000,
  })
  const context = browser.contexts()[0]
  const page = context.pages()[0]
  if (!page) throw new Error('Reference browser has no owned initial page')
  const proxy = new ReferenceProxy({ connection: browser._connection, page })
  // Bootstrap discovery is infrastructure, outside the measured upstream API calls.
  const wrappedPage = proxy.wrap(page)
  const wrappedContext = proxy.wrap(context)
  return {
    page: wrappedPage,
    context: wrappedContext,
    browser: proxy.wrap(browser),
    browserType: proxy.wrap(playwright.chromium),
    async finish() {
      await browser.close()
      return {
        calls: proxy.calls,
        requirements: [...proxy.requirements],
        missing: [],
        transportErrors: [],
      }
    },
  }
}

export const test = baseTest.extend({
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
  context: async ({ __engine }, use) => {
    await use(__engine.context)
  },
  page: async ({ __engine }, use) => {
    await use(__engine.page)
  },
  contextFactory: async ({ browser }, use) => {
    await use((options) => browser.newContext(options))
  },
  launchPersistent: async ({ browserType, createUserDataDir }, use) => {
    await use(async (options) => {
      const context = await browserType.launchPersistentContext(
        await createUserDataDir(),
        options,
      )
      return { context, page: context.pages()[0] }
    })
  },
  createUserDataDir: async ({}, use, testInfo) => {
    await use(async () => {
      const dir = testInfo.outputPath('persistent')
      fs.mkdirSync(dir, { recursive: true })
      return dir
    })
  },
  // Trace recording and context mutation belong to upstream's default context factory.
  // They cannot run against the Neo proxy. Test-requested APIs still go to native reflection.
  _setupArtifacts: [
    async ({}, use) => {
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
  _setupContextOptions: [
    async ({}, use) => {
      await use()
    },
    { auto: 'all-hooks-included' },
  ],
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
