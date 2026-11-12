// Public Read values and the exact argument strings sent through the bootstrap. The native
// route tests cover decoder defaults and result bytes; these pin the client command split.
import { expect, it } from 'bun:test'
import {
  json,
  loadPrelude,
  SEED,
  type WireExchange,
} from './support/prelude-runtime'

const ELEMENT = {
  $neo: 'element',
  page: { $neo: 'page', tab: 1007 },
  seq: 4,
  node: 9,
  display: '<button>Go</button>',
}

const TARGET = {
  $neo: 'locator',
  page: { $neo: 'page', tab: 1007 },
  frame: null,
  chunks: [
    {
      parts: [
        {
          name: 'css',
          body: [
            {
              simples: [
                {
                  selector: { css: '#go', functions: [] },
                  combinator: '',
                },
              ],
            },
          ],
          source: '#go',
        },
      ],
    },
  ],
  display: "locator('#go')",
}

it('sends includeShadow on page and frame content without changing the default request', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => '<html></html>' })
  expect(
    json(
      await runtime.run(`return [
    await page.content(),
    await page.content({ includeShadow: true }),
    await page.mainFrame().content({ includeShadow: false }),
  ]`),
    ),
  ).toEqual(['<html></html>', '<html></html>', '<html></html>'])
  expect(runtime.sent('page.content').map(({ input }) => input)).toEqual([
    { call: { api: 'page.content', timeoutMs: 0 }, page: ELEMENT.page },
    {
      call: { api: 'page.content', timeoutMs: 0 },
      page: ELEMENT.page,
      includeShadow: true,
    },
    {
      call: { api: 'frame.content', timeoutMs: 0 },
      page: ELEMENT.page,
      includeShadow: false,
    },
  ])
})

it('rejects the obsolete waitFor key before starting a page or frame selector wait', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => ELEMENT })
  expect(
    json(
      await runtime.run(`return await Promise.all([
    page.waitForSelector('*', { waitFor: 'attached' }),
    page.mainFrame().waitForSelector('*', { waitFor: 'attached' }),
  ].map(wait => wait.catch(error => ({ name: error.name, message: error.message }))))`),
    ),
  ).toEqual([
    {
      name: 'Error',
      message:
        'page.waitForSelector: options.waitFor is not supported, did you mean options.state?',
    },
    {
      name: 'Error',
      message:
        'frame.waitForSelector: options.waitFor is not supported, did you mean options.state?',
    },
  ])
  expect(runtime.sent('locator.waitFor')).toEqual([])
})

it('prefixes the vendored element preview without changing its handle brand', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => ELEMENT })
  expect(
    json(
      await runtime.run(`const handle = await page.$('button');
    return [String(handle), handle.toJSON()]`),
    ),
  ).toEqual(['JSHandle@<button>Go</button>', ELEMENT])
})

it('preserves the legacy visible and falsy waitFor values upstream tolerates', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => ELEMENT })
  expect(
    json(
      await runtime.run(`return await Promise.all(
    ['visible', undefined, null, false, 0, ''].map(async waitFor =>
      !!await page.waitForSelector('*', { waitFor })))`),
    ),
  ).toEqual([true, true, true, true, true, true])
  expect(runtime.sent('locator.waitFor')).toHaveLength(6)
  expect(
    runtime.sent('locator.waitFor').some(({ input }) => 'waitFor' in input),
  ).toBe(false)
})

it('keeps immediate queries immediate and sends locator elementHandle through the attached wait', async () => {
  const wire: WireExchange[] = []
  const runtime = loadPrelude({
    seed: SEED,
    onWire: (exchange) => wire.push(exchange),
    answer: ({ name }) =>
      name === 'locator.elementHandle' || name === 'locator.waitFor'
        ? ELEMENT
        : null,
  })
  const result = json(
    await runtime.run(`
    const a = await page.$('#go')
    const b = await page.mainFrame().$('#go')
    const c = await page.locator('#go').elementHandle({ timeout: 42 })
    const d = await a.$('#go')
    return [!!a, !!b, !!c, !!d]
  `),
  )
  expect(result).toEqual([true, true, true, true])
  expect(
    wire
      .filter(({ op }) => op === 'locator.elementHandle')
      .map(({ args }) => args),
  ).toEqual([
    JSON.stringify({
      call: { api: 'page.$', timeoutMs: 0 },
      target: TARGET,
      strict: 'firstMatch',
    }),
    JSON.stringify({
      call: { api: 'frame.$', timeoutMs: 0 },
      target: TARGET,
      strict: 'firstMatch',
    }),
    JSON.stringify({
      call: { api: 'elementHandle.$', timeoutMs: 0 },
      target: { ...TARGET, root: ELEMENT },
      strict: 'firstMatch',
    }),
  ])
  expect(
    wire.filter(({ op }) => op === 'locator.waitFor').map(({ args }) => args),
  ).toEqual([
    JSON.stringify({
      call: { api: 'locator.elementHandle', timeoutMs: 42 },
      target: TARGET,
      strict: 'strict',
      state: 'attached',
      omitReturnValue: false,
    }),
  ])
})

it('keeps explicit selector-wait fields and reads the renamed normalize reply', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'locator.waitFor') return ELEMENT
      if (name === 'locator.normalize')
        return { resolvedSelector: 'css=#resolved' }
      return name === 'locator.count' ? 1 : null
    },
  })
  const result = json(
    await runtime.run(`
    await page.waitForSelector('#go')
    await page.locator('#go').waitFor()
    const normalized = await page.locator('#go').normalize()
    return await normalized.count()
  `),
  )
  expect(result).toBe(1)
  expect(runtime.sent('locator.waitFor').map(({ input }) => input)).toEqual([
    {
      call: { api: 'page.waitForSelector', timeoutMs: 5000 },
      target: TARGET,
      strict: 'firstMatch',
      state: 'visible',
      omitReturnValue: false,
    },
    {
      call: { api: 'locator.waitFor', timeoutMs: 5000 },
      target: TARGET,
      strict: 'strict',
      state: 'visible',
      omitReturnValue: true,
    },
  ])
  expect(runtime.sent('locator.count')[0]?.input.target).toMatchObject({
    chunks: [{ parts: [{ name: 'css', source: '#resolved' }] }],
  })
})
