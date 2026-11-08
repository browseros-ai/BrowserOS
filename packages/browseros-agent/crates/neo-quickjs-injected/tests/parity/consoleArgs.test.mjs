// ConsoleMessage.args() through the pinned Playwright and through Neo, on fresh
// tests/pages/console.html tabs (events.mjs): a message's arguments are handles of what
// console.log received, in order, whose jsonValue() is each value; the same handle comes back at
// every read; an object is a JSHandle and a node an ElementHandle; and a browser log entry has
// none; a child frame's arguments live in that frame; and a received message's arguments outlive
// its page. Neo makes the handles when args() first reads them (console-message.ts, Deviation): a
// message read from consoleMessages() kept no arguments, which a Neo-only case pins.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-event-console.spec.ts "CM-001 should work @smoke" (args()[i].jsonValue()).
    name: "a message's arguments are handles of its values, the same ones at every read",
    path: '/console.html',
    source: `const [message] = await Promise.all([page.waitForEvent('console'), page.click('#format')])
    const args = message.args()
    const values = []
    for (const arg of args) values.push(await arg.jsonValue())
    return {
      count: args.length,
      values,
      same: message.args()[2] === args[2],
      fresh: message.args() !== args,
      kinds: args.map((arg) => arg.asElement() === null),
    }`,
    expected: {
      count: 4,
      values: ['a', 1, { x: 1 }, [1, 2]],
      same: true,
      fresh: true,
      kinds: [true, true, true, true],
    },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // page-event-console.spec.ts "CM-011 should use object previews for arrays and objects",
    // "CM-006 should not fail for window object": every argument, primitives and the window
    // included, is a handle, of the value V8 reports after formatting.
    name: 'primitives, objects and the window are each a handle',
    path: '/console.html',
    source: `const [message] = await Promise.all([page.waitForEvent('console'), page.click('#previews')])
    const args = message.args()
    return {
      count: args.length,
      // As strings: a run's value travels as JSON, which has no undefined in an array.
      primitives: [String(await args[3].jsonValue()), String(await args[4].jsonValue()), String(await args[5].jsonValue())],
      objects: [await args[1].jsonValue(), await args[2].jsonValue()],
      window: await settle(args[6].evaluate((value) => value === window)),
    }`,
    expected: {
      count: 7,
      primitives: ['null', 'undefined', 'true'],
      // V8 applies the format's %s and %d to the arguments they take before it reports them.
      objects: ['[object Object]', 1],
      window: { value: true },
    },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // Playwright: server/chromium/crExecutionContext.ts createHandle (a node is an ElementHandle);
    // page-event-console.spec.ts "CM-001 should work @smoke" for the string beside it.
    name: 'a node argument is an ElementHandle of that node',
    path: '/console.html',
    source: `const [message] = await Promise.all([page.waitForEvent('console'), page.click('#element')])
    const [label, body] = message.args()
    return {
      label: await label.jsonValue(),
      element: body.asElement() !== null,
      tag: await body.evaluate((node) => node.tagName),
    }`,
    expected: { label: 'body', element: true, tag: 'BODY' },
    probe: 'window.__logged',
    effect: 3,
  },
  {
    // Neo only (console-message.ts, Deviation): a message read from consoleMessages() kept no
    // arguments, and its args() says so instead of answering an empty list.
    name: 'a message read from consoleMessages() refuses args()',
    path: '/console.html',
    source: `await page.click('#log')
    const messages = await page.consoleMessages()
    const last = messages[messages.length - 1]
    let refused
    try {
      last.args()
    } catch (error) {
      refused = errorCategory(error)
    }
    return { text: last.text(), refused }`,
    neo: (outcome) =>
      assert.deepEqual(outcome.value, { text: 'click 1', refused: 'Error' }),
  },
  {
    // Reviewer's case (review-review-a3.md P2 2): an argument of a same-origin child frame's
    // message is a handle in that frame, so evaluating it runs in the child's document.
    name: "a child frame's argument evaluates in the child's document",
    path: '/console-host.html',
    source: `const child = page.frame({ name: 'child' })
    const message = page.waitForEvent('console')
    await child.evaluate(() => console.log(17))
    const arg = (await message).args()[0]
    const realm = await arg.evaluate(() => {
      document.body.setAttribute('data-edge', 'changed')
      return window.marker
    })
    return {
      value: await arg.jsonValue(),
      realm,
      parent: await page.getAttribute('body', 'data-edge'),
      child: await child.getAttribute('body', 'data-edge'),
    }`,
    expected: { value: 17, realm: 'CHILD', parent: null, child: 'changed' },
  },
  {
    // Reviewer's case (review-review-a3.md P2 3): the arguments a listener received outlive the
    // page; using an object's handle may then fail, as Playwright's do.
    name: "a received message's arguments outlive its page",
    path: '/console.html',
    source: `const message = page.waitForEvent('console')
    await page.evaluate(() => console.log(7, { x: 1 }))
    const held = await message
    await page.close()
    return { length: held.args().length, again: held.args()[1] === held.args()[1] }`,
    expected: { length: 2, again: true },
  },
]

test('console arguments are the same handles through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/console.html': fixture('console.html'),
      '/console-host.html':
        '<!doctype html><title>host</title><script>window.marker = "PARENT"</script><iframe name="child" src="/console-child.html"></iframe>',
      '/console-child.html':
        '<!doctype html><title>child</title><script>window.marker = "CHILD"</script><body>child</body>',
    },
    cases,
  })
})
