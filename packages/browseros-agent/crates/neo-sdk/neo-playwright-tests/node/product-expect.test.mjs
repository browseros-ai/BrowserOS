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
            return 'product'
          },
      }),
    },
    { timeout: 10000 },
  )
  const configured = wrapped.configure({ timeout: 17 }).extend({})
  assert.equal(configured(subject), 'product')
  assert.deepEqual(calls[0], { options: { timeout: 17 }, args: [subject] })
  assert.deepEqual(configured('value', 'message'), ['value', 'message'])
  assert.deepEqual(wrapped.soft('value'), ['soft', 'value'])
  assert.equal(wrapped.soft(subject), 'product')
  assert.deepEqual(calls[1].options, { timeout: 10000, soft: true })
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
