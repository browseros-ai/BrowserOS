/** Fixture behavior uses the committed v1 prelude over the project's fake string-native door. */
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  loadPrelude,
  SEED,
} from '../../neo-playwright-js/tests/support/prelude-runtime.ts'
import {
  clawDefaults,
  createClawContextFactory,
} from './claw-context-options.mjs'
import { resetClawTest } from './claw-harness.mjs'

async function fixture(
  combined = clawDefaults,
  implicit = new Set(Object.keys(clawDefaults)),
) {
  let nextPage = 8
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'context.newPage')
        return {
          page: { $neo: 'page', id: nextPage, tab: 1000 + nextPage++ },
          url: 'about:blank',
        }
      if (name === 'page.frames') return { frames: [] }
      return null
    },
  })
  await runtime.ready
  const { browser, context } = runtime.evaluate('({ browser, context })')
  const testInfo = { annotations: [], outputPath: (name) => `/unused/${name}` }
  const factory = createClawContextFactory({
    browser,
    context,
    combined,
    implicit,
    video: 'off',
    testInfo,
  })
  return { runtime, browser, context, testInfo, factory }
}

test('tests that the first implicit context is the real shared v1 object and close leaves it alive', async () => {
  const { factory, context, runtime } = await fixture()
  const borrowed = await factory()
  assert.equal(borrowed.context, context)
  await borrowed.close()
  assert.equal(context.isClosed(), false)
  assert.equal(runtime.sent('browser.newContext').length, 0)
  assert.equal(runtime.sent('browserContext.close').length, 0)
})

test('tests that explicit requested and test-use defaults go through the real v1 refusal', async () => {
  for (const [combined, implicit, requested] of [
    [
      clawDefaults,
      new Set(Object.keys(clawDefaults)),
      { viewport: { width: 1280, height: 720 } },
    ],
    [
      clawDefaults,
      new Set(
        Object.keys(clawDefaults).filter((key) => key !== 'acceptDownloads'),
      ),
      undefined,
    ],
    [
      { ...clawDefaults, locale: 'fr-FR' },
      new Set(Object.keys(clawDefaults)),
      undefined,
    ],
  ]) {
    const { factory, runtime, testInfo } = await fixture(combined, implicit)
    await assert.rejects(
      factory(requested),
      /browser.newContext: Neo has one browser profile/,
    )
    const [call] = runtime.sent('browser.newContext')
    assert.equal(call.input.$neo, 'refused')
    assert.equal(call.input.code, 'method_unavailable')
    assert.match(testInfo.annotations[0].description, /^explicit options:/)
  }
})

test('tests that a second context request refuses even after the borrowed context is released', async () => {
  const { factory, runtime, testInfo } = await fixture()
  await (await factory()).close()
  await assert.rejects(factory(), /Neo has one browser profile/)
  assert.equal(runtime.sent('browser.newContext')[0].input.$neo, 'refused')
  assert.deepEqual(testInfo.annotations, [
    { type: 'claw-context', description: 'second context in one test' },
  ])
})

test('tests that teardown closes real v1 pages and removes real listeners before clearing storage', async () => {
  const { context, runtime } = await fixture()
  const initialPages = new Set(context.pages())
  const first = await context.newPage()
  const second = await context.newPage()
  context.on('claw-local', () => {})
  const origins = ['http://localhost:12345', 'https://localhost:12346']
  const cleared = []
  await resetClawTest({
    context,
    initialPages,
    origins,
    cdp: {
      async send(method, params) {
        assert.equal(first.isClosed(), true)
        assert.equal(second.isClosed(), true)
        assert.equal(context.listenerCount('claw-local'), 0)
        cleared.push({ method, params })
      },
    },
  })
  assert.equal(runtime.sent('page.close').length, 2)
  assert.ok([...initialPages].every((page) => !page.isClosed()))
  assert.equal(cleared[0].method, 'Storage.clearCookies')
  assert.deepEqual(
    cleared.slice(1).map((call) => call.params.origin),
    origins,
  )
})
