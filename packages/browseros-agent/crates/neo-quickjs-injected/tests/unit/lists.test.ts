/** Locator collections preserve Playwright's snapshot wire contract and public values. */
import { expect, test } from 'bun:test'
import { failure, NO_TIMEOUT, start } from './fake-runtime'

test('tests that count sends a non-strict queryCount with no timeout', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(17)
  fake.answer = () => ({ result: { value: 3 } })
  expect(await page.locator('.item').count()).toBe(3)
  expect(fake.messages).toEqual([
    {
      id: 1,
      guid: 'frame@RUN',
      method: 'queryCount',
      params: { selector: '.item' },
      metadata: NO_TIMEOUT,
    },
  ])
  fake.answer = () => ({ result: { value: 0 } })
  expect(await page.locator('.missing').count()).toBe(0)
})

test('tests that allTextContents sends Playwright’s function and unwraps ordered strings', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(17)
  // The page's own encoding (`utilityScriptSerializers.ts`): strings are plain JSON.
  fake.answer = () => ({
    result: { value: { a: ['visible', 'hidden', ''], id: 1 } },
  })
  expect(await page.locator('.item').allTextContents()).toEqual([
    'visible',
    'hidden',
    '',
  ])
  expect(fake.messages).toEqual([
    {
      id: 1,
      guid: 'frame@RUN',
      method: 'evalOnSelectorAll',
      params: {
        selector: '.item',
        expression: "ee => ee.map(e => e.textContent || '')",
        isFunction: true,
        arg: { value: { v: 'undefined' }, handles: [] },
      },
      metadata: NO_TIMEOUT,
    },
  ])
  fake.answer = () => ({ result: { value: { a: [], id: 1 } } })
  expect(await page.locator('.missing').allTextContents()).toEqual([])
})

test('tests that allTextContents answers the page values as parsed', async () => {
  const { fake, page } = start()
  // Playwright answers what the page's function returned, a getter's number included.
  const cases: [unknown, unknown[]][] = [
    [{ value: { a: [42, 'B'], id: 1 } }, [42, 'B']],
    [{ value: { a: [], id: 1 } }, []],
    [{ value: { a: ['x', { v: 'null' }], id: 1 } }, ['x', null]],
  ]
  for (const [result, values] of cases) {
    fake.answer = () => ({ result })
    const texts: unknown[] = await page.locator('.item').allTextContents()
    expect(texts).toEqual(values)
  }
})

test('tests that collection errors use their Locator API names', async () => {
  const { fake, page } = start()
  fake.answer = () => failure('Error', 'invalid selector')
  await expect(page.locator('[').count()).rejects.toThrow('locator.count:')
  await expect(page.locator('[').allTextContents()).rejects.toThrow(
    'locator.allTextContents:',
  )
})

test('tests that collections add no Page or Frame public methods', () => {
  const { page } = start()
  for (const target of [page, page.mainFrame()]) {
    expect('count' in target).toBe(false)
    expect('allTextContents' in target).toBe(false)
  }
})
