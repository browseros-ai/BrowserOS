// A page's dedicated workers through the pinned Playwright and through Neo (events.mjs): the
// `worker` event, `page.workers()`, `worker.url()`, `evaluate` and `evaluateHandle` in the
// worker's own context, the worker's handles, its `close` when it terminates, when its page
// navigates and when its page closes, and `Worker.waitForEvent`. Cases from Playwright's
// tests/page/workers.spec.ts at 26137face, named by their ids, plus Neo's own: a worker already
// running before the run first looks, the worker's handles, `waitForEvent('close')`, and
// `context.backgroundPages()`, which is always empty.
//
// Each case orders its worker's life through the page itself (a `postMessage` handshake, an
// awaited `terminate()`), never through sleeps. Runs under Node, not Bun, on the shared harness
// (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** Playwright's tests/assets/worker/worker.html and the part of its worker.js the cases read. */
const WORKER_HTML =
  "<!DOCTYPE html><title>Worker test</title><script>var worker = new Worker('worker.js')</script>"
const WORKER_JS = {
  body: "function workerFunction() {\n  return 'worker function result';\n}\n",
  headers: { 'content-type': 'application/javascript; charset=utf-8' },
}

/** The run's origin, from the page's URL: QuickJS has no WHATWG `URL`. */
const ORIGIN = 'const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]\n'

/** Starts a blob worker running `body` from the page. */
const spawn = (body) =>
  `page.evaluate(() => { window.spawned = new Worker(URL.createObjectURL(new Blob([${JSON.stringify(body)}], { type: 'application/javascript' }))) })`

