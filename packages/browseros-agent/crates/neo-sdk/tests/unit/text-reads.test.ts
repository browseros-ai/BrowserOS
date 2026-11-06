/** Selector text reads keep Playwright's wire, receiver names and missing/empty distinction. */
import { expect, test } from 'bun:test'
import * as protocol from '../../src/protocol'
import { failure, NO_TIMEOUT, start } from './fake-runtime'

test('tests that text reads send schema params and the explicit timeout for each receiver', async () => {
  const { page, fake } = start()
  fake.answer = () => ({ result: { value: 'Read' } })
  for (const [scope, target] of [
    ['locator', page.locator('p')],
    ['page', page],
    ['frame', page.mainFrame()],
  ] as const) {
    for (const method of ['innerText', 'getAttribute'] as const) {
      const args: unknown[] = scope === 'locator' ? [] : ['p']
      if (method === 'getAttribute') args.push('data-label')
      const options = { timeout: 77, ignored: true }
      const read = target[method] as (...args: unknown[]) => Promise<unknown>
      expect(await read.apply(target, [...args, options])).toBe('Read')
      const params = {
        selector: 'p',
        ...(scope === 'locator' ? { strict: true } : {}),
        ...(method === 'getAttribute' ? { name: 'data-label' } : {}),
      }
      expect(fake.messages.at(-1)).toEqual({
        id: fake.messages.length,
        guid: 'frame@RUN',
        method,
        params,
        metadata: { ...NO_TIMEOUT, timeout: 77 },
      })
      expect(Object.keys(fake.messages.at(-1)?.params ?? {})).toEqual(
        Object.keys(params),
      )
    }
  }
})

test('tests that missing attributes become null and empty values stay empty', async () => {
  const { page, fake } = start()
  for (const [result, expected] of [
    [{}, null],
    [{ value: '' }, ''],
    [{ value: 'present' }, 'present'],
  ] as const) {
    fake.answer = () => ({ result })
    expect(await page.getAttribute('p', 'title')).toBe(expected)
    expect(await page.mainFrame().getAttribute('p', 'title')).toBe(expected)
    expect(await page.locator('p').getAttribute('title')).toBe(expected)
  }
  fake.answer = () => ({ result: { value: '' } })
  expect(await page.innerText('p')).toBe('')
  expect(await page.mainFrame().innerText('p')).toBe('')
  expect(await page.locator('p').innerText()).toBe('')
})

test('tests that null options and omitted options use the page default timeout', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(123)
  fake.answer = () => ({ result: { value: '' } })
  for (const options of [undefined, null] as const) {
    await page.innerText('p', options as never)
    await page.getAttribute('p', 'title', options as never)
    await page.mainFrame().innerText('p', options as never)
    await page.mainFrame().getAttribute('p', 'title', options as never)
    await page.locator('p').innerText(options as never)
    await page.locator('p').getAttribute('title', options as never)
  }
  expect(fake.messages.map(({ metadata }) => metadata)).toEqual(
    Array(12).fill({ ...NO_TIMEOUT, timeout: 123 }),
  )
})

test('tests that explicit options override strictness and positional params as in Playwright', async () => {
  const { page, fake } = start()
  fake.answer = () => ({ result: { value: '' } })
  const options = {
    selector: 'span',
    name: 'data-other',
    strict: false,
    timeout: 0,
  }
  await page.locator('p').innerText(options as never)
  await page.locator('p').getAttribute('title', options as never)
  await page.getAttribute('p', 'title', options as never)
  await page.mainFrame().getAttribute('p', 'title', options as never)
  expect(fake.messages.map(({ params }) => params)).toEqual([
    { selector: 'span', strict: false },
    ...Array(3).fill({ selector: 'span', strict: false, name: 'data-other' }),
  ])
  expect(fake.messages.map(({ metadata }) => metadata)).toEqual(
    Array(4).fill(NO_TIMEOUT),
  )
})

test('tests that read errors retain the public receiver API name', async () => {
  const { page, fake } = start()
  fake.answer = () => failure('Error', 'read failed')
  for (const [scope, target] of [
    ['locator', page.locator('p')],
    ['page', page],
    ['frame', page.mainFrame()],
  ] as const) {
    for (const method of ['innerText', 'getAttribute'] as const) {
      const args: unknown[] = scope === 'locator' ? [] : ['p']
      if (method === 'getAttribute') args.push('title')
      const read = target[method] as (...args: unknown[]) => Promise<unknown>
      await expect(read.apply(target, args)).rejects.toThrow(
        `${scope}.${method}: read failed`,
      )
    }
  }
})

test('tests that text read key lists match Playwright schema order', () => {
  const keys = protocol as unknown as Record<string, readonly string[]>
  expect(keys.FRAME_INNER_TEXT_KEYS).toEqual(['selector', 'strict'])
  expect(keys.FRAME_GET_ATTRIBUTE_KEYS).toEqual(['selector', 'strict', 'name'])
})
