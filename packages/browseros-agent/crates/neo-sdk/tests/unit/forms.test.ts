/** Forms use Playwright's wire params, return values and outer public API names. */
import { expect, test } from 'bun:test'
import { BrowserContext } from '../../src/browser-context'
import { Connection } from '../../src/connection'
import { Page } from '../../src/page'
import * as protocol from '../../src/protocol'
import { convertSelectOptionValues } from '../../src/select-option'

test('tests that select values preserve Playwright conversion and null errors', () => {
  expect(convertSelectOptionValues(null)).toEqual({})
  expect(convertSelectOptionValues([])).toEqual({})
  expect(convertSelectOptionValues('One')).toEqual({
    options: [{ valueOrLabel: 'One' }],
  })
  expect(convertSelectOptionValues(['a', 'b'])).toEqual({
    options: [{ valueOrLabel: 'a' }, { valueOrLabel: 'b' }],
  })
  const values = [{ value: 'a', label: 'A', index: 0 }]
  expect(convertSelectOptionValues(values).options).toBe(values)
  expect(convertSelectOptionValues(values[0])).toEqual({ options: values })
  expect(() => convertSelectOptionValues(['a', null] as never)).toThrow(
    'options[1]: expected object, got null',
  )
})

test('tests that every forms key list follows the Playwright schema', () => {
  const lists = {
    FRAME_FILL_KEYS: ['selector', 'strict', 'value', 'force'],
    FRAME_PRESS_KEYS: ['selector', 'strict', 'key', 'delay', 'noWaitAfter'],
    FRAME_INPUT_VALUE_KEYS: ['selector', 'strict'],
    FRAME_CHECK_KEYS: [
      'selector',
      'strict',
      'force',
      'scroll',
      'position',
      'trial',
    ],
    FRAME_UNCHECK_KEYS: [
      'selector',
      'strict',
      'force',
      'scroll',
      'position',
      'trial',
    ],
    FRAME_SELECT_OPTION_KEYS: [
      'selector',
      'strict',
      'elements',
      'options',
      'force',
    ],
  }
  for (const [name, keys] of Object.entries(lists)) {
    expect(protocol[name as keyof typeof lists] as readonly string[]).toEqual(
      keys,
    )
    const unordered = Object.fromEntries(
      [...keys].reverse().map((key) => [key, key]),
    )
    expect(
      Object.keys(
        protocol.pickParams(protocol[name as keyof typeof lists], {
          ...unordered,
          timeout: 99,
          extra: true,
        }),
      ),
    ).toEqual(keys)
  }
})

test('tests that missing inputValue and selectOption results are contract errors', async () => {
  const connection = new Connection(0)
  connection.send = async () => undefined
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext(),
    connection,
  )
  await expect(page.mainFrame().inputValue('input')).rejects.toThrow(
    /contract error/,
  )
  await expect(page.mainFrame().selectOption('select', 'one')).rejects.toThrow(
    /contract error/,
  )
})

test('tests that invalid select fields reject before sending anything', async () => {
  const connection = new Connection(0)
  let calls = 0
  connection.send = async () => {
    calls++
    return { values: ['unexpected'] } as never
  }
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext(),
    connection,
  )
  for (const value of [
    { index: NaN },
    { index: Infinity },
    { index: -Infinity },
    { index: null },
    { index: 1.5 },
    { index: 'one' },
    { label: null },
    { label: 1 },
    { value: null },
    { value: false },
    { valueOrLabel: null },
    { valueOrLabel: 1 },
  ]) {
    await expect(
      page.locator('select').selectOption(value as never),
    ).rejects.toThrow()
  }
  expect(calls).toBe(0)
})

test('tests that Page and Frame forms accept null options with the default timeout', async () => {
  const connection = new Connection(0)
  const timeouts: number[] = []
  connection.send = async (...args) => {
    timeouts.push(args[4])
    if (args[2] === 'inputValue') return { value: 'text' }
    if (args[2] === 'selectOption') return { values: ['one'] }
    return undefined
  }
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext(),
    connection,
  )
  page.setDefaultTimeout(123)
  for (const target of [page, page.mainFrame()]) {
    await target.fill('input', 'text', null as never)
    await target.press('input', 'Enter', null as never)
    expect(await target.inputValue('input', null as never)).toBe('text')
    await target.check('input', null as never)
    await target.uncheck('input', null as never)
    await target.setChecked('input', true, null as never)
    await target.setChecked('input', false, null as never)
    expect(await target.selectOption('select', 'one', null as never)).toEqual([
      'one',
    ])
  }
  expect(timeouts).toEqual(Array(16).fill(123))
})

test('tests that forms send the outer API name, strictness and timeout and unwrap results', async () => {
  const connection = new Connection(0)
  const calls: Parameters<Connection['send']>[] = []
  connection.send = async (...args) => {
    calls.push(args)
    if (args[2] === 'inputValue') return { value: 'text' }
    if (args[2] === 'selectOption') return { values: ['a'] }
    return undefined
  }
  const page = new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext(),
    connection,
  )
  page.setDefaultTimeout(123)
  const methods = [
    ['fill', ['text'], 'fill'],
    ['press', ['Enter'], 'press'],
    ['inputValue', [], 'inputValue'],
    ['check', [], 'check'],
    ['uncheck', [], 'uncheck'],
    ['setChecked', [true], 'check'],
    ['setChecked', [false], 'uncheck'],
    ['selectOption', ['a'], 'selectOption'],
  ] as const
  for (const [scope, target] of [
    ['locator', page.locator('input')],
    ['frame', page.mainFrame()],
    ['page', page],
  ] as const) {
    for (const [method, args, wire] of methods) {
      const positional = scope === 'locator' ? [...args] : ['input', ...args]
      const operation = target[method] as (
        ...args: unknown[]
      ) => Promise<unknown>
      const result = await operation.apply(target, [
        ...positional,
        { timeout: 77, ignored: true },
      ])
      const call = calls.at(-1)
      if (!call) throw new Error('forms operation sent no call')
      const [apiName, guid, actualWire, params, timeout] = call
      expect([apiName, guid, actualWire, timeout]).toEqual([
        `${scope}.${method}`,
        'frame@T',
        wire,
        77,
      ])
      expect((params as { selector: string }).selector).toBe('input')
      expect((params as { strict?: boolean }).strict).toBe(
        scope === 'locator' ? true : undefined,
      )
      expect(params).not.toHaveProperty('timeout')
      expect(params).not.toHaveProperty('ignored')
      expect(result).toEqual(
        wire === 'inputValue'
          ? 'text'
          : wire === 'selectOption'
            ? ['a']
            : undefined,
      )
    }
  }
  await page.locator('input').clear()
  expect(calls.at(-1)).toEqual([
    'locator.clear',
    'frame@T',
    'fill',
    { selector: 'input', strict: true, value: '' },
    123,
  ])
})