const cases = [
  {
    name: 'WO-001 Page.workers',
    path: '/empty.html',
    source: `${ORIGIN}const none = page.workers().length
    const [created] = await Promise.all([
      page.waitForEvent('worker'),
      page.goto(origin + '/worker/worker.html'),
    ])
    const worker = page.workers()[0]
    const same = worker === created
    const url = path(worker.url())
    const value = await worker.evaluate(() => self['workerFunction']())
    await page.goto(origin + '/empty.html')
    return { none, same, url, value, after: page.workers().length }`,
    expected: {
      none: 0,
      same: true,
      url: '/worker/worker.js',
      value: 'worker function result',
      after: 0,
    },
  },
  {
    name: 'WO-002 should emit created and destroyed events',
    path: '/empty.html',
    source: `const workerCreated = page.waitForEvent('worker')
    const workerObj = await page.evaluateHandle(() => new Worker(URL.createObjectURL(new Blob(['1'], { type: 'application/javascript' }))))
    const worker = await workerCreated
    const workerThisObj = await worker.evaluateHandle('this')
    const destroyed = new Promise((resolve) => worker.once('close', resolve))
    await page.evaluate((workerObj) => workerObj.terminate(), workerObj)
    const closedWith = await destroyed
    const error = await workerThisObj.getProperty('self').catch((error) => error)
    return {
      same: closedWith === worker,
      listed: page.workers().length,
      api: error.message.includes('jsHandle.getProperty'),
      closed: error.message.includes('Target page, context or browser has been closed'),
      kind: errorCategory(error),
    }`,
    expected: { same: true, listed: 0, api: true, closed: true, kind: 'Error' },
  },
  {
    name: 'WO-003 should reject in-flight evaluate and emit close when page closes',
    path: '/empty.html',
    source: `const other = await page.context().newPage()
    await other.goto(page.url())
    const [worker] = await Promise.all([
      other.waitForEvent('worker'),
      other.evaluate(() => { window.spawned = new Worker(URL.createObjectURL(new Blob(['1'], { type: 'application/javascript' }))) }),
    ])
    const closed = new Promise((resolve) => worker.once('close', resolve))
    // The worker answers first, so the next evaluation is in flight, not waiting to start.
    const ready = await worker.evaluate(() => 'ready')
    const evaluation = worker.evaluate(() => new Promise(() => {})).catch((error) => error)
    await other.close()
    const error = await evaluation
    return {
      ready,
      api: error.message.includes('worker.evaluate'),
      kind: errorCategory(error),
      same: (await closed) === worker,
    }`,
    expected: { ready: 'ready', api: true, kind: 'Error', same: true },
  },
  {
    name: 'WO-008 should evaluate',
    path: '/empty.html',
    source: `const workerCreated = page.waitForEvent('worker')
    await ${spawn('console.log(1)')}
    const worker = await workerCreated
    return await worker.evaluate('1+1')`,
    expected: 2,
  },
  {
    name: 'WO-012 should clear upon navigation',
    path: '/empty.html',
    source: `${ORIGIN}const workerCreated = page.waitForEvent('worker')
    await ${spawn('console.log(1)')}
    const worker = await workerCreated
    const before = page.workers().length
    let destroyed = false
    worker.once('close', () => (destroyed = true))
    await page.goto(origin + '/one-style.html')
    return { before, destroyed, after: page.workers().length }`,
    expected: { before: 1, destroyed: true, after: 0 },
  },
  {
    name: 'WO-013 should clear upon cross-process navigation',
    path: '/empty.html',
    source: `${ORIGIN}const workerCreated = page.waitForEvent('worker')
    await ${spawn('console.log(1)')}
    const worker = await workerCreated
    const before = page.workers().length
    let destroyed = false
    worker.once('close', () => (destroyed = true))
    await page.goto(origin.replace('127.0.0.1', 'localhost') + '/empty.html')
    return { before, destroyed, after: page.workers().length }`,
    expected: { before: 1, destroyed: true, after: 0 },
  },
  {
    name: 'a worker running before the first look is listed and evaluates',
    path: '/empty.html',
    source: `await page.evaluate(() => new Promise((resolve) => {
      const worker = new Worker(URL.createObjectURL(new Blob(['postMessage(1)'], { type: 'application/javascript' })))
      worker.onmessage = () => { window.spawned = worker; resolve() }
    }))
    const workers = page.workers()
    const again = page.workers()
    return {
      count: workers.length,
      same: workers[0] === again[0],
      value: await workers[0].evaluate(() => 6 * 7),
    }`,
    expected: { count: 1, same: true, value: 42 },
  },
  {
    name: "a worker's handles live in the worker",
    path: '/empty.html',
    source: `const workerCreated = page.waitForEvent('worker')
    await ${spawn('1')}
    const worker = await workerCreated
    await worker.evaluate(() => { self.data = { a: 1, b: [2, 3] } })
    const data = await worker.evaluateHandle(() => self.data)
    const five = await worker.evaluateHandle(() => 5)
    const result = {
      json: await data.jsonValue(),
      a: await (await data.getProperty('a')).jsonValue(),
      missing: typeof (await (await data.getProperty('none')).jsonValue()),
      props: [...(await data.getProperties()).keys()],
      sum: await worker.evaluate((d) => d.a + d.b[0], data),
      onHandle: await data.evaluate((d) => d.b.length),
      byHandle: await (await data.evaluateHandle((d) => d.b)).jsonValue(),
      primitive: await five.jsonValue(),
      viaPage: await page.evaluate((x) => x * 2, five),
      foreign: await page.evaluate((d) => d, data).then(
        () => 'resolved',
        (error) => error.message.includes('JSHandles can be evaluated only in the context they were created!'),
      ),
      previews: [String(data), String(five)],
      isElement: data.asElement(),
    }
    await data.dispose()
    return result`,
    expected: {
      json: { a: 1, b: [2, 3] },
      a: 1,
      missing: 'undefined',
      props: ['a', 'b'],
      sum: 3,
      onHandle: 2,
      byHandle: [2, 3],
      primitive: 5,
      viaPage: 10,
      foreign: true,
      previews: ['Object', '5'],
      isElement: null,
    },
  },
  {
    name: "Worker.waitForEvent('close') resolves with the worker; another event rejects",
    path: '/empty.html',
    source: `const workerCreated = page.waitForEvent('worker')
    const workerObj = await page.evaluateHandle(() => new Worker(URL.createObjectURL(new Blob(['1'], { type: 'application/javascript' }))))
    const worker = await workerCreated
    const other = worker.waitForEvent('my-event').catch((error) => error)
    const [closedWith] = await Promise.all([
      worker.waitForEvent('close'),
      page.evaluate((workerObj) => workerObj.terminate(), workerObj),
    ])
    const error = await other
    return {
      same: closedWith === worker,
      api: error.message.includes('worker.waitForEvent'),
      closed: error.message.includes('Target page, context or browser has been closed'),
      after: await worker.evaluate('1').catch((error) => error.message.includes('Target page, context or browser has been closed')),
    }`,
    expected: { same: true, api: true, closed: true, after: true },
  },
  {
    name: 'context.backgroundPages() is empty',
    path: '/empty.html',
    source: 'return page.context().backgroundPages()',
    expected: [],
  },
]

test('dedicated workers through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/worker/worker.html': WORKER_HTML,
      '/worker/worker.js': WORKER_JS,
    },
    cases,
  })
})
