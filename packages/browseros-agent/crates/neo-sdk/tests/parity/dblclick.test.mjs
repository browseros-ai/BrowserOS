// Double clicks through Neo and through Playwright on fresh tests/pages/pointer-actions.html
// pages. Every receiver returns undefined, and the page saw the same trusted events on the same
// elements: two presses with click counts 1 and 2 and the `dblclick` they make (the regression
// for a writer that repeats one click), with the button asked for. A disabled element is waited
// for until the timeout (unlike hover), a covered one is never pressed, a hidden one times out
// and a strict locator over two matches fails at once.
//
// From Playwright's specs at the pin: tests/page/page-click.spec.ts 'CL-028 should double click
// the button', tests/page/locator-click.spec.ts 'LO-054 should double click the button'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { assertDecoysUntouched, compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

test('dblclick returns undefined and presses twice with rising click counts', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/pointer-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.locator('#target').dblclick() === undefined,
        await page.dblclick('.dup', null) === undefined,
        await page.mainFrame().dblclick('#other', { button: 'right' }) === undefined,
        await page.locator('#target').dblclick({ delay: 30, position: { x: 4, y: 4 } }) === undefined,
      ]
      const failures = {}
      for (const [name, act] of [
        ['strict', () => page.locator('.dup').dblclick()],
        ['disabled', () => page.locator('#disabled').dblclick({ timeout: 500 })],
        ['hidden', () => page.locator('#hidden').dblclick({ timeout: 500 })],
        ['covered', () => page.locator('#covered').dblclick({ timeout: 1000 })],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { voids, failures }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    voids: [true, true, true, true],
    failures: {
      strict: 'Error:strict',
      disabled: 'TimeoutError',
      hidden: 'TimeoutError',
      covered: 'TimeoutError',
    },
  })
  assert.deepEqual(
    outcome.calls.map((call) => call.delivery),
    [
      'done',
      'done',
      'done',
      'done',
      'notSent',
      'notSent',
      'notSent',
      'notSent',
    ],
    "Neo's delivery records",
  )
})

/**
 * Double clicks inside frames, on tabs of frame-top.html (frames.mjs): the Send button of the
 * cross-site out-of-process frame, of its 127.0.0.1 child and, through a child Frame, of the
 * same-origin frame two levels deep get two trusted clicks each, and no decoy frame sees one.
 */
const frameCases = [
  {
    name: 'a double click reaches the Send button of each frame and no decoy',
    source: `const login = page.frameLocator('#login-iframe')
    return {
      cross: await settle(() => login.locator('#send').dblclick()),
      grandchild: await settle(() => login.frameLocator('#grandchild').locator('#send').dblclick()),
      frame: await settle(() => page.frame('chain-inner').dblclick('#send')),
    }`,
    expected: {
      cross: 'undefined',
      grandchild: 'undefined',
      frame: 'undefined',
    },
    effect: (effect) => {
      const touched = assertDecoysUntouched(effect, [
        'cross',
        'grandchild',
        'chain-inner',
      ])
      for (const [who, facts] of Object.entries(touched))
        assert.deepEqual(
          facts.events.filter((event) => event.type === 'click'),
          [
            { type: 'click', target: 'send', trusted: true },
            { type: 'click', target: 'send', trusted: true },
          ],
          `${who}'s clicks`,
        )
    },
    deliveries: { dblclick: ['done', 'done', 'done'] },
  },
]

test('dblclick inside frames reaches the same element through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
