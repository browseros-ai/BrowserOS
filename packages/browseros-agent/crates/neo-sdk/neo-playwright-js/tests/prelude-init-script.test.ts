// `page.addInitScript` and `context.addInitScript` in the prelude: the source each sends is the one
// upstream's client builds (`evaluationScript`), a `{ path }` goes to Rust to read, upstream's
// client checks throw before anything is registered, and the disposable each returns disposes its
// registration.
import { describe, expect, it } from 'bun:test'
import { loadPrelude, SEED } from './support/prelude-runtime'

function runtime() {
  return loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.addInitScript' ||
      call.name === 'context.addInitScript'
        ? 7
        : null,
  })
}

describe('the source sent', () => {
  const cases: [label: string, script: string, sent: unknown][] = [
    [
      'a function with its argument, as JSON.stringify spells it',
      'page.addInitScript((a) => { window.a = a }, { n: 1, cb: () => {}, u: undefined })',
      { source: '((a) => { window.a = a })({"n":1})' },
    ],
    [
      'a function without an argument',
      'page.addInitScript(() => 1)',
      { source: '(() => 1)(undefined)' },
    ],
    [
      'a string as written',
      "page.addInitScript('window.s = 1')",
      { source: 'window.s = 1' },
    ],
    [
      'content before a path',
      "page.addInitScript({ content: 'window.c = 1', path: '/x.js' })",
      { source: 'window.c = 1' },
    ],
    [
      'a path, for Rust to read',
      "page.addInitScript({ path: 'x.js' })",
      { path: 'x.js' },
    ],
  ]
  for (const [label, script, sent] of cases)
    it(`tests that the page sends ${label}`, async () => {
      const run = runtime()
      await run.run(`await ${script}`)
      const [call] = run.sent('page.addInitScript')
      const { source, path } = call?.input ?? {}
      expect(JSON.parse(JSON.stringify({ source, path }))).toEqual(sent)
      expect(call?.input.call).toEqual({
        api: 'page.addInitScript',
        timeoutMs: 0,
      })
    })

  it('tests that the context sends its script with no page', async () => {
    const run = runtime()
    await run.run("await context.addInitScript('window.c = 1')")
    const [call] = run.sent('context.addInitScript')
    expect(call?.input).toEqual({
      call: { api: 'browserContext.addInitScript', timeoutMs: 0 },
      source: 'window.c = 1',
    })
  })
})

describe("upstream's client checks", () => {
  const cases: [script: string, message: string][] = [
    [
      "page.addInitScript('window.s = 1', 5)",
      'Cannot evaluate a string with arguments',
    ],
    [
      'page.addInitScript({})',
      'Either path or content property must be present',
    ],
    [
      "page.addInitScript(() => 1, undefined, 'options')",
      'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.',
    ],
    [
      "page.addInitScript({ content: 'window.foo = 1;' }, undefined, { exposeFunctions: true })",
      'Passing functions requires the init script to be a function',
    ],
  ]
  for (const [script, message] of cases)
    it(`tests that ${script} throws before anything is registered`, async () => {
      const run = runtime()
      const thrown = await run.run(
        `try { await ${script}; return null } catch (error) { return error.message }`,
      )
      expect(thrown).toBe(message)
      // Only the door's record of the refusal reaches the host: nothing is registered.
      expect(run.sent('page.addInitScript').map((call) => call.input)).toEqual([
        { $neo: 'refused', code: 'invalid_input', reason: message },
      ])
    })
})

it('tests that dispose removes the registration it returned, each time it is called', async () => {
  const run = runtime()
  await run.run(`
    const disposable = await page.addInitScript(() => 1)
    await disposable.dispose()
    await disposable.dispose()
  `)
  expect(run.sent('disposable.dispose').map((call) => call.input)).toEqual([
    { call: { api: 'disposable.dispose', timeoutMs: 0 }, id: 7 },
    { call: { api: 'disposable.dispose', timeoutMs: 0 }, id: 7 },
  ])
})
