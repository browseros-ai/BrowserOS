/**
 * `null` options as Playwright's client treats them (`client/frame.ts`, `client/page.ts` and the
 * protocol validator at 26137face): a `null` options object reads as `{}` where Playwright spreads
 * it and resolves its timeout with `options || {}`, and a schema key set to `null` rejects with a
 * plain `Error` before any message is sent, because the validator's `tOptional` lets only
 * `undefined` through. `timeout` and unknown keys are not schema keys, so `null` there is ignored.
 */

import { expect, test } from 'bun:test'
import { pickParams } from '../../src/protocol'
import { NO_TIMEOUT, start } from './fake-runtime'

/** What a call's rejection carries for a script, `null` when it resolved. */
async function rejection(call: () => Promise<unknown>) {
  return await Promise.resolve()
    .then(call)
    .then(
      () => null,
      (error) => error.name,
    )
}

test('tests that click and textContent read a null options object as {} on Page and Frame', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(123)
  fake.answer = ({ method }) =>
    method === 'textContent' ? { result: { value: 'Text' } } : {}
  for (const target of [page, page.mainFrame()]) {
    expect(await target.click('#go', null as never)).toBeUndefined()
    expect(await target.textContent('#go', null as never)).toBe('Text')
  }
  expect(
    fake.messages.map(({ method, params, metadata }) => [
      method,
      params,
      metadata,
    ]),
  ).toEqual(
    Array(2)
      .fill([
        ['click', { selector: '#go' }, { ...NO_TIMEOUT, timeout: 123 }],
        ['textContent', { selector: '#go' }, { ...NO_TIMEOUT, timeout: 123 }],
      ])
      .flat(),
  )
})

/** Each method that takes options, its arguments before them, and its schema keys. */
const METHODS: [string, unknown[], string[]][] = [
  [
    'click',
    [],
    [
      'selector',
      'strict',
      'force',
      'scroll',
      'noWaitAfter',
      'modifiers',
      'position',
      'delay',
      'button',
      'clickCount',
      'trial',
      'steps',
    ],
  ],
  [
    'dblclick',
    [],
    [
      'selector',
      'strict',
      'force',
      'scroll',
      'modifiers',
      'position',
      'delay',
      'button',
      'trial',
      'steps',
    ],
  ],
  [
    'hover',
    [],
    ['selector', 'strict', 'force', 'scroll', 'modifiers', 'position', 'trial'],
  ],
  [
    'tap',
    [],
    ['selector', 'strict', 'force', 'scroll', 'modifiers', 'position', 'trial'],
  ],
  ['fill', ['text'], ['selector', 'strict', 'value', 'force']],
  ['press', ['Enter'], ['selector', 'strict', 'key', 'delay', 'noWaitAfter']],
  ['inputValue', [], ['selector', 'strict']],
  ['isChecked', [], ['selector', 'strict']],
  ['isVisible', [], ['selector', 'strict']],
  ['check', [], ['selector', 'strict', 'force', 'scroll', 'position', 'trial']],
  [
    'uncheck',
    [],
    ['selector', 'strict', 'force', 'scroll', 'position', 'trial'],
  ],
  [
    'setChecked',
    [true],
    ['selector', 'strict', 'force', 'scroll', 'position', 'trial'],
  ],
  [
    'selectOption',
    ['one'],
    ['selector', 'strict', 'elements', 'options', 'force'],
  ],
  ['textContent', [], ['selector', 'strict']],
  ['innerText', [], ['selector', 'strict']],
  ['getAttribute', ['title'], ['selector', 'strict', 'name']],
]

test('tests that a schema key set to null rejects with Error and sends nothing, on every receiver', async () => {
  const { page, fake } = start()
  for (const [scope, target] of [
    ['page', page],
    ['frame', page.mainFrame()],
    ['locator', page.locator('#go')],
  ] as const) {
    const methods =
      scope === 'locator'
        ? [
            ...METHODS,
            ['clear', [], ['selector', 'strict', 'value', 'force']] as const,
          ]
        : METHODS
    for (const [method, args, keys] of methods) {
      const operation = (
        target as unknown as Record<
          string,
          ((...args: unknown[]) => Promise<unknown>) | undefined
        >
      )[method]
      if (!operation) throw new Error(`${scope} has no ${method}`)
      const head = scope === 'locator' ? [] : ['#go']
      for (const key of keys) {
        expect([
          `${scope}.${method} ${key}`,
          await rejection(() =>
            operation.apply(target, [...head, ...args, { [key]: null }]),
          ),
        ]).toEqual([`${scope}.${method} ${key}`, 'Error'])
      }
    }
  }
  expect(fake.messages).toEqual([])
})

