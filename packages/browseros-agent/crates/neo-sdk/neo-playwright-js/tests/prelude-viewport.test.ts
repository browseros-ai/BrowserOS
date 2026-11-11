// Viewport configuration belongs to a tab; each fresh agent realm initializes its cache
// from host facts, then owns the same synchronous client cache as Playwright's Page.
import { expect, it } from 'bun:test'
import { isRefusal, json, loadPrelude, SEED } from './support/prelude-runtime'

it('keeps each tab independent and caches the caller object before the write answers', async () => {
  const runtime = loadPrelude({
    seed: { ...SEED, pages: [{ tab: 1007 }, { tab: 1008 }] },
  })
  expect(
    json(
      await runtime.run(`
    const initial = page.viewportSize()
    const other = context.pages().find(p => p !== page)
    const size = { width: 50, height: 5000 }
    const pending = page.setViewportSize(size)
    const immediate = page.viewportSize() === size
    await pending
    size.width = 51
    return { initial, immediate, width: page.viewportSize().width, other: other.viewportSize() }
  `),
    ),
  ).toEqual({ initial: null, immediate: true, width: 51, other: null })
  expect(runtime.sent('page.setViewportSize')[0]?.input).toMatchObject({
    call: { api: 'page.setViewportSize', timeoutMs: 0 },
    viewportSize: { width: 50, height: 5000 },
  })
})

it('seeds a later run from tab facts and preserves its local cache after a refused write', async () => {
  const size = { width: 640, height: 480 }
  const runtime = loadPrelude({
    seed: {
      ...SEED,
      pages: [{ tab: 1007, viewportSize: size }],
    },
    answer: ({ name }) =>
      name === 'page.setViewportSize'
        ? {
            err: {
              code: 'tab_locked',
              owner: 'host',
              message: 'owner holds tab',
              delivery: 'not_sent',
            },
            pages: [{ tab: 1007, closed: false, viewportSize: size }],
            observed: { seq: 10, events: 0 },
          }
        : null,
  })
  expect(
    json(
      await runtime.run(`
    const previous = page.viewportSize()
    try { await page.setViewportSize({ width: 800, height: 600 }) } catch {}
    return { previous, requested: page.viewportSize() }
  `),
    ),
  ).toEqual({ previous: size, requested: { width: 800, height: 600 } })
})

it('rejects non-integer dimensions before any browser write', async () => {
  const runtime = loadPrelude({ seed: SEED })
  expect(
    await runtime.run(`
    try { await page.setViewportSize({ width: 1.5, height: 480 }) }
    catch (error) { return error.message }
  `),
  ).toContain('viewportSize.width')
  expect(
    runtime.sent('page.setViewportSize').filter((call) => !isRefusal(call)),
  ).toEqual([])
})

it('normalizes a failed undefined viewport argument to null like upstream', async () => {
  const runtime = loadPrelude({ seed: SEED })
  expect(
    await runtime.run(`
    try { await page.setViewportSize(undefined) } catch {}
    return page.viewportSize()
  `),
  ).toBeNull()
})

it('preserves a restored viewport object across unrelated host observations', async () => {
  const size = { width: 640, height: 480 }
  const runtime = loadPrelude({
    seed: { ...SEED, pages: [{ tab: 1007, viewportSize: size }] },
    answer: ({ name }) =>
      name === 'page.title'
        ? {
            ok: 'title',
            pages: [{ tab: 1007, closed: false, viewportSize: size }],
            observed: { seq: 10, events: 0 },
          }
        : null,
  })
  expect(
    await runtime.run(`
    const size = page.viewportSize()
    await page.title()
    return page.viewportSize() === size
  `),
  ).toBe(true)
})
