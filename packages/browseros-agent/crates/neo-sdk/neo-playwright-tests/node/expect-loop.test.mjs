import assert from 'node:assert/strict'
import test from 'node:test'
import { createExpect, expectLoop } from './expect-loop.mjs'

test('tests that the table uses public boolean, text, attribute and page reads', async () => {
  const element = {
    textContent: '',
    indeterminate: true,
    detail: { nested: [1, 2] },
    selectedOptions: [{ value: 'a' }, { value: 'b' }],
    getRootNode: () => ({ activeElement: element }),
    ownerDocument: {
      hasFocus: () => true,
      defaultView: {
        getComputedStyle: () => ({ getPropertyValue: () => 'red' }),
      },
    },
  }
  const locator = {
    isVisible: async () => true,
    isHidden: async () => false,
    isEnabled: async () => true,
    isDisabled: async () => false,
    isChecked: async () => true,
    isEditable: async () => true,
    count: async () => 1,
    textContent: async () => ' hello\n world ',
    innerText: async () => 'rendered',
    allTextContents: async () => ['first', 'middle', 'last'],
    allInnerTexts: async () => ['rendered'],
    inputValue: async () => 'value',
    getAttribute: async (name) =>
      ({ id: 'node', class: 'one two', present: '' })[name] ?? null,
    evaluate: async (fn, arg) => fn(element, arg),
    evaluateAll: async (fn, arg) =>
      fn([{ getAttribute: () => 'one two' }], arg),
  }
  const expect = expectLoop(locator)
  await expect.toBeVisible()
  await expect.not.toBeHidden()
  await expect.toBeEnabled()
  await expect.not.toBeDisabled()
  await expect.toBeChecked()
  await expect.toBeChecked({ indeterminate: true })
  await expect.toBeEditable()
  await expect.toBeFocused()
  await expect.toBeAttached()
  await expect.toBeEmpty()
  await expect.toHaveText('hello world')
  await expect.toHaveText('rendered', { useInnerText: true })
  await expect.toHaveText(['first', 'middle', 'last'])
  await expect.toHaveText(['rendered'], { useInnerText: true })
  await expect.toContainText(['first', 'last'])
  await expect.toContainText(/WORLD/i)
  await expect.toHaveValue('value')
  await expect.toHaveValues(['a', /b/])
  await expect.toHaveAttribute('id', /NODE/, { ignoreCase: true })
  await expect.toHaveAttribute('present')
  await expect.not.toHaveAttribute('absent', { timeout: 10 })
  await expect.not.toHaveAttribute('absent', /.*/)
  await expect.toHaveClass('one two')
  await expect.toHaveClass([/one/])
  await expect.toHaveCount(1)
  await expect.toHaveCSS('color', 'red')
  await expect.toHaveId('node')
  await expect.toHaveJSProperty('detail', { nested: [1, 2] })
  const page = expectLoop({
    url: () => 'https://example.test/path',
    title: async () => ' A\n title ',
  })
  await page.toHaveURL('https://example.test/path')
  await page.toHaveURL((url) => url.pathname === '/path')
  await page.toHaveURL(/example\.test/)
  await page.toHaveTitle('A title')
})

test('tests that polling waits for a changing read and records one use', async () => {
  let value = 'before'
  const annotations = []
  const timer = setTimeout(() => {
    value = 'after'
  }, 120)
  const started = performance.now()
  try {
    await expectLoop(
      { textContent: async () => value },
      { info: () => ({ annotations }) },
    ).toHaveText('after', { timeout: 1000 })
  } finally {
    clearTimeout(timer)
  }
  assert.ok(performance.now() - started >= 120)
  assert.deepEqual(annotations, [
    { type: 'expectLoop', description: 'toHaveText' },
  ])
})

test('tests that failed and negated checks report expected and last received values', async () => {
  await assert.rejects(
    expectLoop({ inputValue: async () => 'actual' }).toHaveValue('wanted', {
      timeout: 10,
    }),
    (error) => {
      assert.match(error.message, /^expectLoop: toHaveValue/)
      assert.match(error.message, /wanted/)
      assert.match(error.message, /actual/)
      assert.equal(error.name, 'AssertionError')
      return true
    },
  )
  await assert.rejects(
    expectLoop({ count: async () => 1 }).not.toHaveCount(1, { timeout: 10 }),
    /expectLoop: not\.toHaveCount/,
  )
  await expectLoop({ isVisible: async () => false }).toBeVisible({
    visible: false,
  })
})

test('tests that a read error cannot pass negation and missing methods retain their cause', async () => {
  await assert.rejects(
    expectLoop({
      inputValue: async () => {
        throw new Error('detached')
      },
    }).not.toHaveValue('wanted', { timeout: 10 }),
    /expectLoop: not\.toHaveValue.*read failed: detached/,
  )
  class Locator {}
  await assert.rejects(
    expectLoop(new Locator()).toHaveValue('wanted', { timeout: 10 }),
    /expectLoop: toHaveValue.*read failed: Locator\.inputValue is not a function/,
  )
})

