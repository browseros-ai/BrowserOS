import assert from 'node:assert/strict'
import test from 'node:test'
import { productExpect } from './product-expect.mjs'

test('the run owns options, negation, promise modifiers and failure identity', async () => {
  const subject = {}
  const options = {
    timeout: 23,
    signal: new AbortController().signal,
    ratio: 0.5,
  }
  const failure = Object.assign(new Error('product failure'), {
    matcherResult: { actual: 'product result' },
  })
  const calls = []
  const annotations = []
  let active
  const wrapped = productExpect(
    () => assert.fail('must use the run'),
    {
      isNeo: (value) => value === subject,
      provider: () => active,
      info: () => ({ annotations }),
    },
    { timeout: 10000 },
  )
  active = {
    configure(config) {
      assert.deepEqual(config, { timeout: 10000 })
      return (actual, message) => {
        assert.equal(actual, subject)
        assert.equal(message, 'custom message')
        return {
          not: {
            rejects: {
              toBeInViewport: async (...args) => {
                calls.push(args)
                throw failure
              },
            },
          },
        }
      }
    },
  }
  await assert.rejects(
    wrapped(subject, 'custom message').not.rejects.toBeInViewport(options),
    (error) => error === failure,
  )
  assert.equal(calls[0][0], options)
  assert.deepEqual(annotations, [{ type: 'neoExpect', description: 'product' }])
})

test('configured and extended imports keep the provider while values keep Playwright', () => {
  const subject = {}
  const calls = []
  function original(...args) {
    return args
  }
  original.configure = () => original
  original.extend = () => original
  original.soft = (...args) => ['soft', ...args]
  original.poll = () => 'poll'
  const wrapped = productExpect(
    original,
    {
      isNeo: (value) => value === subject,
      info: () => undefined,
      provider: () => ({
        configure:
          (options) =>
          (...args) => {
            calls.push({ options, args })
            return { name: 'product' }
          },
      }),
    },
    { timeout: 10000 },
  )
  const configured = wrapped.configure({ timeout: 17 }).extend({})
  assert.equal(configured(subject).name, 'product')
  assert.deepEqual(calls[0], { options: { timeout: 17 }, args: [subject] })
  assert.deepEqual(configured('value', 'message'), ['value', 'message'])
  assert.deepEqual(wrapped.soft('value'), ['soft', 'value'])
  // Neo's expect never sees `soft`: the runner owns it (see the soft tests below).
  assert.equal(wrapped.soft(subject).name, 'product')
  assert.deepEqual(calls[1].options, { timeout: 10000 })
  assert.equal(wrapped.poll, original.poll)
})

test('unsupported product matchers fail without skipping or synthesizing results', () => {
  const failure = new Error('method unavailable')
  const wrapped = productExpect(() => undefined, {
    isNeo: () => true,
    info: () => ({ annotations: [], skip: () => assert.fail('must not skip') }),
    provider: () => ({
      configure: () => () => ({
        toMatchAriaSnapshot() {
          throw failure
        },
      }),
    }),
  })
  assert.throws(
    () => wrapped({}).toMatchAriaSnapshot('- button'),
    (error) => error === failure,
  )
})

test('visual assertions keep runner options and chains while web assertions stay in Neo', async () => {
  const subject = {},
    visual = {},
    annotations = [],
    calls = []
  const options = { timeout: 17, maxDiffPixels: 3 }
  function original(actual, message) {
    assert.equal(actual, visual)
    assert.equal(message, 'visual message')
    return { not: { toHaveScreenshot: async (...args) => calls.push(args) } }
  }
  original.configure = (config) => {
    assert.deepEqual(config, { timeout: 17 })
    return original
  }
  const product = {
    not: { toHaveScreenshot: () => assert.fail('Neo has no test baselines') },
    toBeVisible: () => 'product',
  }
  const wrapped = productExpect(original, {
    isNeo: (value) => value === subject,
    provider: () => ({ configure: () => () => product }),
    info: () => ({ annotations }),
    visual: (value) => {
      assert.equal(value, subject)
      return visual
    },
  }).configure({ timeout: 17 })
  await wrapped(subject, 'visual message').not.toHaveScreenshot(
    'expected.png',
    options,
  )
  assert.deepEqual(calls, [['expected.png', options]])
  assert.equal(annotations[0].description, 'playwright-visual/neo-capture')
  assert.equal(wrapped(subject).toBeVisible(), 'product')
  assert.equal(annotations[1].description, 'product')
})

