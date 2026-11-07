/**
 * Every method Neo serves that takes Playwright's `signal` option refuses one before sending
 * anything (`TimeoutSettings.timeout`/`navigationTimeout`, the one place a call resolves its
 * options into a timeout): an already-aborted signal and a live one alike reject with an `Error`
 * prefixed with the method's api name, and no message reaches the runtime, so the browser sees
 * nothing. Playwright aborts the call instead (`AbortError` for an aborted signal), which Neo
 * cannot do for a sent call yet. `null` and `undefined` are no signal, as Playwright reads them
 * (supermediator DECIDED 01:27): the call is sent.
 */

import { expect, test } from 'bun:test'
import { start } from './fake-runtime'

const SIGNALS: [string, unknown][] = [
  ['an aborted signal', { aborted: true, reason: new Error('stop') }],
  [
    'a live signal',
    { aborted: false, addEventListener() {}, removeEventListener() {} },
  ],
]

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

for (const [family, calls] of [
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
] as const) {
  test(`tests that every ${family} method refuses a signal before sending anything`, async () => {
    const { page, fake } = start()
    const sent = fake.messages.length
    for (const [label, signal] of SIGNALS) {
      for (const [apiName, call] of calls(page)) {
        const error = await call(signal as never).then(
          () => null,
          (error: Error) => error,
        )
        expect(error?.name, `${apiName} with ${label}`).toBe('Error')
        expect(error?.message, `${apiName} with ${label}`).toBe(
          `${apiName}: Neo does not support the "signal" option yet`,
        )
      }
    }
    expect(fake.messages.slice(sent)).toEqual([])
  })
}

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
