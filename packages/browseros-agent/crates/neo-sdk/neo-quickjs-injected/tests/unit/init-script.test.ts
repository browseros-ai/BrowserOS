/**
 * `page.addInitScript`, `browserContext.addInitScript` and their `Disposable` against a fake
 * `__neo_runtime` (fake-runtime.ts): the source Playwright's `evaluationScript` builds and the
 * exact messages, the refusals before any message, and a `dispose` that takes a closed script as
 * done, as Playwright's `DisposableObject.dispose` does (`client/clientHelper.ts`,
 * `client/disposable.ts` at 26137face).
 */

import { describe, expect, test } from 'bun:test'
import { failure, NO_TIMEOUT, start, TARGET_CLOSED } from './fake-runtime'

const SCRIPT = 'disposable@run.1'
const ADDED = { result: { disposable: { guid: SCRIPT } }, log: [] }

/** What a call's rejection says, `null` when it resolved. */
async function rejection(call: Promise<unknown>) {
  return await call.then(
    () => null,
    (error) => error.message,
  )
}

describe('addInitScript', () => {
  test('sends the source Playwright builds, on the page or the context', async () => {
    const { fake, page, context } = start()
    fake.answer = () => ADDED
    const withArg = (arg: unknown) => arg
    function noArg() {}
    await page.addInitScript(withArg, { a: [1, 'b'] })
    await page.addInitScript(noArg)
    await page.addInitScript('window.y = 2')
    await context.addInitScript({ content: 'window.z = 3' })
    // A script's `signal: null` is no signal (Playwright's type has no `signal`).
    await page.addInitScript(withArg, undefined, { signal: null } as never)
    expect(fake.messages).toEqual([
      {
        id: 1,
        guid: 'page@RUN',
        method: 'addInitScript',
        params: { source: `(${withArg.toString()})({"a":[1,"b"]})` },
        metadata: NO_TIMEOUT,
      },
      {
        id: 2,
        guid: 'page@RUN',
        method: 'addInitScript',
        params: { source: `(${noArg.toString()})(undefined)` },
        metadata: NO_TIMEOUT,
      },
      {
        id: 3,
        guid: 'page@RUN',
        method: 'addInitScript',
        params: { source: 'window.y = 2' },
        metadata: NO_TIMEOUT,
      },
      {
        id: 4,
        guid: 'context@default',
        method: 'addInitScript',
        params: { source: 'window.z = 3' },
        metadata: NO_TIMEOUT,
      },
      {
        id: 5,
        guid: 'page@RUN',
        method: 'addInitScript',
        params: { source: `(${withArg.toString()})(undefined)` },
        metadata: NO_TIMEOUT,
      },
    ])
  })

  test('refuses before any message what Playwright refuses, and what Neo cannot do', async () => {
    const { fake, page, context } = start()
    fake.answer = () => ADDED
    expect(await rejection(page.addInitScript('1', 2))).toBe(
      'Cannot evaluate a string with arguments',
    )
    expect(
      await rejection(page.addInitScript({} as { content?: string })),
    ).toBe('Either path or content property must be present')
    expect(await rejection(context.addInitScript({ path: '/tmp/a.js' }))).toBe(
      'browserContext.addInitScript: path: this run has no artifact directory, so Neo cannot read "/tmp/a.js"',
    )
    expect(
      await rejection(page.addInitScript('1', undefined, 5 as never)),
    ).toStartWith('Too many arguments.')
    expect(
      await rejection(
        page.addInitScript('1', undefined, {
          signal: new AbortController().signal,
        } as never),
      ),
    ).toBe('page.addInitScript: Neo does not support signal yet')
    expect(
      await rejection(
        context.addInitScript(() => 1, undefined, { exposeFunctions: true }),
      ),
    ).toBe(
      'browserContext.addInitScript: Neo does not support exposeFunctions yet',
    )
    expect(fake.messages).toEqual([])
  })
})

describe('Disposable.dispose', () => {
  test('sends dispose to the script, and takes a closed script as done', async () => {
    const { fake, page } = start()
    fake.answer = (message) =>
      message.method === 'addInitScript' ? ADDED : TARGET_CLOSED
    const script = await page.addInitScript('1')
    expect(await script.dispose()).toBeUndefined()
    expect(fake.messages.at(-1)).toEqual({
      id: 2,
      guid: SCRIPT,
      method: 'dispose',
      params: {},
      metadata: NO_TIMEOUT,
    })
  })

  test('rethrows every other failure', async () => {
    const { fake, page } = start()
    fake.answer = (message) =>
      message.method === 'addInitScript'
        ? ADDED
        : failure('Error', 'the browser went away')
    const script = await page.addInitScript('1')
    expect(await rejection(script.dispose())).toBe(
      'disposable.dispose: the browser went away',
    )
  })
})
