// The EventEmitter methods of Page and BrowserContext through the pinned Playwright and through
// Neo, on fresh tests/pages/console.html tabs (events.mjs), with real console events: on, once,
// prependListener and prependOnceListener run in Node's order and return their emitter;
// listeners() unwraps a once listener where rawListeners() does not; listenerCount, eventNames
// and max listeners (0 from the start, as ChannelOwner sets them); a listener that is not a
// function or a bad maximum is refused. Playwright's Page also holds its own once('close') and
// once('crash') from the start, and its BrowserContext its own once('close'), which eventNames()
// lists; Neo's hold none (an internal of Playwright's, a Deviation in page.ts), so the names
// compared leave those two out.
// removeAllListeners stops delivery, and with behavior 'wait' waits for running listeners. A
// listener's error goes to the emitter's 'error' listeners and dispatch goes on; with none,
// emit throws it. That last rule, for an event from the browser, ends Neo's run: in Playwright's
// Node process the same throw escapes the connection's message handler, an uncaught exception
// this test cannot survive, so that case runs through Neo alone.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/events/prepend.spec.ts "EE-030 EventEmitter functionality";
    // listeners.spec.ts "EE-015 Array copy modification does not modify orig", "EE-017
    // listeners and once", "EE-021 raw listeners"; listener-count.spec.ts "EE-012 Listener
    // count test"; events-list.spec.ts "EE-011 should maintain event names correctly";
    // once.spec.ts "EE-028 should work"; add-listeners.spec.ts "EE-007 listener type check";
    // max-listeners.spec.ts "EE-023 emit maxListeners on e".
    name: 'listeners register, run and leave in Node order',
    path: '/console.html',
    source: `const calls = []
    const on = () => calls.push('on')
    const prepended = () => calls.push('prepend')
    const onced = () => calls.push('once')
    const prependedOnce = () => calls.push('prependOnce')
    const listeners = [on, prepended, onced, prependedOnce]
    const which = (listener) => ['on', 'prepend', 'once', 'prependOnce'][listeners.indexOf(listener)] ?? null
    const names = (emitter) => emitter.eventNames().filter((name) => name !== 'close' && name !== 'crash')
    const initial = { names: names(page), contextNames: names(page.context()), console: page.listenerCount('console') }
    const returnPage = [
      page.on('console', on),
      page.prependListener('console', prepended),
      page.once('console', onced),
      page.prependOnceListener('console', prependedOnce),
    ].every((value) => value === page)
    const copy = page.listeners('console')
    copy.push(on)
    const copyLeftAlone = [copy.length, page.listenerCount('console')]
    const registered = {
      count: page.listenerCount('console'),
      listeners: page.listeners('console').map(which),
      rawListeners: page.rawListeners('console').map((listener) => [which(listener), which(listener.listener)]),
      names: names(page),
    }
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const firstClick = calls.splice(0)
    const afterOnce = page.listeners('console').map(which)
    const removed = () => calls.push('removed')
    page.once('console', removed)
    const removal = [
      page.removeListener('console', removed),
      page.off('console', prepended),
      page.addListener('console', prepended),
    ].every((value) => value === page)
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const secondClick = calls.splice(0)
    page.off('console', on).off('console', prepended)
    const browserContext = page.context()
    const contextListener = () => {}
    const contextListeners = [
      browserContext.on('console', contextListener) === browserContext,
      browserContext.listenerCount('console'),
      browserContext.off('console', contextListener) === browserContext,
      browserContext.listenerCount('console'),
    ]
    const refused = (call) => {
      try {
        const value = call()
        return value === page ? 'page' : value
      } catch (error) {
        return { error: errorCategory(error) }
      }
    }
    return {
      initial,
      returnPage,
      copyLeftAlone,
      registered,
      firstClick,
      afterOnce,
      removal,
      secondClick,
      contextListeners,
      notAFunction: [
        refused(() => page.on('console', null)),
        refused(() => page.once('console', 'listener')),
        refused(() => page.prependListener('console', {})),
        refused(() => page.off('console', undefined)),
      ],
      maxListeners: {
        page: page.getMaxListeners(),
        context: browserContext.getMaxListeners(),
        set: refused(() => page.setMaxListeners(3)),
        read: page.getMaxListeners(),
        negative: refused(() => page.setMaxListeners(-1)),
        notANumber: refused(() => page.setMaxListeners(NaN)),
        string: refused(() => page.setMaxListeners('7')),
        unchanged: page.getMaxListeners(),
      },
      end: { console: page.listenerCount('console'), names: names(page), unknown: page.listenerCount('no-such-event') },
    }`,
    expected: {
      initial: {
        names: [],
        contextNames: [],
        console: 0,
      },
      returnPage: true,
      copyLeftAlone: [5, 4],
      registered: {
        count: 4,
        listeners: ['prependOnce', 'prepend', 'on', 'once'],
        // A once listener is a wrapper whose `listener` is the function given.
        rawListeners: [
          [null, 'prependOnce'],
          ['prepend', null],
          ['on', null],
          [null, 'once'],
        ],
        names: ['console'],
      },
      firstClick: ['prependOnce', 'prepend', 'on', 'once'],
      afterOnce: ['prepend', 'on'],
      removal: true,
      secondClick: ['on', 'prepend'],
      contextListeners: [true, 1, true, 0],
      notAFunction: [
        { error: 'TypeError' },
        { error: 'TypeError' },
        { error: 'TypeError' },
        { error: 'TypeError' },
      ],
      maxListeners: {
        page: 0,
        context: 0,
        set: 'page',
        read: 3,
        negative: { error: 'RangeError' },
        notANumber: { error: 'RangeError' },
        string: { error: 'RangeError' },
        unchanged: 3,
      },
      end: { console: 0, names: [], unknown: 0 },
    },
    probe: 'window.__logged',
    effect: 4,
  },
  {
    // tests/library/events/remove-all-listeners.spec.ts "EE-036 listeners", "EE-040
    // removeAllListeners returns EventEmitter"; remove-all-listeners-wait.spec.ts "EE-033
    // should wait", "EE-034 should wait all".
    name: "removeAllListeners stops delivery, and behavior 'wait' waits for running listeners",
    path: '/console.html',
    source: `const seen = []
    page.on('console', (message) => seen.push('first ' + message.text()))
    page.on('console', (message) => seen.push('second ' + message.text()))
    page.on('pageerror', () => {})
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const returnsPage = page.removeAllListeners('console') === page
    const left = [page.listenerCount('console'), page.listenerCount('pageerror')]
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const delivered = [...seen]

    let release
    const released = new Promise((resolve) => { release = resolve })
    let started
    const running = new Promise((resolve) => { started = resolve })
    let value = 0
    page.on('console', async () => {
      started()
      await released
      value = 42
    })
    await page.click('#log')
    await running
    const removing = page.removeAllListeners('console', { behavior: 'wait' })
    const whileWaiting = { isPromise: removing instanceof Promise, count: page.listenerCount('console'), value }
    release()
    const waited = await settle(removing)

    let releaseAll
    const releasedAll = new Promise((resolve) => { releaseAll = resolve })
    let startedCount = 0
    let bothStarted
    const bothRunning = new Promise((resolve) => { bothStarted = resolve })
    const finished = []
    const slow = (name) => async () => {
      if (++startedCount === 2) bothStarted()
      await releasedAll
      finished.push(name)
    }
    page.on('console', slow('console'))
    page.on('pageerror', slow('pageerror'))
    await page.click('#log')
    await page.click('#throw')
    await bothRunning
    const removingAll = page.removeAllListeners(undefined, { behavior: 'wait' })
    const finishedBeforeRelease = [...finished]
    releaseAll()
    const waitedAll = await settle(removingAll)
    return {
      returnsPage,
      left,
      delivered,
      whileWaiting,
      waited,
      value,
      finishedBeforeRelease,
      waitedAll,
      finished: finished.sort(),
      namesAfter: page.eventNames(),
    }`,
    // With no type, removeAllListeners takes Playwright's own close and crash listeners too.
    expected: {
      returnsPage: true,
      left: [0, 1],
      delivered: ['first click 1', 'second click 1'],
      whileWaiting: { isPromise: true, count: 0, value: 0 },
      waited: 'undefined',
      value: 42,
      finishedBeforeRelease: [],
      waitedAll: 'undefined',
      finished: ['console', 'pageerror'],
      namesAfter: [],
    },
    probe: 'window.__logged',
    effect: 6,
  },
  {
    // tests/library/events/listener-errors.spec.ts "EE-057 should report synchronous listener
    // errors and continue dispatch", "EE-058 should capture rejections from promises and
    // thenables".
    name: "a listener's error goes to 'error' listeners, or to emit's caller",
    path: '/console.html',
    source: `const caught = (call) => {
      try {
        return { value: call() }
      } catch (error) {
        return { error: errorCategory(error), message: error.message }
      }
    }
    page.on('boom', () => {
      throw new TypeError('listener boom')
    })
    const unhandled = caught(() => page.emit('boom'))
    const errors = []
    page.on('error', (error) => errors.push(errorCategory(error) + ': ' + error.message))
    page.on('boom', () => errors.push('next listener'))
    const handled = caught(() => page.emit('boom'))
    const afterEmit = errors.splice(0)
    const seen = []
    page.on('console', () => {
      throw new RangeError('console listener boom')
    })
    page.on('console', (message) => seen.push(message.text()))
    await Promise.all([page.waitForEvent('console'), page.click('#log')])
    const afterClick = errors.splice(0)
    page.removeAllListeners('console')
    page.on('console', async () => {
      throw new SyntaxError('async listener boom')
    })
    const [rejection] = await Promise.all([page.waitForEvent('error'), page.click('#log')])
    return {
      unhandled,
      handled,
      afterEmit,
      afterClick,
      seen,
      rejection: errorCategory(rejection) + ': ' + rejection.message,
      emitWithoutListeners: page.emit('nothing-listens'),
    }`,
    expected: {
      unhandled: { error: 'TypeError', message: 'listener boom' },
      handled: { value: true },
      afterEmit: ['TypeError: listener boom', 'next listener'],
      afterClick: ['RangeError: console listener boom'],
      seen: ['click 1'],
      rejection: 'SyntaxError: async listener boom',
      emitWithoutListeners: false,
    },
    probe: 'window.__logged',
    effect: 4,
  },
  {
    // listener-errors.spec.ts "EE-057 should report synchronous listener errors and continue
    // dispatch" (`expect(() => ee.emit('throw')).toThrow(error)`): with no 'error' listener,
    // client/eventEmitter.ts `_handleListenerError` rethrows. Neo only (see the header).
    name: "with no 'error' listener, a throwing listener ends Neo's run with its error",
    path: '/console.html',
    source: `page.on('console', () => {
      throw new TypeError('listener boom')
    })
    await page.click('#log')
    await page.waitForEvent('console', { timeout: 2000 }).catch(() => {})
    return 'the run went on'`,
    neo: (outcome) => {
      assert.equal(outcome.status, 'error', JSON.stringify(outcome))
      assert.deepEqual(
        { name: outcome.error.name, message: outcome.error.message },
        { name: 'TypeError', message: 'listener boom' },
      )
    },
  },
]

test('Page and BrowserContext emit events the way Node EventEmitter does through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/console.html': fixture('console.html') },
    cases,
  })
})
