/**
 * Every method Neo serves that takes Playwright's `signal` option refuses one before sending
 * anything (`TimeoutSettings.timeout`/`navigationTimeout`, the one place a call resolves its
 * options into a timeout): an already-aborted signal, a live one and `null` alike reject with an
 * `Error` prefixed with the method's api name, and no message reaches the runtime, so the
 * browser sees nothing. Playwright aborts the call instead (`AbortError` for an aborted signal),
 * which Neo cannot do for a sent call yet.
 */

import { expect, test } from 'bun:test'
import { start } from './fake-runtime'

const SIGNALS: [string, unknown][] = [
  ['an aborted signal', { aborted: true, reason: new Error('stop') }],
  [
    'a live signal',
    { aborted: false, addEventListener() {}, removeEventListener() {} },
  ],
  ['null', null],
]

type Call = [apiName: string, call: (signal: never) => Promise<unknown>]

function locatorCalls(page: ReturnType<typeof start>['page']): Call[] {
  const locator = page.locator('#target')
  return [
    ['locator.click', (signal) => locator.click({ signal })],
    ['locator.dblclick', (signal) => locator.dblclick({ signal })],
    ['locator.hover', (signal) => locator.hover({ signal })],
    ['locator.tap', (signal) => locator.tap({ signal })],
    ['locator.textContent', (signal) => locator.textContent({ signal })],
    ['locator.innerText', (signal) => locator.innerText({ signal })],
    ['locator.innerHTML', (signal) => locator.innerHTML({ signal })],
    [
      'locator.getAttribute',
      (signal) => locator.getAttribute('id', { signal }),
    ],
    ['locator.ariaSnapshot', (signal) => locator.ariaSnapshot({ signal })],
    ['locator.fill', (signal) => locator.fill('x', { signal })],
    ['locator.clear', (signal) => locator.clear({ signal })],
    ['locator.press', (signal) => locator.press('a', { signal })],
    ['locator.inputValue', (signal) => locator.inputValue({ signal })],
    ['locator.isChecked', (signal) => locator.isChecked({ signal })],
    ['locator.isEnabled', (signal) => locator.isEnabled({ signal })],
    ['locator.isDisabled', (signal) => locator.isDisabled({ signal })],
    ['locator.isEditable', (signal) => locator.isEditable({ signal })],
    ['locator.check', (signal) => locator.check({ signal })],
    ['locator.uncheck', (signal) => locator.uncheck({ signal })],
    ['locator.setChecked', (signal) => locator.setChecked(true, { signal })],
    ['locator.selectOption', (signal) => locator.selectOption('a', { signal })],
    ['locator.waitFor', (signal) => locator.waitFor({ signal })],
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
      (signal) => target.goto('https://run.test/next', { signal }),
    ],
    [
      `${name}.waitForURL`,
      (signal) => target.waitForURL('**/next', { signal }),
    ],
    [
      `${name}.waitForLoadState`,
      (signal) => target.waitForLoadState('load', { signal }),
    ],
    [`${name}.click`, (signal) => target.click('#target', { signal })],
    [`${name}.dblclick`, (signal) => target.dblclick('#target', { signal })],
    [`${name}.hover`, (signal) => target.hover('#target', { signal })],
    [`${name}.tap`, (signal) => target.tap('#target', { signal })],
    [
      `${name}.textContent`,
      (signal) => target.textContent('#target', { signal }),
    ],
    [`${name}.innerText`, (signal) => target.innerText('#target', { signal })],
    [`${name}.innerHTML`, (signal) => target.innerHTML('#target', { signal })],
    [
      `${name}.getAttribute`,
      (signal) => target.getAttribute('#target', 'id', { signal }),
    ],
    [`${name}.fill`, (signal) => target.fill('#target', 'x', { signal })],
    [`${name}.press`, (signal) => target.press('#target', 'a', { signal })],
    [
      `${name}.inputValue`,
      (signal) => target.inputValue('#target', { signal }),
    ],
    [`${name}.isChecked`, (signal) => target.isChecked('#target', { signal })],
    [`${name}.isEnabled`, (signal) => target.isEnabled('#target', { signal })],
    [
      `${name}.isDisabled`,
      (signal) => target.isDisabled('#target', { signal }),
    ],
    [
      `${name}.isEditable`,
      (signal) => target.isEditable('#target', { signal }),
    ],
    [`${name}.check`, (signal) => target.check('#target', { signal })],
    [`${name}.uncheck`, (signal) => target.uncheck('#target', { signal })],
    [
      `${name}.setChecked`,
      (signal) => target.setChecked('#target', true, { signal }),
    ],
    [
      `${name}.selectOption`,
      (signal) => target.selectOption('#target', 'a', { signal }),
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
        (signal: never) => page.ariaSnapshot({ signal }),
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
          `${apiName}: Neo does not support the signal option yet`,
        )
      }
    }
    expect(fake.messages.slice(sent)).toEqual([])
  })
}

test('tests that no signal, or an undefined one, still sends the call', async () => {
  const { page, fake } = start()
  const sent = fake.messages.length
  await page
    .locator('#target')
    .click({ signal: undefined })
    .catch(() => {})
  await page.click('#target').catch(() => {})
  expect(fake.messages.slice(sent).map(({ method }) => method)).toEqual([
    'click',
    'click',
  ])
})
