/**
 * `createRun`, `context.pages()` and `context.newPage()` against a fake `__neo_runtime`
 * (fake-runtime.ts): one `Page` per guid across reads, each bound to its own tab and main frame,
 * and the exact `newPage` message of the wire contract.
 */

import { describe, expect, test } from 'bun:test'
import {
  failure,
  NO_TIMEOUT,
  OTHER_TAB,
  RUN_TAB,
  sameObjects,
  start,
} from './fake-runtime'

describe('createRun', () => {
  test('the run page belongs to the context and is its first page', () => {
    const { page, context } = start()
    expect(page.context()).toBe(context)
    expect(sameObjects(context.pages(), [page])).toBe(true)
  })

  test('reads and sends nothing before the script does', () => {
    const { fake } = start()
    expect([fake.gets, fake.messages]).toEqual([[], []])
  })
})

describe('BrowserContext.pages', () => {
  test('returns the same objects on every read, in the order Rust lists the tabs', () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [first, other] = context.pages()
    expect(first).toBe(page)
    expect(sameObjects(context.pages(), [page, other])).toBe(true)

    fake.tabs.reverse()
    expect(sameObjects(context.pages(), [other, page])).toBe(true)
  })

  test('binds each listed page to its own tab and its own main frame', () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    expect(other.url()).toBe(OTHER_TAB.url)
    expect(fake.gets.at(-1)).toEqual({ guid: 'page@B', field: 'url' })
    expect(other.context()).toBe(context)
    expect(other.mainFrame()).toBe(other.mainFrame())
    expect(other.mainFrame().page()).toBe(other)
    expect(other.mainFrame()).not.toBe(page.mainFrame())
    expect(() => page.locator('a').and(other.locator('a'))).toThrow(
      'Locators must belong to the same frame.',
    )
  })

  test("each page's calls go to its own tab's main frame", async () => {
    const { fake, page, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    await Promise.all([
      other.locator('#b').click(),
      page.click('#run'),
      other.getByText('B').click(),
    ])
    expect(
      fake.messages.map(({ guid, method, params }) => [
        guid,
        method,
        (params as { selector: string }).selector,
      ]),
    ).toEqual([
      ['frame@B', 'click', '#b'],
      ['frame@RUN', 'click', '#run'],
      ['frame@B', 'click', 'internal:text="B"i'],
    ])
  })

  test('a page missing from a later read leaves the map: listed again, it is a new object', () => {
    const { fake, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    fake.tabs.splice(1, 1)
    expect(context.pages().includes(other)).toBe(false)

    fake.tabs.push(OTHER_TAB)
    const [, again] = context.pages()
    expect(again).not.toBe(other)
    expect(again.url()).toBe(OTHER_TAB.url)
  })

  test('once its tab is gone, a listed page still answers url() with the URL it was listed at', () => {
    const { fake, context } = start([RUN_TAB, OTHER_TAB])
    const [, other] = context.pages()
    fake.tabs.splice(1, 1)
    expect([other.isClosed(), other.url()]).toEqual([true, OTHER_TAB.url])
  })
})

describe('BrowserContext.newPage', () => {
  test('sends newPage to the context, with no params and no timeout', async () => {
    const { fake, page, context } = start()
    context.setDefaultTimeout(5000)
    page.setDefaultTimeout(1000)
    await context.newPage()
    expect(fake.messages).toEqual([
      {
        id: 1,
        guid: 'context@default',
        method: 'newPage',
        params: {},
        metadata: NO_TIMEOUT,
      },
    ])
  })

  test('resolves with the page a later pages() lists for its tab', async () => {
    const { fake, page, context } = start()
    const created = await context.newPage()
    expect(fake.gets).toEqual([{ guid: 'page@NEW1', field: 'initializer' }])
    expect(created).not.toBe(page)
    expect(created.context()).toBe(context)
    expect(created.url()).toBe('about:blank')
    expect(sameObjects(context.pages(), [page, created])).toBe(true)
  })

  test('two new pages are two objects, each with its own main frame', async () => {
    const { fake, context } = start()
    const first = await context.newPage()
    const second = await context.newPage()
    expect(first).not.toBe(second)
    expect(first.mainFrame()).not.toBe(second.mainFrame())
    expect(() => first.locator('a').or(second.locator('a'))).toThrow(
      'Locators must belong to the same frame.',
    )
    await second.locator('#go').click()
    expect(fake.messages.at(-1)?.guid).toBe('frame@NEW2')
  })

  test('reuses the object a pages() read made while newPage was in flight', async () => {
    const { fake, context } = start()
    const opening = context.newPage()
    const [, listed] = context.pages()
    expect(await opening).toBe(listed)
    expect(fake.gets.map(({ field }) => field)).toEqual(['pages'])
  })

  test('a new page closed behind the script keeps answering url() with about:blank', async () => {
    const { fake, context } = start()
    const created = await context.newPage()
    fake.tabs.pop()
    expect([created.isClosed(), created.url()]).toEqual([true, 'about:blank'])
  })

  test('a failed newPage rejects with the browserContext.newPage prefix', async () => {
    const { fake, context } = start()
    fake.answer = () => failure('Error', 'no window to open the tab in')
    const error = await context.newPage().catch((caught) => caught)
    expect([error.name, error.message]).toEqual([
      'Error',
      'browserContext.newPage: no window to open the tab in',
    ])
  })
})
