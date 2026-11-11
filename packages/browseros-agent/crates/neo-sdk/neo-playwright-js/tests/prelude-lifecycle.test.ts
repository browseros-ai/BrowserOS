// Host-observed Page lifecycle through reply envelopes, delayed events and stable tab objects.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude } from './support/prelude-runtime'

const live = {
  tab: 51,
  url: 'https://example.test/',
  title: 'Example',
  closed: false,
}
const closed = { tab: 51, closed: true }
const brand = { $neo: 'page', tab: 51 }
const seed = { page: brand, pages: [live, { tab: 52, closed: false }] }
const observed = (seq: number, events = 0) => ({ seq, events })
const never = new Promise<never>(() => {})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

function closeRecord(seq: number) {
  return { seq, at: 1000, event: 'close', page: brand }
}

describe('host-observed tab lifecycle', () => {
  it('re-admits the same Page for pages(), isClosed() and opener()', async () => {
    let reads = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'page.title')
          return { ok: 'Title', pages: [++reads === 1 ? closed : live] }
        if (name === 'page.opener')
          return { ok: { opener: brand }, pages: [live] }
        return null
      },
    })
    expect(
      json(
        await runtime.run(`
      const popup = context.pages().find(p => p._tab === 52)
      await page.title()
      const excluded = { closed: page.isClosed(), listed: context.pages().includes(page) }
      await page.title()
      return { excluded, closed: page.isClosed(), same: context.pages().find(p => p._tab === 51) === page,
        opener: (await popup.opener()) === page }
    `),
      ),
    ).toEqual({
      excluded: { closed: true, listed: false },
      closed: false,
      same: true,
      opener: true,
    })
  })

  for (const laterClose of [true, false]) {
    it(
      laterClose
        ? 'applies a later real close read from an earlier waiting request'
        : 'ignores an earlier close read delivered after a newer live read',
      async () => {
        const entered = deferred<void>()
        const delayed = deferred<unknown>()
        const runtime = loadPrelude({
          seed,
          answer: ({ name }) => {
            if (name === 'events.wait') {
              entered.resolve()
              return delayed.promise
            }
            if (name === 'page.title')
              return {
                ok: 'Title',
                pages: [live],
                observed: observed(laterClose ? 1 : 2),
              }
            return null
          },
        })
        const pending = runtime.run(
          "try { await page.waitForEvent('popup') } catch (e) { return e.code }",
        )
        await entered.promise
        await runtime.run('await page.title()')
        delayed.resolve({
          err: {
            code: 'target_closed',
            message: 'Closed',
            owner: 'neo',
            delivery: 'not_sent',
            retryable: true,
          },
          pages: [closed],
          observed: observed(laterClose ? 2 : 1),
        })
        await pending
        expect(
          json(
            await runtime.run(
              'return { closed: page.isClosed(), listed: context.pages().includes(page) }',
            ),
          ),
        ).toEqual({ closed: laterClose, listed: !laterClose })
      },
    )
  }

  for (const cutoff of [6, 7]) {
    it(
      cutoff === 7
        ? 'ignores the first delayed delivery of a close predating re-admission'
        : 'applies a genuine close recorded after re-admission',
      async () => {
        const entered = deferred<void>()
        const delayed = deferred<unknown>()
        let reads = 0
        const runtime = loadPrelude({
          seed,
          answer: ({ name }) => {
            if (name === 'events.wait') {
              entered.resolve()
              return delayed.promise
            }
            if (name === 'page.title')
              return {
                ok: 'Title',
                pages: [++reads === 1 ? closed : live],
                observed: observed(reads, cutoff),
              }
            return null
          },
        })
        const pending = runtime.run("await page.waitForEvent('close')")
        await entered.promise
        await runtime.run('await page.title(); await page.title()')
        delayed.resolve(closeRecord(7))
        await pending
        expect(
          json(
            await runtime.run(
              'return { closed: page.isClosed(), listed: context.pages().includes(page) }',
            ),
          ),
        ).toEqual({ closed: cutoff === 6, listed: cutoff === 7 })
      },
    )
  }

  it('drops repeat closes across re-admission and accepts the next close', async () => {
    const records = [7, 7, 8]
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.next') return never
        if (name === 'events.wait') return closeRecord(records.shift() ?? 8)
        if (name === 'page.title')
          return { ok: 'Title', pages: [live], observed: observed(20, 7) }
        return null
      },
    })
    expect(
      json(
        await runtime.run(`
      let count = 0
      const listen = () => count++
      page.on('close', listen)
      await page.waitForEvent('close')
      await page.title()
      await page.waitForEvent('close')
      const afterRepeat = { closed: page.isClosed(), count }
      await page.waitForEvent('close')
      page.off('close', listen)
      return { afterRepeat, closed: page.isClosed(), count }
    `),
      ),
    ).toEqual({
      afterRepeat: { closed: false, count: 1 },
      closed: true,
      count: 2,
    })
  })

  it('rejects a live read predating an applied close and accepts a later one', async () => {
    let reads = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.wait') return closeRecord(9)
        if (name === 'page.title')
          return {
            ok: 'Title',
            pages: [live],
            observed: observed(++reads, reads === 1 ? 7 : 9),
          }
        return null
      },
    })
    expect(
      json(
        await runtime.run(`
      await page.waitForEvent('close')
      await page.title()
      const stale = page.isClosed()
      await page.title()
      return { stale, current: page.isClosed() }
    `),
      ),
    ).toEqual({ stale: true, current: false })
  })

  it('orders URL, title and closed state by observation rather than reply arrival', async () => {
    const entered = deferred<void>()
    const delayed = deferred<unknown>()
    let reads = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name !== 'page.title') return null
        if (++reads === 1) {
          entered.resolve()
          return delayed.promise
        }
        return {
          ok: 'New',
          pages: [{ ...live, url: 'https://new.test/', title: 'New' }],
          observed: observed(2),
        }
      },
    })
    const pending = runtime.run('await page.title()')
    await entered.promise
    await runtime.run('await page.title()')
    delayed.resolve({
      ok: 'Old',
      pages: [{ ...closed, url: 'https://old.test/', title: 'Old' }],
      observed: observed(1),
    })
    await pending
    expect(
      json(
        await runtime.run(
          'return { url: page.url(), title: page._title, closed: page.isClosed() }',
        ),
      ),
    ).toEqual({ url: 'https://new.test/', title: 'New', closed: false })
  })

  for (const beforeUnload of [false, true]) {
    it(
      beforeUnload
        ? 'keeps a vetoable close live when its fact is live'
        : 'marks close from its reply fact without another operation',
      async () => {
        const runtime = loadPrelude({
          seed,
          answer: ({ name }) =>
            name === 'page.close'
              ? {
                  ok: null,
                  pages: [beforeUnload ? live : closed],
                  observed: observed(1),
                }
              : null,
        })
        expect(
          await runtime.run(
            `await page.close({ runBeforeUnload: ${beforeUnload} }); return page.isClosed()`,
          ),
        ).toBe(!beforeUnload)
        expect(runtime.sent('page.close')).toHaveLength(1)
      },
    )
  }

  for (const linkedFact of [false, true]) {
    it(
      linkedFact
        ? 're-admits an opener through its linked fact'
        : 'treats an opener result without a fact as identity only',
      async () => {
        const runtime = loadPrelude({
          seed,
          answer: ({ name }) => {
            if (name === 'page.title') return { ok: 'Closed', pages: [closed] }
            if (name === 'page.opener')
              return { ok: { opener: brand }, pages: linkedFact ? [live] : [] }
            return null
          },
        })
        expect(
          json(
            await runtime.run(`
        const popup = context.pages().find(p => p._tab === 52)
        await page.title()
        const opener = await popup.opener()
        return { same: opener === page, closed: page.isClosed(), listed: context.pages().includes(page) }
      `),
          ),
        ).toEqual({ same: linkedFact, closed: !linkedFact, listed: linkedFact })
      },
    )
  }

  it('uses the init observation cutoff and keeps a closed init tab closed', async () => {
    const records = [7, 8]
    const runtime = loadPrelude({
      seed: { ...seed, observed: observed(10, 7) },
      answer: ({ name }) =>
        name === 'events.wait' ? closeRecord(records.shift() ?? 8) : null,
    })
    expect(
      json(
        await runtime.run(
          "await page.waitForEvent('close'); const first = page.isClosed(); await page.waitForEvent('close'); return { first, next: page.isClosed() }",
        ),
      ),
    ).toEqual({ first: false, next: true })
    const initiallyClosed = loadPrelude({
      seed: { page: brand, pages: [closed], observed: observed(10, 7) },
    })
    expect(await initiallyClosed.run('return page.isClosed()')).toBe(true)
  })

  for (const head of [9, 1025]) {
    it(`closes a delivered event with cached lag and event head ${head}, with one emission`, async () => {
      const runtime = loadPrelude({
        seed,
        answer: ({ name }) => {
          if (name === 'events.next') return never
          if (name === 'events.wait')
            return {
              ok: closeRecord(9),
              pages: [closed],
              observed: observed(2, head),
            }
          return null
        },
      })
      expect(
        json(
          await runtime.run(`
      let count = 0
      const listen = () => count++
      page.on('close', listen)
      await page.waitForEvent('close')
      page.off('close', listen)
      return { closed: page.isClosed(), count }
    `),
        ),
      ).toEqual({ closed: true, count: 1 })
    })
  }

  it('closes from the event pump without a subsequent operation', async () => {
    let pumps = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) =>
        name === 'events.next'
          ? ++pumps === 1
            ? { events: [closeRecord(9)], lost: 0 }
            : never
          : null,
    })
    expect(
      await runtime.run(
        "await new Promise(resolve => page.on('close', resolve)); return page.isClosed()",
      ),
    ).toBe(true)
  })

  it('preserves fresh URLs against old events and initializes newly discovered event pages', async () => {
    const entered = deferred<void>()
    const delayed = deferred<unknown>()
    let reads = 0
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) => {
        if (name === 'events.wait') {
          entered.resolve()
          return delayed.promise
        }
        if (name === 'page.title')
          return {
            ok: 'Title',
            pages: [
              ++reads === 1 ? closed : { ...live, url: 'https://new.test/' },
            ],
          }
        return null
      },
    })
    const pending = runtime.run("await context.waitForEvent('page')")
    await entered.promise
    await runtime.run('await page.title(); await page.title()')
    delayed.resolve({
      seq: 7,
      at: 1000,
      event: 'page',
      page: brand,
      url: 'https://old.test/',
    })
    await pending
    expect(await runtime.run('return page.url()')).toBe('https://new.test/')
    const discovered = loadPrelude({
      answer: ({ name }) =>
        name === 'events.wait'
          ? {
              seq: 8,
              event: 'page',
              page: { $neo: 'page', tab: 99 },
              url: 'https://discovered.test/',
            }
          : null,
    })
    expect(
      await discovered.run("return (await context.waitForEvent('page')).url()"),
    ).toBe('https://discovered.test/')
  })

  it('requires a valid observation for lifecycle facts', async () => {
    const runtime = loadPrelude({
      seed,
      answer: ({ name }) =>
        name === 'page.title'
          ? { ok: 'Title', pages: [closed], observed: null }
          : null,
    })
    expect(
      await runtime.run('await page.title(); return page.isClosed()'),
    ).toBe(false)
  })
})