test('both soft forms route visual assertions before configuring the production Neo provider', async () => {
  const { loadPrelude, SEED } = await import(
    '../../neo-playwright-js/tests/support/prelude-runtime.ts'
  )
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.frames' ? { frames: [] } : null),
  })
  await runtime.ready
  const page = runtime.evaluate('page'),
    neo = runtime.evaluate('expect')
  let calls = 0
  const original = () => ({
    toHaveScreenshot: () => {
      calls++
    },
  })
  original.configure = () => original
  original.soft = original
  const wrapped = productExpect(original, {
    isNeo: (value) => value === page,
    provider: () => neo,
    info: () => undefined,
    visual: (value) => value,
  })
  wrapped.soft(page).toHaveScreenshot('expected.png')
  wrapped.configure({ soft: true })(page).toHaveScreenshot('expected.png')
  assert.equal(calls, 2)
})

test('soft Neo assertions run hard in Neo and record a failure on the test without throwing', async () => {
  const subject = {}
  const recorded = []
  const configs = []
  const rejected = new Error('async failure')
  const thrown = new Error('sync failure')
  const testInfo = {
    annotations: [],
    _failWithError: (error) => recorded.push(error),
  }
  const original = () => assert.fail('a Neo receiver never reaches Playwright')
  original.configure = () => original
  original.soft = original
  const wrapped = productExpect(original, {
    isNeo: (value) => value === subject,
    info: () => testInfo,
    provider: () => ({
      configure(config) {
        configs.push(config)
        return () => ({
          toBeVisible: async () => {
            throw rejected
          },
          toHaveTitle: async () => 'passed',
          get not() {
            return {
              toBeChecked: () => {
                throw thrown
              },
            }
          },
        })
      },
    }),
  })
  assert.equal(await wrapped.soft(subject).toBeVisible(), undefined)
  assert.equal(await wrapped.soft(subject).toHaveTitle('x'), undefined)
  assert.equal(
    wrapped.configure({ soft: true, timeout: 7 })(subject).not.toBeChecked(),
    undefined,
  )
  await assert.rejects(
    wrapped(subject).toBeVisible(),
    (error) => error === rejected,
  )
  assert.deepEqual(recorded, [rejected, thrown])
  assert.deepEqual(configs, [
    { timeout: 10000 },
    { timeout: 10000 },
    { timeout: 7 },
    { timeout: 10000 },
  ])
  assert.deepEqual(
    testInfo.annotations.map((annotation) => annotation.description),
    ['product', 'product', 'product', 'product'],
  )
})

test('a soft failure with no test to record on is thrown, never dropped', async () => {
  const failure = new Error('product failure')
  const original = () => undefined
  original.soft = original
  const wrapped = productExpect(original, {
    isNeo: () => true,
    info: () => undefined,
    provider: () => ({
      configure: () => () => ({
        toBeVisible: async () => {
          throw failure
        },
      }),
    }),
  })
  await assert.rejects(
    wrapped.soft({}).toBeVisible(),
    (error) => error === failure,
  )
})

test("a spec's own extend matchers run in Playwright on the Neo receiver; built-ins stay in Neo", async () => {
  const subject = {}
  const annotations = []
  const ran = []
  const neo = { toBeVisible: () => 'neo', toHaveText: () => 'neo' }
  function original(...args) {
    return original.matchers(...args)
  }
  original.matchers = () => assert.fail('the plain import has no spec matchers')
  original.configure = () => original
  original.extend = (matchers) => {
    const extended = (actual, message) => ({
      toMatchMedia: (query) => {
        ran.push({ actual, message, query, isNot: false })
        return matchers.toMatchMedia(actual, query)
      },
      not: {
        toMatchMedia: (query) => {
          ran.push({ actual, message, query, isNot: true })
          return 'negated'
        },
      },
    })
    extended.configure = () => extended
    extended.extend = original.extend
    return extended
  }
  const wrapped = productExpect(original, {
    isNeo: (value) => value === subject,
    info: () => ({ annotations }),
    provider: () => ({ configure: () => () => neo }),
  })
  const expect = wrapped.extend({
    toMatchMedia: (_page, query) => `ran ${query}`,
  })
  assert.equal(
    expect(subject, 'msg').toMatchMedia('(hover: hover)'),
    'ran (hover: hover)',
  )
  assert.equal(expect(subject).not.toMatchMedia('(pointer: coarse)'), 'negated')
  assert.equal(expect(subject).toBeVisible(), 'neo')
  assert.equal(
    expect.configure({ timeout: 5 })(subject).toMatchMedia('q'),
    'ran q',
  )
  assert.equal(wrapped(subject).toHaveText('x'), 'neo')
  assert.deepEqual(ran, [
    { actual: subject, message: 'msg', query: '(hover: hover)', isNot: false },
    {
      actual: subject,
      message: undefined,
      query: '(pointer: coarse)',
      isNot: true,
    },
    { actual: subject, message: undefined, query: 'q', isNot: false },
  ])
  assert.deepEqual(
    annotations.map((annotation) => annotation.description),
    [
      'playwright-custom/neo-calls',
      'playwright-custom/neo-calls',
      'product',
      'playwright-custom/neo-calls',
      'product',
    ],
  )
})
