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

test('tests that allTextContents rejects what is not a string array', async () => {
  const { fake, page } = start()
  for (const result of [
    undefined,
    {},
    { value: null },
    { value: 'x' },
    { value: { v: 'null' } },
    { value: { a: [3], id: 1 } },
    { value: { a: [{ v: 'null' }], id: 1 } },
    { value: { a: ['x', { v: 'undefined' }], id: 1 } },
    { value: { o: [], id: 1 } },
  ]) {
    fake.answer = () => ({ result })
    await expect(page.locator('.item').allTextContents()).rejects.toThrow(
      /neo-sdk: .*contract error/,
    )
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
