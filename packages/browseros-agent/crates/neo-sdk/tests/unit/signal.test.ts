/**
 * Every method Neo serves that takes Playwright's `signal` option honours it as Playwright's
 * client does (`client/connection.ts` `sendMessageToServer`): an already-aborted signal rejects
 * with `AbortError` (its `cause` the signal's reason, its message prefixed with the api name)
 * before anything is sent, so the browser sees nothing. A live signal goes with the call, and its
 * abort reaches the runtime as `abort(id, reason)` for that call's message; the `AbortError`
 * reply is rebuilt with the same `cause`. `null` and `undefined` are no signal (supermediator
 * DECIDED 01:27): the call is sent and nothing listens.
 */

import { expect, test } from 'bun:test'
import { start } from './fake-runtime'

type Call = [apiName: string, call: (signal: never) => Promise<unknown>]

/** Options carrying `signal`, which no Neo option type declares: the methods take them anyway,
 * as a script may pass them. */
const withSignal = (signal: never) => ({ signal }) as never

function locatorCalls(page: ReturnType<typeof start>['page']): Call[] {
  const locator = page.locator('#target')
  return [
    ['locator.click', (signal) => locator.click(withSignal(signal))],
    ['locator.dblclick', (signal) => locator.dblclick(withSignal(signal))],
    ['locator.hover', (signal) => locator.hover(withSignal(signal))],
    ['locator.tap', (signal) => locator.tap(withSignal(signal))],
    [
      'locator.textContent',
      (signal) => locator.textContent(withSignal(signal)),
    ],
    ['locator.innerText', (signal) => locator.innerText(withSignal(signal))],
    ['locator.innerHTML', (signal) => locator.innerHTML(withSignal(signal))],
    [
      'locator.getAttribute',
      (signal) => locator.getAttribute('id', withSignal(signal)),
    ],
    [
      'locator.ariaSnapshot',
      (signal) => locator.ariaSnapshot(withSignal(signal)),
    ],
    ['locator.fill', (signal) => locator.fill('x', withSignal(signal))],
    ['locator.clear', (signal) => locator.clear(withSignal(signal))],
    ['locator.press', (signal) => locator.press('a', withSignal(signal))],
    ['locator.inputValue', (signal) => locator.inputValue(withSignal(signal))],
    ['locator.isChecked', (signal) => locator.isChecked(withSignal(signal))],
    ['locator.isEnabled', (signal) => locator.isEnabled(withSignal(signal))],
    ['locator.isDisabled', (signal) => locator.isDisabled(withSignal(signal))],
    ['locator.isEditable', (signal) => locator.isEditable(withSignal(signal))],
    ['locator.check', (signal) => locator.check(withSignal(signal))],
    ['locator.uncheck', (signal) => locator.uncheck(withSignal(signal))],
    [
      'locator.setChecked',
      (signal) => locator.setChecked(true, withSignal(signal)),
    ],
    [
      'locator.selectOption',
      (signal) => locator.selectOption('a', withSignal(signal)),
    ],
    ['locator.waitFor', (signal) => locator.waitFor(withSignal(signal))],
  ]
}

/** `page.*` and `frame.*`: the same selector methods on both, plus Page's own. */
function selectorCalls(
  name: 'page' | 'frame',
  // biome-ignore lint/suspicious/noExplicitAny: Page and Frame share these methods' shapes
  target: any,
): Call[] {
  return [
    [
      `${name}.goto`,
      (signal) => target.goto('https://run.test/next', withSignal(signal)),
    ],
    [
      `${name}.waitForURL`,
      (signal) => target.waitForURL('**/next', withSignal(signal)),
    ],
    [
      `${name}.waitForLoadState`,
      (signal) => target.waitForLoadState('load', withSignal(signal)),
    ],
    [`${name}.click`, (signal) => target.click('#target', withSignal(signal))],
    [
      `${name}.dblclick`,
      (signal) => target.dblclick('#target', withSignal(signal)),
    ],
    [`${name}.hover`, (signal) => target.hover('#target', withSignal(signal))],
    [`${name}.tap`, (signal) => target.tap('#target', withSignal(signal))],
    [
      `${name}.textContent`,
      (signal) => target.textContent('#target', withSignal(signal)),
    ],
    [
      `${name}.innerText`,
      (signal) => target.innerText('#target', withSignal(signal)),
    ],
    [
      `${name}.innerHTML`,
      (signal) => target.innerHTML('#target', withSignal(signal)),
    ],
    [
      `${name}.getAttribute`,
      (signal) => target.getAttribute('#target', 'id', withSignal(signal)),
    ],
    [
      `${name}.fill`,
      (signal) => target.fill('#target', 'x', withSignal(signal)),
    ],
    [
      `${name}.press`,
      (signal) => target.press('#target', 'a', withSignal(signal)),
    ],
    [
      `${name}.inputValue`,
      (signal) => target.inputValue('#target', withSignal(signal)),
    ],
    [
      `${name}.isChecked`,
      (signal) => target.isChecked('#target', withSignal(signal)),
    ],
    [
      `${name}.isEnabled`,
      (signal) => target.isEnabled('#target', withSignal(signal)),
    ],
    [
      `${name}.isDisabled`,
      (signal) => target.isDisabled('#target', withSignal(signal)),
    ],
    [
      `${name}.isEditable`,
      (signal) => target.isEditable('#target', withSignal(signal)),
    ],
    [`${name}.check`, (signal) => target.check('#target', withSignal(signal))],
    [
      `${name}.uncheck`,
      (signal) => target.uncheck('#target', withSignal(signal)),
    ],
    [
      `${name}.setChecked`,
      (signal) => target.setChecked('#target', true, withSignal(signal)),
    ],
    [
      `${name}.selectOption`,
      (signal) => target.selectOption('#target', 'a', withSignal(signal)),
    ],
  ]
}

