// `context.newCDPSession` and the `CDPSession` it returns, through Neo and through Playwright on
// one BrowserOS neo: a session of the script's own on the page's target (or its main frame's)
// answers what it is sent and hands back its events under each method name and as `event`; a
// method Chromium does not know fails naming it; a detached session fails as closed and emits
// `close`; `timeout` and `signal` stop a call; a same-process frame has no session of its own,
// and anything but a page or frame is refused. What Neo refuses on purpose, the methods that run
// or pause page JavaScript, leave the page, or send input around Neo's own paths
// (neo-runtime `cdp_session.rs` `REFUSED`), is asserted on each engine's side: Playwright
// forwards them, Neo fails naming the method, with no effect on the page.
//
// Playwright cases (pin 26137face): `tests/library/chromium/session.spec.ts` "CD-038 should
// respect timeout and signal options", "CD-039 should send events", "CD-040 should only accept a
// page or frame", "CD-042 should be able to detach session", "CD-043 should throw nice errors",
// "CD-044 should work with main frame", "CD-045 should throw if target is part of main", "CD-049
// should emit event for each CDP event" and "CD-050 should emit close event when session is
// detached", with Neo-allowed methods where the spec sends `Runtime.evaluate`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/cdp.html': '<title>cdp</title><p>cdp</p>',
  '/framed.html': '<title>framed</title><iframe src="/cdp.html"></iframe>',
  // Answered once the case has given up on it.
  '/slow': (request, response) => {
    setTimeout(() => response.end('<title>slow</title>'), 4000)
  },
}

const FIRST_LINE = `const firstLine = (error) => [error.name, error.message.split('\\n')[0]]
const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length))
`

const cases = [
  {
    name: 'answers what it is sent and hands back its events',
    source: `${FIRST_LINE}const client = await context.newCDPSession(page)
await client.send('Network.enable')
const named = []
const all = []
client.on('Network.requestWillBeSent', (event) => named.push(event.request.url))
client.on('event', (event) => all.push(event.method))
await page.goto(origin + '/framed.html')
const tree = await client.send('Page.getFrameTree')
await client.detach()
return {
  // The browser's own favicon fetch comes when it comes, in either engine.
  named: named.map((url) => url.slice(origin.length)).filter((path) => path !== '/favicon.ico'),
  sawRequest: all.includes('Network.requestWillBeSent'),
  frame: tree.frameTree.frame.url.slice(origin.length),
  children: tree.frameTree.childFrames.length,
}`,
    expected: {
      named: ['/framed.html', '/cdp.html'],
      sawRequest: true,
      frame: '/framed.html',
      children: 1,
    },
  },
  {
    name: 'works on the main frame, and fails on a same-process frame',
    source: `${FIRST_LINE}await page.goto(origin + '/framed.html')
const main = await context.newCDPSession(page.mainFrame())
const { root } = await main.send('DOM.getDocument')
await main.detach()
const child = await context.newCDPSession(page.frames()[1]).then(() => 'opened', firstLine)
const notAPage = await context.newCDPSession(context).then(() => 'opened', firstLine)
return { root: root.nodeName, child, notAPage }`,
    expected: {
      root: '#document',
      child: [
        'Error',
        "browserContext.newCDPSession: This frame does not have a separate CDP session, it is a part of the parent frame's session",
      ],
      notAPage: ['Error', 'page: expected Page or Frame'],
    },
  },
  {
    name: 'a method Chromium does not know fails naming it',
    source: `${FIRST_LINE}const client = await context.newCDPSession(page)
const error = await client.send('ThisCommand.DoesNotExist').catch((error) => error)
await client.detach()
return { name: error.name, named: error.message.includes('ThisCommand.DoesNotExist') }`,
    expected: { name: 'Error', named: true },
  },
  {
    name: 'a detached session emits close and fails as closed',
    source: `${FIRST_LINE}const client = await context.newCDPSession(page)
let closed = null
client.on('close', (session) => { closed = session })
await client.send('Page.getFrameTree')
await client.detach()
const after = await client.send('Page.getFrameTree').then(() => 'sent', firstLine)
const again = await client.detach().then(() => 'detached', firstLine)
return { closed: closed === client, after, again }`,
    expected: {
      closed: true,
      // Playwright's client `TargetClosedError` keeps `Error` as its name (`client/errors.ts`).
      after: [
        'Error',
        'cdpSession.send: Target page, context or browser has been closed',
      ],
      again: [
        'Error',
        'cdpSession.detach: Target page, context or browser has been closed',
      ],
    },
  },
  {
    name: 'timeout and signal stop a call',
    source: `${FIRST_LINE}const client = await context.newCDPSession(page)
const timedOut = await client.send('Page.navigate', { url: origin + '/slow' }, { timeout: 500 }).then(() => 'sent', (error) => error.name)
const controller = new AbortController()
const reason = new Error('Aborted by user')
const pending = client.send('Page.navigate', { url: origin + '/slow' }, { signal: controller.signal }).catch((error) => error)
controller.abort(reason)
const aborted = await pending
await client.detach()
return { timedOut, aborted: aborted.name, cause: aborted.cause === reason }`,
    expected: { timedOut: 'TimeoutError', aborted: 'AbortError', cause: true },
  },
]

/** The methods Neo refuses, each sent once; the page's title says whether any ran. */
const REFUSED = `${FIRST_LINE}const client = await context.newCDPSession(page)
const tries = [
  ['Runtime.evaluate', { expression: 'document.title = "evaluated"', returnByValue: true }],
  ['Page.navigate', { url: 'javascript:document.title = "navigated"' }],
  ['Debugger.pause', {}],
  ['Input.dispatchKeyEvent', { type: 'keyDown', key: 'a' }],
  ['Target.getTargets', {}],
  ['DOM.setOuterHTML', { nodeId: 1, outerHTML: '<html></html>' }],
]
const answers = []
for (const [method, params] of tries) {
  answers.push(await client.send(method, params).then(() => 'sent', (error) => error.message.includes(method) ? 'refused naming it' : 'failed: ' + error.message))
}
await client.detach()
return answers`

test('context.newCDPSession: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/cdp.html', source))
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/cdp.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})

test('context.newCDPSession: Neo refuses what Playwright forwards', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const tried = `${REFUSED.replace('return answers', 'return { answers, title: await page.title() }')}`
    const reference = outcomeValue(await playwright('/cdp.html', tried))
    assert.equal(
      reference.answers[0],
      'sent',
      `Playwright forwards: ${JSON.stringify(reference)}`,
    )
    assert.deepEqual(
      outcomeValue(await neo('/cdp.html', tried)),
      {
        answers: Array(6).fill('refused naming it'),
        title: 'cdp',
      },
      'Neo refuses each, naming it, and the page is untouched',
    )
  })
})
