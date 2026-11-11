/** Native navigation completion and event publication cross different async reply paths. */
import { expect, it } from 'bun:test'
import { Publications } from '../src/prelude/publication'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

const tree = {
  frames: [{ id: 'main', parentId: null, name: '', url: 'https://a.example/' }],
}
const frame = {
  id: 'main',
  parentId: null,
  name: '',
  url: 'https://a.example/next',
}

it('publishes ready navigation wrappers without joining pending browser work', async () => {
  const publications = new Publications()
  const browser = deferred<void>()
  const order: string[] = []
  const pending = publications.track(async () => {
    await browser.promise
  })
  const completed = publications.track(async (ready) => {
    ready()
    return 'done'
  })
  void completed.then(() => {
    order.push('navigation')
  })
  await Promise.resolve()
  await publications.publishReady()
  order.push('evaluation')
  expect(order).toEqual(['navigation', 'evaluation'])
  browser.resolve()
  await pending
})

it('publishes an earlier unfiltered navigation wait before completing its click', async () => {
  const candidate = deferred<unknown>()
  const clicked = deferred<void>()
  const runtime = loadPrelude({
    seed: SEED,
    now: 100,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'events.wait') return candidate.promise
      if (name === 'locator.click') {
        clicked.resolve()
        return { navigation: { seq: 8, at: 100 } }
      }
      return null
    },
  })
  const result = runtime.run(`
    const order = []
    const wait = page.waitForEvent('framenavigated').then(() => order.push('navigation'))
    const click = page.click('a').then(() => order.push('click'))
    await Promise.all([wait, click])
    return order
  `)
  await clicked.promise
  // Delay the events.wait reply until the native click reply already arrived.
  await new Promise((resolve) => setTimeout(resolve, 10))
  candidate.resolve({
    seq: 8,
    at: 100,
    event: 'framenavigated',
    page: { $neo: 'page', tab: 1007 },
    frame,
    context: false,
  })
  expect(json(await result)).toEqual(['navigation', 'click'])
})

it('publishes an earlier unfiltered context navigation wait before completing its click', async () => {
  const candidate = deferred<unknown>()
  const clicked = deferred<void>()
  const runtime = loadPrelude({
    seed: SEED,
    now: 100,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'events.wait') return candidate.promise
      if (name === 'locator.click') {
        clicked.resolve()
        return { navigation: { seq: 8, at: 100 } }
      }
      return null
    },
  })
  const result = runtime.run(`
    const order = []
    const wait = context.waitForEvent('framenavigated').then(() => order.push('navigation'))
    const click = page.click('a').then(() => order.push('click'))
    await Promise.all([wait, click])
    return order
  `)
  await clicked.promise
  // Delay the events.wait reply until the native click reply already arrived.
  await new Promise((resolve) => setTimeout(resolve, 10))
  candidate.resolve({
    seq: 8,
    at: 100,
    event: 'framenavigated',
    page: { $neo: 'page', tab: 1007 },
    frame,
    context: true,
  })
  expect(json(await result)).toEqual(['navigation', 'click'])
})

it('does not join a navigation predicate that itself awaits the click', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    now: 100,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'events.wait')
        return {
          seq: 8,
          at: 100,
          event: 'framenavigated',
          page: { $neo: 'page', tab: 1007 },
          frame,
          context: false,
        }
      if (name === 'locator.click') return { navigation: { seq: 8, at: 100 } }
      return null
    },
  })
  expect(
    await runtime.run(`
    const click = page.click('a')
    const event = page.waitForEvent('framenavigated', async () => { await click; return true })
    await Promise.all([click, event])
    return true
  `),
  ).toBe(true)
})

it('does not join a navigation wait that started after the script received that navigation', async () => {
  let waits = 0
  const runtime = loadPrelude({
    seed: SEED,
    now: 100,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'events.wait') {
        waits += 1
        if (waits > 1) return new Promise(() => {})
        return {
          seq: 8,
          at: 100,
          event: 'framenavigated',
          page: { $neo: 'page', tab: 1007 },
          frame,
          context: false,
        }
      }
      if (name === 'locator.click') return { navigation: { seq: 8, at: 100 } }
      return null
    },
  })
  const result = await runtime.run(`
    await page.waitForEvent('framenavigated')
    // Starts after seq 8, in the same millisecond: it will never receive that navigation.
    page.waitForEvent('framenavigated').catch(() => {})
    try {
      await page.click('a', { timeout: 200 })
      return 'clicked'
    } catch (error) {
      return error.message
    }
  `)
  expect(json(result)).toBe('clicked')
})