test('tests that page-level calls reject a null schema key with Error and send nothing', async () => {
  const { page, fake } = start()
  for (const key of ['mode', 'depth', 'boxes'])
    expect(await rejection(() => page.ariaSnapshot({ [key]: null }))).toBe(
      'Error',
    )
  expect(await rejection(() => page.close({ reason: null as never }))).toBe(
    'Error',
  )
  for (const target of [page, page.mainFrame()])
    for (const key of ['url', 'waitUntil', 'referer'])
      expect(
        await rejection(() =>
          target.goto('https://run.test/', { [key]: null }),
        ),
      ).toBe('Error')
  expect(fake.messages).toEqual([])
  expect(page.isClosed()).toBe(false)
})

test('tests that a null timeout or unknown key is ignored, as neither is a schema key', async () => {
  const { page, fake } = start()
  page.setDefaultTimeout(123)
  await page.click('#go', { timeout: null, unknown: null } as never)
  await page.locator('#go').fill('text', { timeout: null } as never)
  await page.close({ unknown: null } as never)
  expect(
    fake.messages.map(({ method, params, metadata }) => [
      method,
      params,
      metadata,
    ]),
  ).toEqual([
    ['click', { selector: '#go' }, { ...NO_TIMEOUT, timeout: 123 }],
    [
      'fill',
      { selector: '#go', strict: true, value: 'text' },
      { ...NO_TIMEOUT, timeout: 123 },
    ],
    ['close', {}, NO_TIMEOUT],
  ])
})

test('tests that null options objects throw TypeError where Playwright reads them first', async () => {
  const { page, fake } = start()
  expect(await rejection(() => page.ariaSnapshot(null as never))).toBe(
    'TypeError',
  )
  expect(await rejection(() => page.close(null as never))).toBe('TypeError')
  expect(
    await rejection(() => page.goto('https://run.test/', null as never)),
  ).toBe('TypeError')
  expect(() => page.getByRole('button', null as never)).toThrow(TypeError)
  expect(fake.messages).toEqual([])
})

test('tests that pickParams rejects null like the validator, drops undefined and ignores other keys', () => {
  expect(() =>
    pickParams(['selector', 'force'] as const, {
      selector: '#go',
      force: null,
    }),
  ).toThrow('force: ')
  expect(() => pickParams(['selector'] as const, { selector: null })).toThrow(
    Error,
  )
  expect(
    pickParams(['selector', 'force'] as const, {
      selector: '#go',
      force: undefined,
      timeout: null,
      unknown: null,
    }),
  ).toEqual({ selector: '#go' })
})

/**
 * Playwright's server validates every call's metadata (`dispatcher.ts`, `tOptional(tFloat)` for
 * `timeout`): after `setDefaultTimeout(null)` a timed call resolves its timeout to `null` and
 * rejects with a plain `Error` before it runs; a call without a timeout still runs.
 */
test('tests that a null default timeout rejects timed calls before they run', async () => {
  for (const reset of [
    (page: ReturnType<typeof start>['page']) =>
      page.setDefaultTimeout(null as never),
    (page: ReturnType<typeof start>['page']) =>
      page.context().setDefaultTimeout(null as never),
  ]) {
    const { page, fake } = start()
    reset(page)
    fake.answer = ({ method }) =>
      method === 'title' ? { result: { value: 'Title' } } : {}
    expect(await rejection(() => page.textContent('#go'))).toBe('Error')
    expect(await rejection(() => page.locator('#go').click())).toBe('Error')
    expect(await rejection(() => page.goto('http://a.test/'))).toBe('Error')
    expect(await page.title()).toBe('Title')
    expect(fake.messages.map(({ method }) => method)).toEqual(['title'])
  }
  const { page, fake } = start()
  page.setDefaultNavigationTimeout(null as never)
  fake.answer = () => ({})
  expect(await rejection(() => page.goto('http://a.test/'))).toBe('Error')
  expect(await page.locator('#go').click()).toBeUndefined()
  expect(fake.messages.map(({ method }) => method)).toEqual(['click'])
})
