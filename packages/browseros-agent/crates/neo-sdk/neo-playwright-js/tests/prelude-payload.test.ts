// Host observations order facts and payloads independently from async reply arrival.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude } from './support/prelude-runtime'

const brand = { $neo: 'page', tab: 51 }
const url = 'https://current.test/'
const live = { tab: 51, url, closed: false }
const closed = { tab: 51, closed: true }
const seed = { page: brand, pages: [live] }
const never = new Promise<never>(() => {})
const tree = (child: string, mainUrl = url) => ({
  frames: [
    { id: 'main', parentId: null, name: '', url: mainUrl },
    { id: child, parentId: 'main', name: child, url: `${mainUrl}${child}` },
  ],
})
const closeRecord = (seq: number) => ({
  seq,
  at: 1000,
  event: 'close',
  page: brand,
})
const consoleRecord = (seq: number) => ({
  seq,
  at: 1000,
  event: 'console',
  page: brand,
  message: { type: 'log', text: 'message', args: [], location: {} },
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

describe('page payload freshness', () => {
  it('keeps a re-admitted page current when an earlier navigation arrives, preserving its Response', async () => {
    const entered = deferred<void>()
    const delayed = deferred<unknown>()
    let titles = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'page.frames') return tree('current')
        if (name === 'page.goto') {
          entered.resolve()
          return delayed.promise
        }
        if (name === 'page.title')
          return {
            ok: 'Title',
            pages: [++titles === 1 ? closed : live],
            observed: { seq: titles + 2, events: 0 },
          }
        return null
      },
    })
    const pending = runtime.run(
      "return (await page.goto('https://earlier.test/')).status()",
    )
    await entered.promise
    await runtime.run('await page.title(); await page.title()')
    delayed.resolve({
      ok: {
        ...tree('earlier', 'https://earlier.test/'),
        url: 'https://earlier.test/',
        response: {
          url: 'https://earlier.test/',
          status: 201,
          statusText: 'Created',
          headers: {},
        },
      },
      pages: [{ ...live, url: 'https://earlier.test/' }],
      observed: { seq: 2, events: 0, payloadCurrent: true },
    })
    expect(await pending).toBe(201)
    expect(await runtime.run('return page.url()')).toBe(url)
    expect(
      json(await runtime.run('return page.frames().map(f => f.name())')),
    ).toEqual(['', 'current'])
    expect(runtime.sent('page.frames')).toHaveLength(1)
  })

  it('rejects a same-URL payload from a replaced target and reads frames on the current binding', async () => {
    const entered = deferred<void>()
    const fresh = deferred<unknown>()
    let reads = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'page.frames') {
          if (++reads === 1) return { ok: tree('initial'), pages: [live] }
          entered.resolve()
          return fresh.promise
        }
        if (name === 'page.goto')
          return {
            ok: { url, ...tree('obsolete') },
            pages: [live],
            observed: { seq: 5, events: 0, payloadCurrent: false },
          }
        return null
      },
    })
    const pending = runtime.run(`await page.goto(${JSON.stringify(url)})`)
    await entered.promise
    expect(
      json(await runtime.run('return page.frames().map(f => f.name())')),
    ).toEqual(['', 'initial'])
    fresh.resolve({ ok: tree('current'), pages: [live] })
    await pending
    expect(runtime.sent('page.frames')).toHaveLength(1)
    expect(
      json(await runtime.run('return page.frames().map(f => f.name())')),
    ).toEqual(['', 'current'])
  })

  it('discards a frame tree read that finishes after exclusion and re-admission', async () => {
    const entered = deferred<void>()
    const delayed = deferred<unknown>()
    let reads = 0
    let titles = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'page.frames') {
          if (++reads === 1) return tree('initial')
          entered.resolve()
          return delayed.promise
        }
        if (name === 'page.goto') return { ok: { url }, pages: [live] }
        if (name === 'page.title')
          return {
            ok: 'Title',
            pages: [++titles === 1 ? closed : live],
            observed: { seq: titles + 3, events: 0 },
          }
        return null
      },
    })
    const pending = runtime.run(`await page.goto(${JSON.stringify(url)})`)
    await entered.promise
    await runtime.run('await page.title(); await page.title()')
    delayed.resolve({
      ok: tree('obsolete'),
      pages: [live],
      observed: { seq: 3, events: 0, payloadCurrent: true },
    })
    await pending
    expect(
      json(await runtime.run('return page.frames().map(f => f.name())')),
    ).toEqual(['', 'initial'])
  })

  it('applies a current navigation payload without a redundant frame read', async () => {
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'page.frames') return tree('initial')
        if (name === 'page.goto')
          return {
            ok: { url, ...tree('navigated') },
            pages: [live],
            observed: { seq: 2, events: 0, payloadCurrent: true },
          }
        return null
      },
    })
    await runtime.run(`await page.goto(${JSON.stringify(url)})`)
    expect(runtime.sent('page.frames')).toHaveLength(0)
    expect(
      json(await runtime.run('return page.frames().map(f => f.name())')),
    ).toEqual(['', 'navigated'])
  })
})

