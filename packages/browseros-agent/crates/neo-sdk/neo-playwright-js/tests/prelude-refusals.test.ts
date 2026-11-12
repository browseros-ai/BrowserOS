// The prelude's refusals as door calls (FR/RC, sdk-skeleton's S3): every method, option or
// argument the prelude refuses before any op reaches the door as a refusal under the method the
// script called, which the door records and answers. The script's error is the bootstrap's own
// NeoError, named as Playwright names it, with Playwright's message, so an uncaught one settles
// the run as that op's failure and keeps its code; its door fields arrive after the script next
// yields to Rust. A bootstrap without `__neo.refuse` gets the errors the prelude threw before.
import { expect, it } from 'bun:test'
import { json, loadPrelude, refusalCall, SEED } from './support/prelude-runtime'

const ROUTE = 'page.route: network interception is not available in Neo'

/** A tab with one child frame, which the run reads before its first statement (FR-1). */
const WITH_CHILD = {
  frames: [
    { id: 'main', parentId: null, name: '', url: 'https://a.example/' },
    { id: 'child', parentId: 'main', name: 'child', url: 'https://b.example/' },
  ],
}

const SAME_FRAME = 'Locators must belong to the same frame.'

/**
 * The checks upstream's client makes of a call's arguments before any channel call, each with
 * the method the script called, the name and message upstream throws, and the script that trips
 * it. `child` is a locator in the tab's child frame.
 */
const ARGUMENT_CHECKS: [
  api: string,
  name: string,
  message: string,
  script: string,
][] = [
  [
    'page.frame',
    'Error',
    'Either name or url matcher should be specified',
    'page.frame({})',
  ],
  ['locator.locator', 'Error', SAME_FRAME, "page.locator('a').locator(child)"],
  ['locator.and', 'Error', SAME_FRAME, "page.locator('a').and(child)"],
  ['locator.or', 'Error', SAME_FRAME, "page.locator('a').or(child)"],
  [
    'frameLocator.locator',
    'Error',
    SAME_FRAME,
    "page.frameLocator('iframe').locator(child)",
  ],
  [
    'frameLocator.first',
    'Error',
    'Selecting the nth frame is not allowed on frameLocator().',
    'page.frameLocator().first()',
  ],
  [
    'page.locator',
    'Error',
    'Inner "has" locator must belong to the same frame.',
    "page.locator('a', { has: child })",
  ],
  [
    'frame.locator',
    'Error',
    'Inner "hasNot" locator must belong to the same frame.',
    "page.mainFrame().locator('a', { hasNot: child })",
  ],
  [
    'locator.filter',
    'Error',
    'Inner "has" locator must belong to the same frame.',
    "page.locator('a').filter({ has: child })",
  ],
  [
    'locator.locator',
    'Error',
    'Inner "has" locator must belong to the same frame.',
    "page.locator('a').locator('b', { has: child })",
  ],
  [
    'frameLocator.locator',
    'Error',
    'Inner "hasNot" locator must belong to the same frame.',
    "page.frameLocator('iframe').locator('b', { hasNot: child })",
  ],
  [
    'expect.toHaveRole',
    'Error',
    '"role" argument in toHaveRole must be a string',
    "expect(page.locator('a')).toHaveRole(5)",
  ],
  [
    'expect.toHaveText',
    'Error',
    'expected value must be a string or regular expression',
    "expect(page.locator('a')).toHaveText(5)",
  ],
  [
    'expect.toContainClass',
    'Error',
    '"expected" argument in toContainClass cannot contain RegExp values',
    "expect(page.locator('a')).toContainClass([/x/])",
  ],
  [
    'expect.toContainClass',
    'Error',
    '"expected" argument in toContainClass cannot be a RegExp value',
    "expect(page.locator('a')).toContainClass(/x/)",
  ],
  [
    'expect.toHaveText',
    'Error',
    'toHaveText can be only used with Locator object, was called with 5',
    "expect(5).toHaveText('x')",
  ],
  [
    'expect.toHaveURL',
    'Error',
    'toHaveURL can be only used with Page object, was called with 5',
    'expect(5).toHaveURL(() => true)',
  ],
  [
    'expect.toPass',
    'Error',
    'toPass: expected a function',
    'expect(5).toPass()',
  ],
  [
    'expect.poll',
    'Error',
    'expect.poll: expected a function',
    'expect.poll(5).toBe(1)',
  ],
  [
    'expect.any',
    'TypeError',
    'any() expects to be passed a constructor function. Please pass one or use anything() to match any object.',
    'expect.any(undefined)',
  ],
]