test('tests that the first read gets the one-shot floor and later reads end at the deadline', async () => {
  const started = performance.now()
  await assert.rejects(
    expectLoop({ title: () => new Promise(() => {}) }).toHaveTitle('x', {
      timeout: 20,
    }),
    /^AssertionError.*expectLoop: toHaveTitle/,
  )
  const hung = performance.now() - started
  assert.ok(
    hung >= 900 && hung < 1500,
    `a hung read ends at the floor: ${hung}ms`,
  )
  const options = []
  await expectLoop({
    inputValue: async (value) => {
      options.push(value.timeout)
      return options.length === 1 ? 'y' : 'x'
    },
  }).toHaveValue('x', { timeout: 1500 })
  const [one, ...later] = options
  assert.ok(one > 100 && one <= 1000, `the one-shot gets the floor: ${one}`)
  assert.ok(
    later.length > 0 && later.every((timeout) => timeout > 0 && timeout <= 100),
    `${later}`,
  )
})

test('tests that a later read stops at the deadline, without a floor of its own', async () => {
  let reads = 0
  const started = performance.now()
  await assert.rejects(
    expectLoop({
      title: () => (++reads === 1 ? 'y' : new Promise(() => {})),
    }).toHaveTitle('x', { timeout: 300 }),
    /expectLoop: toHaveTitle/,
  )
  const took = performance.now() - started
  assert.equal(reads, 2)
  assert.ok(
    took >= 250 && took < 800,
    `the second read ends at the deadline: ${took}ms`,
  )
})

// Upstream's expect-boolean "with impossible timeout" tests (EX-047, EX-083, EX-084): one check
// always runs, so a read slower than the timeout still passes.
test('tests that a read slower than an impossible timeout still passes', async () => {
  await expectLoop({
    isVisible: () => new Promise((done) => setTimeout(() => done(true), 30)),
  }).toBeVisible({ timeout: 1 })
  await expectLoop({
    count: () => new Promise((done) => setTimeout(() => done(0), 30)),
  }).not.toBeAttached({ timeout: 1 })
  // A read that enforces its own timeout, as Neo's do, and needs longer than a polling step.
  await expectLoop({
    inputValue: ({ timeout }) =>
      new Promise((done, fail) => {
        if (timeout < 150) fail(new Error(`timed out after ${timeout}ms`))
        else setTimeout(() => done('x'), 150)
      }),
  }).toHaveValue('x', { timeout: 1 })
})

test('tests that the default timeout is five seconds and zero permits waiting', async () => {
  const started = performance.now()
  await assert.rejects(
    expectLoop({ count: async () => 0 }).toHaveCount(1),
    /expectLoop: toHaveCount/,
  )
  assert.ok(performance.now() - started >= 4900)
  assert.ok(performance.now() - started < 8000)
  let visible = true
  const timer = setTimeout(() => {
    visible = false
  }, 20)
  try {
    await expectLoop({ isVisible: async () => visible }).not.toBeVisible({
      timeout: 0,
    })
  } finally {
    clearTimeout(timer)
  }
})

test('tests that unknown checks skip immediately with the uncovered reason', async () => {
  const annotations = []
  const reasons = []
  for (const name of [
    'toMatchAriaSnapshot',
    'toHaveAccessibleName',
    'toString',
  ]) {
    await assert.rejects(
      expectLoop(
        {},
        {
          info: () => ({
            annotations,
            skip: (condition, reason) => {
              if (condition) reasons.push(reason)
            },
          }),
        },
      )[name](),
      new RegExp(`expectLoop: ${name} is not covered`),
    )
  }
  assert.deepEqual(reasons, [
    'expectLoop: toMatchAriaSnapshot is not covered',
    'expectLoop: toHaveAccessibleName is not covered',
    'expectLoop: toString is not covered',
  ])
})

test('tests that strings normalize whitespace while regexes and values preserve it', async () => {
  const expect = expectLoop({
    textContent: async () => 'a\n b',
    inputValue: async () => ' a ',
  })
  await expect.toHaveText('a b')
  await expect.toHaveText(/a\n b/g)
  await expect.toHaveText(/a\n b/g)
  await expect.toHaveValue(' a ')
  await assert.rejects(expect.toHaveText(/a b/, { timeout: 5 }), /expectLoop:/)
  await assert.rejects(expect.toHaveValue('a', { timeout: 5 }), /expectLoop:/)
  await assert.rejects(
    expectLoop({
      allTextContents: async () => ['last', 'first'],
    }).toContainText(['first', 'last'], { timeout: 5 }),
    /expectLoop:/,
  )
})

test('tests that the wrapper routes Neo instances and preserves real expect and its statics', async () => {
  class Page {
    title() {
      return Promise.resolve('Neo')
    }
  }
  const real = Object.assign((...args) => ({ original: args }), {
    poll: () => 'poll',
    soft: () => 'soft',
    configure: () => 'configure',
    extend: () => 'extend',
  })
  const wrapped = createExpect(real, {
    isNeo: (value) => value instanceof Page,
  })
  await wrapped(new Page()).toHaveTitle('Neo')
  assert.deepEqual(wrapped('value', 'message'), {
    original: ['value', 'message'],
  })
  for (const name of ['poll', 'soft', 'configure', 'extend'])
    assert.equal(wrapped[name], real[name])
  assert.equal(wrapped({ constructor: Page }).original.length, 1)
})
