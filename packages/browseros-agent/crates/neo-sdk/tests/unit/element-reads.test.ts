/**
 * B1 element reads and states on the wire: each receiver's `apiName`, Playwright's strictness
 * (Locator strict, Page and Frame not), the timeout each sends (`isHidden` none, the waiting
 * states the action timeout), and the realm-only `all`, `toString`, `description` and
 * `getByRef`.
 */
import { expect, test } from 'bun:test'
import { failure, type Message, NO_TIMEOUT, start } from './fake-runtime'

/** Every message's `[method, params, metadata.timeout]`. */
function sent(messages: Message[]) {
  return messages.map(({ method, params, metadata }) => [
    method,
    params,
    (metadata as { timeout?: number }).timeout,
  ])
}

test('tests that the waiting states send strict locator reads under the action timeout', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  fake.answer = () => ({ result: { value: true } })
  for (const method of ['isEnabled', 'isDisabled', 'isEditable'] as const) {
    expect(await page.locator('#control')[method]({ timeout: 10 })).toBe(true)
    expect(await page[method]('#control')).toBe(true)
    expect(await page.mainFrame()[method]('#control', { strict: true })).toBe(
      true,
    )
  }
  expect(sent(fake.messages)).toEqual(
    ['isEnabled', 'isDisabled', 'isEditable'].flatMap((method) => [
      [method, { selector: '#control', strict: true }, 10],
      [method, { selector: '#control' }, 700],
      [method, { selector: '#control', strict: true }, 700],
    ]),
  )
})

test('tests that isHidden reads once with no timeout on every receiver', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  fake.answer = () => ({ result: { value: false } })
  expect(await page.locator('#control').isHidden({ timeout: 10 })).toBe(false)
  expect(await page.isHidden('#control', { timeout: 20 })).toBe(false)
  expect(await page.mainFrame().isHidden('#control', null as never)).toBe(false)
  expect(sent(fake.messages)).toEqual([
    ['isHidden', { selector: '#control', strict: true }, 0],
    ['isHidden', { selector: '#control' }, 0],
    ['isHidden', { selector: '#control' }, 0],
  ])
})

test('tests that innerHTML returns the markup string from every receiver', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  fake.answer = () => ({ result: { value: '<b>x</b>' } })
  expect(await page.locator('#list').innerHTML()).toBe('<b>x</b>')
  expect(await page.innerHTML('#list', null as never)).toBe('<b>x</b>')
  expect(await page.mainFrame().innerHTML('#list', { timeout: 5 })).toBe(
    '<b>x</b>',
  )
  expect(sent(fake.messages)).toEqual([
    ['innerHTML', { selector: '#list', strict: true }, 700],
    ['innerHTML', { selector: '#list' }, 700],
    ['innerHTML', { selector: '#list' }, 5],
  ])
})

test('tests that allInnerTexts sends the served function and unwraps ordered strings', async () => {
  const { fake, page } = start()
  fake.answer = () => ({
    result: { value: { a: ['One', ''], id: 1 } },
  })
  expect(await page.locator('.item').allInnerTexts()).toEqual(['One', ''])
  expect(fake.messages).toEqual([
    {
      id: 1,
      guid: 'frame@RUN',
      method: 'evalOnSelectorAll',
      params: {
        selector: '.item',
        expression: 'ee => ee.map(e => e.innerText)',
        isFunction: true,
        arg: { value: { v: 'undefined' }, handles: [] },
      },
      metadata: NO_TIMEOUT,
    },
  ])
  // An SVG match has no innerText: Playwright's page function reads undefined there.
  fake.answer = () => ({
    result: { value: { a: ['HTML', { v: 'undefined' }], id: 1 } },
  })
  // Playwright declares string[] and answers undefined at runtime, as Neo does.
  const withSvg: (string | undefined)[] = await page
    .locator('.item')
    .allInnerTexts()
  expect(withSvg).toEqual(['HTML', undefined])
  expect(withSvg.length).toBe(2)
  expect(1 in withSvg).toBe(true)
  fake.answer = () => ({ result: { value: { a: [1], id: 1 } } })
  const numeric: unknown[] = await page.locator('.item').allInnerTexts()
  expect(numeric).toEqual([1])
  // A page-defined getter may answer a number: $$eval preserves its result.
  fake.answer = () => ({ result: { value: { a: [42], id: 1 } } })
  const texts: unknown[] = await page.locator('.item').allTextContents()
  expect(texts).toEqual([42])
})

test('tests that all() counts once and returns one nth locator per match', async () => {
  const { fake, page } = start()
  fake.answer = () => ({ result: { value: 3 } })
  const items = await page.locator('li').all()
  expect(items.map((item) => item._selector)).toEqual([
    'li >> nth=0',
    'li >> nth=1',
    'li >> nth=2',
  ])
  expect(items.every((item) => item._frame === page.mainFrame())).toBe(true)
  expect(sent(fake.messages)).toEqual([['queryCount', { selector: 'li' }, 0]])
  fake.answer = () => ({ result: { value: 0 } })
  expect(await page.locator('li').all()).toEqual([])
})

test('tests that toString and description read the selector back without a page call', () => {
  const { fake, page } = start()
  const role = page.getByRole('button', { name: 'Submit' })
  expect(role.toString()).toBe(`getByRole('button', { name: 'Submit' })`)
  expect(`${role}`).toBe(role.toString())
  expect(role.description()).toBe(null)
  const described = role.describe('Submit button')
  expect(described.toString()).toBe('Submit button')
  expect(described.description()).toBe('Submit button')
  expect(described.locator('span').description()).toBe(null)
  expect(page.locator('internal:has-text=>>').toString()).toBe(
    'internal:has-text=>>',
  )
  expect(fake.messages).toEqual([])
})

test('tests that getByRef is an aria-ref locator of the main frame', () => {
  const { page } = start()
  const ref = page.getByRef('e5')
  expect(ref._selector).toBe('aria-ref=e5')
  expect(ref._frame).toBe(page.mainFrame())
  expect('getByRef' in page.mainFrame()).toBe(false)
})

test('tests that B1 errors carry their receivers API names', async () => {
  const { fake, page } = start()
  fake.answer = () => failure('Error', 'invalid selector')
  await expect(page.locator('[').isEnabled()).rejects.toThrow(
    'locator.isEnabled:',
  )
  await expect(page.isHidden('[')).rejects.toThrow('page.isHidden:')
  await expect(page.mainFrame().innerHTML('[')).rejects.toThrow(
    'frame.innerHTML:',
  )
  await expect(page.locator('[').allInnerTexts()).rejects.toThrow(
    'locator.allInnerTexts:',
  )
  await expect(page.locator('[').all()).rejects.toThrow('locator.all:')
})
