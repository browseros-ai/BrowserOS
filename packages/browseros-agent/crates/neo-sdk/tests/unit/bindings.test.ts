/**
 * `exposeFunction` / `exposeBinding` and `BindingCall` against a fake `__neo_runtime`
 * (fake-runtime.ts): the exact messages, the answers a callback's result and throws become
 * (Playwright's `BindingCall.call` at 26137face), and a refused registration that keeps the
 * callback a name had.
 */

import { describe, expect, test } from 'bun:test'
import { failure, NO_TIMEOUT, start } from './fake-runtime'

const EXPOSED = {
  result: { disposable: { guid: 'disposable@binding.r.1' } },
  log: [],
}

function record(name: string, args: unknown[]) {
  return {
    guid: 'bindingcall@r.7',
    frame: { guid: 'frame@RUN' },
    page: { guid: 'page@RUN' },
    name,
    args,
  }
}

describe('exposeBinding', () => {
  test('sends exposeBinding on the page or the context', async () => {
    const { fake, page, context } = start()
    fake.answer = () => EXPOSED
    await page.exposeFunction('compute', (a: number, b: number) => a * b)
    await context.exposeBinding('who', () => 'me')
    expect(
      fake.messages.map(({ guid, method, params, metadata }) => ({
        guid,
        method,
        params,
        metadata,
      })),
    ).toEqual([
      {
        guid: 'page@RUN',
        method: 'exposeBinding',
        params: { name: 'compute' },
        metadata: NO_TIMEOUT,
      },
      {
        guid: 'context@default',
        method: 'exposeBinding',
        params: { name: 'who' },
        metadata: NO_TIMEOUT,
      },
    ])
  })

  test('a refused registration keeps the callback the name had', async () => {
    const { fake, page } = start()
    fake.answer = () => EXPOSED
    await page.exposeFunction('compute', () => 'first')
    fake.answer = () =>
      failure('Error', 'Function "compute" has been already registered')
    await expect(
      page.exposeFunction('compute', () => 'second'),
    ).rejects.toThrow('has been already registered')
    fake.answer = () => ({ result: {}, log: [] })
    await page._onBinding(record('compute', []))
    expect(fake.messages.at(-1)?.params).toEqual({
      result: { value: 'first', handles: [] },
    })
  })
})

describe('BindingCall', () => {
  test('answers the page with the callback’s result, its arguments parsed', async () => {
    const { fake, page } = start()
    fake.answer = () => EXPOSED
    await page.exposeFunction('compute', (a: number, b: number) =>
      Promise.resolve(a * b),
    )
    fake.answer = () => ({ result: {}, log: [] })
    await page._onBinding(record('compute', [3, { v: 'undefined' }]))
    expect(fake.messages.at(-1)).toMatchObject({
      guid: 'bindingcall@r.7',
      method: 'resolve',
      params: { result: { value: { v: 'NaN' }, handles: [] } },
    })
  })

  test('rejects with an Error’s fields, or any thrown value itself', async () => {
    const { fake, page } = start()
    fake.answer = () => EXPOSED
    await page.exposeFunction('woof', () => {
      throw new TypeError('WOOF')
    })
    await page.exposeFunction('nothing', () => {
      throw null
    })
    fake.answer = () => ({ result: {}, log: [] })
    await page._onBinding(record('woof', []))
    const woof = fake.messages.at(-1)
    expect(woof?.method).toBe('reject')
    expect(woof?.params).toMatchObject({
      error: { error: { message: 'WOOF', name: 'TypeError' } },
    })
    await page._onBinding(record('nothing', []))
    expect(fake.messages.at(-1)?.params).toEqual({
      error: { value: { v: 'null' } },
    })
  })

  test('the callback gets the call’s context, page and frame', async () => {
    const { fake, page, context } = start()
    fake.answer = () => EXPOSED
    let seen: unknown
    await page.exposeBinding('who', (source) => {
      seen = source
    })
    fake.answer = () => ({ result: {}, log: [] })
    await page._onBinding(record('who', []))
    expect(seen).toEqual({ context, page, frame: page.mainFrame() })
  })
})