it('tests that an uncaught refusal settles the run as its refusal op', async () => {
  for (const code of [
    "await page.route('**', () => {})",
    'browser.browserType()',
    'context.request',
  ]) {
    const runtime = loadPrelude({ seed: SEED })
    const settled = await runtime.settle(code)
    expect([code, settled]).toMatchObject([
      code,
      {
        operation: {
          code: 'method_unavailable',
          owner: 'script',
          delivery: 'not_sent',
        },
      },
    ])
    expect([code, runtime.calls.length]).toEqual([code, 1])
  }
})

// The door's fields are read after a real op: they reach the error only once its answer did,
// and a script must not count on them sooner.
it('tests that a caught refusal is one door call and the run goes on', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => 'Title' })
  const outcome = json(
    await runtime.run(`
      let refused
      try { await page.route('**', () => {}) } catch (error) { refused = error }
      const before = { name: refused.name, message: refused.message }
      await page.title()
      return { before, after: { code: refused.code, owner: refused.owner, delivery: refused.delivery,
        retryable: refused.retryable, timeout: refused.timeout, operationId: refused.operationId } }
    `),
  )
  expect(outcome).toEqual({
    before: { name: 'Error', message: ROUTE },
    after: {
      code: 'method_unavailable',
      owner: 'script',
      delivery: 'not_sent',
      retryable: true,
      timeout: false,
      operationId: 'refused.1',
    },
  })
  expect(runtime.calls).toEqual([
    {
      name: 'page.route',
      input: { $neo: 'refused', code: 'method_unavailable', reason: ROUTE },
    },
    expect.objectContaining({ name: 'page.title' }),
  ])
})

it('tests that an argument refusal is invalid_input under the method the script called', async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => 'Title' })
  const outcome = json(
    await runtime.run(`
      let refused
      try { await page.waitForLoadState('bogus') } catch (error) { refused = error }
      await page.title()
      return { name: refused.name, message: refused.message, code: refused.code, owner: refused.owner }
    `),
  )
  expect(outcome).toEqual({
    name: 'Error',
    message:
      'state: expected one of (load|domcontentloaded|networkidle|commit)',
    code: 'invalid_input',
    owner: 'script',
  })
  expect(runtime.calls[0]).toEqual({
    name: 'page.waitForLoadState',
    input: {
      $neo: 'refused',
      code: 'invalid_input',
      reason:
        'state: expected one of (load|domcontentloaded|networkidle|commit)',
    },
  })
})

it("tests that each argument check upstream's client makes is one refusal under the method the script called", async () => {
  const seen: unknown[] = []
  for (const [, , , script] of ARGUMENT_CHECKS) {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => (call.name === 'page.frames' ? WITH_CHILD : null),
    })
    const thrown = json(
      await runtime.run(`
        const child = page.frames()[1].locator('b')
        try { await (async () => ${script})() } catch (error) { return [error.name, error.message] }
        return 'returned'
      `),
    )
    seen.push([script, thrown, runtime.calls])
  }
  expect(seen).toEqual(
    ARGUMENT_CHECKS.map(([api, name, message, script]) => [
      script,
      [name, message],
      [refusalCall(api, 'invalid_input', message)],
    ]),
  )
})

it('tests that an uncaught argument check settles the run as its refusal, with its code', async () => {
  const runtime = loadPrelude({ seed: SEED })
  expect(await runtime.settle('page.frame({})')).toEqual({
    operation: {
      code: 'invalid_input',
      owner: 'script',
      delivery: 'not_sent',
      retryable: true,
      operationId: 'refused.1',
      timeout: false,
      message: 'Either name or url matcher should be specified',
    },
  })
})

it('tests that a bootstrap without refuse gets the errors the prelude threw before', async () => {
  const runtime = loadPrelude({ seed: SEED, withoutRefuse: true })
  const outcome = json(
    await runtime.run(`
      const fields = (error) => ({ name: error.name, message: error.message, code: error.code,
        owner: error.owner, retryable: error.retryable })
      const caught = async (work) => { try { await work() } catch (error) { return fields(error) } }
      return [
        await caught(() => page.route('**', () => {})),
        await caught(() => page.waitForLoadState('bogus')),
        await caught(() => page.frame({})),
      ]
    `),
  )
  expect(outcome).toEqual([
    {
      name: 'Error',
      message: ROUTE,
      code: 'unsupported',
      owner: 'neo',
      retryable: false,
    },
    {
      name: 'Error',
      message:
        'state: expected one of (load|domcontentloaded|networkidle|commit)',
      code: 'invalid_input',
      owner: 'script',
      retryable: false,
    },
    {
      name: 'Error',
      message: 'Either name or url matcher should be specified',
      code: 'invalid_input',
      owner: 'script',
      retryable: false,
    },
  ])
  expect(runtime.calls).toEqual([])
})