it('publishes page and context navigation listeners before a click resolves', async () => {
  const batch = deferred<unknown>()
  const clicked = deferred<void>()
  let next = false
  const runtime = loadPrelude({
    seed: SEED,
    now: 100,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'events.next') {
        if (!next) {
          next = true
          return batch.promise
        }
        return new Promise(() => {})
      }
      if (name === 'locator.click') {
        clicked.resolve()
        return { navigation: { seq: 8, at: 100 } }
      }
      return null
    },
  })
  const result = runtime.run(`
    const order = []
    const pageListener = () => order.push('page')
    const contextListener = () => order.push('context')
    page.on('framenavigated', pageListener)
    context.on('framenavigated', contextListener)
    await page.click('a')
    order.push('click')
    page.off('framenavigated', pageListener)
    context.off('framenavigated', contextListener)
    return order
  `)
  await clicked.promise
  batch.resolve({
    events: [false, true].map((context) => ({
      seq: 8,
      at: 100,
      event: 'framenavigated',
      page: { $neo: 'page', tab: 1007 },
      frame,
      context,
    })),
    lost: 0,
  })
  expect(json(await result)).toEqual(['page', 'context', 'click'])
})

it('keeps navigation metadata in its native reply and publishes goto before later evaluation', async () => {
  const navigation = deferred<unknown>()
  const entered = deferred<void>()
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'page.goto') {
        entered.resolve()
        return navigation.promise
      }
      if (name === 'page.evaluate') {
        navigation.resolve({
          url: frame.url,
          response: null,
          frames: [frame],
          frameSnapshotAttempted: true,
        })
        return { value: { type: 'undefined' }, frames: [frame] }
      }
      return null
    },
  })
  const result = runtime.run(`
    let done = false
    const goto = page.goto('https://a.example/next').then(() => done = true)
    await page.evaluate(() => window.stop())
    const immediate = done
    await goto
    return { immediate, url: page.url() }
  `)
  await entered.promise
  expect(json(await result)).toEqual({ immediate: true, url: frame.url })
  expect(runtime.sent('page.frames')).toHaveLength(0)
})

for (const lostBeforeClick of [false, true]) {
  it(`rejects a lost navigation publication with no timeout, lostBeforeClick=${lostBeforeClick}`, async () => {
    const batch = deferred<unknown>()
    const clicked = deferred<void>()
    let next = false
    const runtime = loadPrelude({
      seed: SEED,
      now: 100,
      answer: ({ name }) => {
        if (name === 'page.frames') return tree
        if (name === 'events.next') {
          if (!next) {
            next = true
            return batch.promise
          }
          return new Promise(() => {})
        }
        if (name === 'locator.click') {
          clicked.resolve()
          return { navigation: { seq: 8, at: 100 } }
        }
        return null
      },
    })
    if (lostBeforeClick)
      batch.resolve({
        ok: { events: [], lost: 1 },
        observed: { seq: 2, events: 8 },
      })
    const result = runtime.run(`
      const listener = () => {}
      page.on('framenavigated', listener)
      try {
        await page.click('a', { timeout: 0 })
        return 'accepted'
      } catch (e) {
        return { lost: e.message.includes('Navigation events were lost'), delivery: e.delivery, retryable: e.retryable }
      } finally { page.off('framenavigated', listener) }
    `)
    await clicked.promise
    if (!lostBeforeClick)
      batch.resolve({
        ok: { events: [], lost: 1 },
        observed: { seq: 2, events: 8 },
      })
    expect(json(await result)).toEqual({
      lost: true,
      delivery: 'unknown',
      retryable: false,
    })
  })
}

it('publishes a receipted navigation whose native JS reply arrives after evaluate', async () => {
  const navigation = deferred<unknown>()
  const entered = deferred<void>()
  let id = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name, input }) => {
      if (name === 'page.frames') return tree
      if (name === 'page.goto') {
        id = Reflect.get(Object(input.publication), 'id')
        entered.resolve()
        return navigation.promise
      }
      if (name === 'page.evaluate')
        return entered.promise.then(() => {
          // The producer finished in Rust already; its NAPI callback is independently queued.
          setTimeout(
            () =>
              navigation.resolve({
                ok: {
                  url: frame.url,
                  response: null,
                  frames: [frame],
                  frameSnapshotAttempted: true,
                },
                pages: [{ tab: 1007 }],
                observed: { seq: 3, events: 1, payloadCurrent: true },
              }),
            10,
          )
          return {
            ok: { value: null, frames: [frame] },
            pages: [{ tab: 1007 }],
            observed: { seq: 4, events: 1, payloadCurrent: true },
            completedNavigations: [id],
          }
        })
      return null
    },
  })
  expect(
    await runtime.run(`
    let done = false
    const goto = page.goto('https://a.example/next').then(() => done = true)
    await page.evaluate(() => window.stop())
    const immediate = done
    await goto
    return immediate
  `),
  ).toBe(true)
  expect(runtime.sent('page.frames')).toHaveLength(0)
})

it('does not join a native navigation that has no completion receipt', async () => {
  const navigation = deferred<unknown>()
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) => {
      if (name === 'page.frames') return tree
      if (name === 'page.goto') return navigation.promise
      if (name === 'page.evaluate') return { value: null }
      return null
    },
  })
  const pending = runtime.run(`
    let done = false
    page.goto('https://a.example/stall').then(() => done = true)
    await page.evaluate(() => window.stop())
    return done
  `)
  expect(await pending).toBe(false)
  navigation.resolve({ response: null, frameSnapshotAttempted: true })
})