describe('host-approved close positions', () => {
  for (const approval of [5, 7]) {
    it(`closes at the approved record ${approval} among two lives of one tab`, async () => {
      const entered = deferred<void>()
      const delayed = deferred<unknown>()
      let pumps = 0
      const runtime = loadPrelude({
        seed,
        answer: ({ name }) => {
          if (name === 'events.next' && ++pumps === 1) {
            entered.resolve()
            return delayed.promise
          }
          if (name === 'events.next') return never
          return null
        },
      })
      const pending = runtime.run(`
        const seen = []
        return await new Promise(resolve => {
          page.on('console', () => seen.push('console:' + page.isClosed()))
          page.on('close', () => { seen.push('close:' + page.isClosed()); Promise.resolve().then(() => resolve(seen)) })
        })
      `)
      await entered.promise
      delayed.resolve({
        ok: {
          events: [closeRecord(5), consoleRecord(6), closeRecord(7)],
          lost: 0,
        },
        pages: [closed],
        observed: {
          seq: 20,
          events: 1000,
          closes: [{ tab: 51, seq: approval }],
        },
      })
      expect(json(await pending)).toEqual(
        approval === 5
          ? ['close:true', 'console:true']
          : ['console:false', 'close:true'],
      )
      expect(await runtime.run('return page.isClosed()')).toBe(true)
    })
  }

  it('lets an earlier console callback observe an open page before the batch closes it once', async () => {
    const entered = deferred<void>()
    const delayed = deferred<unknown>()
    let pumps = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next' && ++pumps === 1) {
          entered.resolve()
          return delayed.promise
        }
        if (name === 'events.next') return never
        return null
      },
    })
    const pending = runtime.run(`
      const seen = []
      return await new Promise(resolve => {
        page.on('console', () => seen.push(page.isClosed()))
        page.on('close', () => { seen.push(page.isClosed()); resolve(seen) })
      })
    `)
    await entered.promise
    delayed.resolve({
      ok: { events: [consoleRecord(1), closeRecord(2)], lost: 0 },
      pages: [closed],
      observed: { seq: 10, events: 2, closes: [{ tab: 51, seq: 2 }] },
    })
    expect(json(await pending)).toEqual([false, true])
  })

  it('drops an old approved close after a newer live fact and settles a missing approved record', async () => {
    const delayed = deferred<unknown>()
    const entered = deferred<void>()
    let pumps = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next' && ++pumps === 1) {
          entered.resolve()
          return delayed.promise
        }
        if (name === 'events.next') return never
        if (name === 'page.title')
          return {
            ok: 'Title',
            pages: [live],
            observed: { seq: 10, events: 8 },
          }
        return null
      },
    })
    await runtime.run(
      'page.on("close", () => {}); page.on("console", () => {})',
    )
    await entered.promise
    await runtime.run('await page.title()')
    delayed.resolve({
      ok: { events: [closeRecord(5)], lost: 0 },
      pages: [closed],
      observed: { seq: 2, events: 8, closes: [{ tab: 51, seq: 5 }] },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(await runtime.run('return page.isClosed()')).toBe(false)

    let waits = 0
    const missing = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next' && ++waits === 1)
          return {
            ok: { events: [], lost: 0 },
            pages: [closed],
            observed: { seq: 10, events: 8, closes: [{ tab: 51, seq: 5 }] },
          }
        if (name === 'events.next') return never
        return null
      },
    })
    expect(
      await missing.run(
        'await new Promise(resolve => page.on("close", resolve)); return page.isClosed()',
      ),
    ).toBe(true)
  })

  it('settles a closed fact when an event read fails after copying its close', async () => {
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) =>
        name === 'events.wait'
          ? {
              err: {
                code: 'cancelled',
                message: 'Cancelled',
                owner: 'neo',
                delivery: 'not_sent',
                retryable: false,
              },
              pages: [closed],
              observed: { seq: 20, events: 7, closes: [{ tab: 51, seq: 7 }] },
            }
          : null,
    })
    expect(
      await runtime.run(
        "try { await page.waitForEvent('close') } catch {} return page.isClosed()",
      ),
    ).toBe(true)
  })

  it('settles one approved close before either concurrent wait predicate runs', async () => {
    const entered = deferred<void>()
    const replies: Array<(value: unknown) => void> = []
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next') return never
        if (name !== 'events.wait') return null
        return new Promise((resolve) => {
          replies.push(resolve)
          if (replies.length === 2) entered.resolve()
        })
      },
    })
    const pending = runtime.run(`
      let count = 0
      const seen = []
      page.on('close', () => count++)
      const first = page.waitForEvent('close', p => { seen.push(p.isClosed()); return p.isClosed() })
      const second = page.waitForEvent('close', p => { seen.push(p.isClosed()); return p.isClosed() })
      const values = await Promise.all([first, second])
      return { seen, count, same: values.every(p => p === page), closed: page.isClosed() }
    `)
    await entered.promise
    for (const [index, resolve] of replies.entries())
      resolve({
        ok: closeRecord(7),
        pages: [closed],
        observed: { seq: index + 10, events: 7, closes: [{ tab: 51, seq: 7 }] },
      })
    expect(json(await pending)).toEqual({
      seen: [true, true],
      count: 1,
      same: true,
      closed: true,
    })
    expect(runtime.sent('events.wait')).toHaveLength(2)
  })

  it('keeps a newer live re-admission while two older close waits resume', async () => {
    const entered = deferred<void>()
    const reading = deferred<void>()
    const liveReply = deferred<unknown>()
    const replies: Array<(value: unknown) => void> = []
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next') return never
        if (name === 'page.title') {
          reading.resolve()
          return liveReply.promise
        }
        if (name !== 'events.wait') return null
        return new Promise((resolve) => {
          replies.push(resolve)
          if (replies.length === 2) entered.resolve()
        })
      },
    })
    const pending = runtime.run(`
      let count = 0
      const seen = []
      page.on('close', () => count++)
      await Promise.all([1, 2].map(() => page.waitForEvent('close', p => { seen.push(p.isClosed()); return true })))
      return { seen, count, closed: page.isClosed() }
    `)
    await entered.promise
    const read = runtime.run('await page.title()')
    await reading.promise
    for (const [index, resolve] of replies.entries())
      resolve({
        ok: closeRecord(7),
        pages: [closed],
        observed: { seq: index + 10, events: 7, closes: [{ tab: 51, seq: 7 }] },
      })
    liveReply.resolve({
      ok: 'Current',
      pages: [live],
      observed: { seq: 20, events: 7 },
    })
    await read
    expect(json(await pending)).toEqual({
      seen: [false, false],
      count: 0,
      closed: false,
    })
  })

  it('delivers allowed records without closing a page whose close was withheld', async () => {
    const other = { $neo: 'page', tab: 52 }
    let pulls = 0
    const runtime = loadPrelude({
      seed: { ...seed, pages: [live, { tab: 52, closed: false }] },
      answer: ({ name }) => {
        if (name !== 'events.next') return null
        if (++pulls !== 1) return never
        return {
          ok: {
            events: [
              { ...consoleRecord(5), page: other },
              { ...consoleRecord(7), page: other },
            ],
            lost: 0,
            withheld: 1,
          },
          pages: [],
          observed: null,
        }
      },
    })
    expect(
      json(
        await runtime.run(`
      let closes = 0
      const seen = []
      page.on('close', () => closes++)
      await new Promise(resolve => context.pages().find(p => p._tab === 52).on('console', () => {
        seen.push(page.isClosed())
        if (seen.length === 2) resolve()
      }))
      return { seen, closes, closed: page.isClosed() }
    `),
      ),
    ).toEqual({ seen: [false, false], closes: 0, closed: false })
  })

  for (const pumpLast of [false, true]) {
    it(`lets a staged pump close at its own position before duplicate wait predicates, pumpLast=${pumpLast}`, async () => {
      const entered = deferred<void>()
      const waits: Array<(value: unknown) => void> = []
      const pump = deferred<unknown>()
      let pumping = false
      const runtime = loadPrelude({
        seed,
        answer: ({ name }) => {
          if (name !== 'events.next' && name !== 'events.wait') return null
          if (name === 'events.next') {
            if (pumping) return never
            pumping = true
            if (waits.length === 2) entered.resolve()
            return pump.promise
          }
          return new Promise((resolve) => {
            waits.push(resolve)
            if (pumping && waits.length === 2) entered.resolve()
          })
        },
      })
      const pending = runtime.run(`
        const seen = []
        const log = () => seen.push(['console', page.isClosed()])
        page.on('console', log)
        page.on('close', () => { seen.push(['close', page.isClosed()]); page.off('console', log) })
        await Promise.all([1, 2].map(() => page.waitForEvent('close', p => {
          seen.push(['predicate', p.isClosed()])
          return p.isClosed()
        })))
        return seen
      `)
      await entered.promise
      const reply = (seq: number, ok: unknown) => ({
        ok,
        pages: [closed],
        observed: { seq, events: 7, closes: [{ tab: 51, seq: 7 }] },
      })
      if (!pumpLast)
        pump.resolve(
          reply(10, { events: [consoleRecord(6), closeRecord(7)], lost: 0 }),
        )
      waits.forEach((resolve, index) => {
        resolve(reply(11 + index, closeRecord(7)))
      })
      if (pumpLast)
        pump.resolve(
          reply(13, { events: [consoleRecord(6), closeRecord(7)], lost: 0 }),
        )
      expect(json(await pending)).toEqual([
        ['console', false],
        ['close', true],
        ['predicate', true],
        ['predicate', true],
      ])
      expect(runtime.sent('events.wait')).toHaveLength(2)
    })
  }

  it('applies an evicted close at its wait resolution, once', async () => {
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next') return never
        if (name === 'events.wait')
          return {
            ok: closeRecord(7),
            pages: [closed],
            observed: { seq: 20, events: 2000, closes: [{ tab: 51, seq: 7 }] },
          }
        return null
      },
    })
    expect(
      json(
        await runtime.run(`
      let count = 0
      page.on('close', () => count++)
      const result = await page.waitForEvent('close')
      return { same: result === page, closed: page.isClosed(), count }
    `),
      ),
    ).toEqual({ same: true, closed: true, count: 1 })
  })
})
