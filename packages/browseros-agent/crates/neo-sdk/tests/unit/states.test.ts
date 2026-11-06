/** State reads preserve receiver names, booleans, strict matching and their distinct timeouts. */
import { expect, test } from 'bun:test'
import { BrowserContext } from '../../src/browser-context'
import { Connection } from '../../src/connection'
import { Page } from '../../src/page'

function fixture(value: boolean | undefined = true) {
  const messages: unknown[][] = []
  const connection = new Connection(0)
  connection.send = async (...args) => {
    messages.push(args)
    return (value === undefined ? undefined : { value }) as never
  }
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext('browser-context@T', connection),
    connection,
  )
  return { page, messages, connection }
}

test('tests that visibility ignores timeouts on every receiver and returns booleans', async () => {
  const { page, messages } = fixture(false)
  page.setDefaultTimeout(700)
  expect(await page.locator('#control').isVisible({ timeout: 10 })).toBe(false)
  expect(await page.isVisible('#control', { timeout: 20 })).toBe(false)
  expect(await page.mainFrame().isVisible('#control', { timeout: 30 })).toBe(
    false,
  )
  expect(messages).toEqual([
    [
      'locator.isVisible',
      'frame@T',
      'isVisible',
      { selector: '#control', strict: true },
      0,
    ],
    ['page.isVisible', 'frame@T', 'isVisible', { selector: '#control' }, 0],
    ['frame.isVisible', 'frame@T', 'isVisible', { selector: '#control' }, 0],
  ])
})

test('tests that checked reads honour receiver options and the default timeout', async () => {
  const { page, messages } = fixture()
  page.setDefaultTimeout(700)
  expect(await page.locator('#control').isChecked({ timeout: 10 })).toBe(true)
  expect(await page.isChecked('#control')).toBe(true)
  expect(
    await page.mainFrame().isChecked('#control', { strict: true, timeout: 0 }),
  ).toBe(true)
  expect(messages).toEqual([
    [
      'locator.isChecked',
      'frame@T',
      'isChecked',
      { selector: '#control', strict: true },
      10,
    ],
    ['page.isChecked', 'frame@T', 'isChecked', { selector: '#control' }, 700],
    [
      'frame.isChecked',
      'frame@T',
      'isChecked',
      { selector: '#control', strict: true },
      0,
    ],
  ])
})

test('tests that state reads reject a missing protocol result', async () => {
  const { page, connection } = fixture()
  connection.send = async () => undefined
  await expect(page.isVisible('#control')).rejects.toThrow(/contract error/)
  await expect(page.isChecked('#control')).rejects.toThrow(/contract error/)
})