const FAMILIES = [
  ['Locator', (page: ReturnType<typeof start>['page']) => locatorCalls(page)],
  [
    'Page',
    (page: ReturnType<typeof start>['page']) => [
      ...selectorCalls('page', page),
      [
        'page.ariaSnapshot',
        (signal: never) => page.ariaSnapshot(withSignal(signal)),
      ] as Call,
    ],
  ],
  [
    'Frame',
    (page: ReturnType<typeof start>['page']) =>
      selectorCalls('frame', page.mainFrame()),
  ],
] as const

for (const [family, calls] of FAMILIES) {
  test(`tests that every ${family} method rejects an aborted signal with AbortError before sending anything`, async () => {
    const { page, fake } = start()
    const sent = fake.messages.length
    const reason = new Error('stop')
    for (const [apiName, call] of calls(page)) {
      const error = await call({ aborted: true, reason } as never).then(
        () => null,
        (error: Error) => error,
      )
      expect(error?.name, apiName).toBe('AbortError')
      expect(error?.message, apiName).toBe(
        `${apiName}: The operation was aborted`,
      )
      expect(error?.cause, apiName).toBe(reason)
    }
    expect(fake.messages.slice(sent)).toEqual([])
  })

  test(`tests that every ${family} method aborts its call when a live signal aborts`, async () => {
    const { page, fake } = start()
    fake.hold = true
    for (const [apiName, call] of calls(page)) {
      const controller = new AbortController()
      const reason = new Error('stop')
      const settled = call(controller.signal as never).then(
        () => null,
        (error: Error) => error,
      )
      const id = fake.messages.at(-1)?.id ?? -1
      controller.abort(reason)
      const error = await settled
      expect(fake.aborts.at(-1), apiName).toEqual({ id, reason: 'stop' })
      expect(error?.name, apiName).toBe('AbortError')
      expect(error?.message.startsWith(`${apiName}: stop`), apiName).toBe(true)
      expect(error?.cause, apiName).toBe(reason)
    }
  })
}

test('tests that a reason that is not an Error goes to the runtime as its string', async () => {
  const { page, fake } = start()
  fake.hold = true
  const controller = new AbortController()
  const settled = page
    .click('#target', withSignal(controller.signal as never))
    .catch((error: Error) => error)
  const id = fake.messages.at(-1)?.id ?? -1
  controller.abort(42)
  const error = await settled
  expect(fake.aborts).toEqual([{ id, reason: '42' }])
  expect((error as Error).cause).toBe(42)
})

test('tests that the abort listener is removed once the call settles, so a late abort sends nothing', async () => {
  const { page, fake } = start()
  const listening: string[] = []
  let listener: (() => void) | undefined
  const signal = {
    aborted: false,
    reason: undefined,
    addEventListener(type: string, added: () => void, options: unknown) {
      listening.push(`add ${type} ${JSON.stringify(options)}`)
      listener = added
    },
    removeEventListener(type: string, removed: () => void) {
      listening.push(`remove ${type} ${removed === listener}`)
    },
  }
  await page.click('#target', withSignal(signal as never))
  expect(listening).toEqual(['add abort {"once":true}', 'remove abort true'])
  const controller = new AbortController()
  await page.click('#target', withSignal(controller.signal as never))
  controller.abort()
  expect(fake.aborts).toEqual([])
})

test('tests that a signal without addEventListener throws a TypeError after the call is sent, as Playwright', async () => {
  const { page, fake } = start()
  const sent = fake.messages.length
  const error = await page
    .click('#target', withSignal({ aborted: false } as never))
    .then(
      () => null,
      (error: Error) => error,
    )
  expect(error).toBeInstanceOf(TypeError)
  expect(error?.message.startsWith('page.click: ')).toBe(true)
  expect(fake.messages.slice(sent).map(({ method }) => method)).toEqual([
    'click',
  ])
})

test('tests that waitForEvent rejects with AbortError for an aborted signal and when one aborts', async () => {
  const { page } = start()
  const reason = new Error('stop')
  const early = await page
    .waitForEvent('console', { signal: { aborted: true, reason } } as never)
    .then(
      () => null,
      (error: Error) => error,
    )
  expect(early?.name).toBe('AbortError')
  expect(early?.message.startsWith('page.waitForEvent: ')).toBe(true)
  expect(early?.cause).toBe(reason)
  const controller = new AbortController()
  const waiting = page
    .waitForEvent('console', { signal: controller.signal } as never)
    .then(
      () => null,
      (error: Error) => error,
    )
  controller.abort(reason)
  const late = await waiting
  expect(late?.name).toBe('AbortError')
  expect(late?.cause).toBe(reason)
})

test('tests that no signal, an undefined one or null still sends the call', async () => {
  const { page, fake } = start()
  const sent = fake.messages.length
  await page
    .locator('#target')
    .click(withSignal(undefined as never))
    .catch(() => {})
  await page
    .locator('#target')
    .click(withSignal(null as never))
    .catch(() => {})
  await page.click('#target', withSignal(null as never)).catch(() => {})
  await page
    .mainFrame()
    .textContent('#target', withSignal(null as never))
    .catch(() => {})
  await page.click('#target').catch(() => {})
  expect(fake.messages.slice(sent).map(({ method }) => method)).toEqual([
    'click',
    'click',
    'click',
    'textContent',
    'click',
  ])
})
