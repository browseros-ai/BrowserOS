/**
 * These four acknowledgments are stored as null instead of {} in run records. The script's
 * return values, page facts and locator-handler lifecycle must be identical across the change.
 */
import { expect, it } from 'bun:test'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

const acknowledgments = [{}, null]
const turns = () => new Promise((resolve) => setTimeout(resolve, 10))

it('page.close hides both acknowledgment forms and retains the reply page fact', async () => {
  const results = []
  for (const acknowledgment of acknowledgments) {
    const runtime = loadPrelude({
      seed: SEED,
      answer: ({ name }) =>
        name === 'page.close'
          ? { ok: acknowledgment, pages: [{ tab: 1007, closed: true }] }
          : null,
    })
    results.push(
      json(
        await runtime.run(`
      const result = await page.close()
      return { nothing: result === undefined, closed: page.isClosed() }
    `),
      ),
    )
    expect(runtime.sent('page.close')).toHaveLength(1)
  }
  expect(results).toEqual([
    { nothing: true, closed: true },
    { nothing: true, closed: true },
  ])
})

it('page.setContent hides both acknowledgment forms and performs the same frame refresh', async () => {
  const results = []
  for (const acknowledgment of acknowledgments) {
    const runtime = loadPrelude({
      seed: SEED,
      answer: ({ name }) =>
        name === 'page.setContent' ? acknowledgment : null,
    })
    const nothing = await runtime.run(
      `return await page.setContent('<p>same</p>') === undefined`,
    )
    results.push({ nothing, calls: json(runtime.calls) })
    expect(runtime.sent('page.setContent')).toHaveLength(1)
    expect(nothing).toBe(true)
  }
  expect(results[1]).toEqual(results[0])
})

/** A pull stays pending until the test triggers a registered handler, as the Rust host does. */
function handlerHost(acknowledgment: object | null) {
  const pulls: ((next: { uid: number | null }) => void)[] = []
  return {
    answer: ({ name }: { name: string }) => {
      if (name === 'page.addLocatorHandler') return { uid: 1 }
      if (name === 'page.locatorHandlerTriggered')
        return new Promise((resolve) => pulls.push(resolve))
      return acknowledgment
    },
    trigger(uid: number | null) {
      const pull = pulls.shift()
      if (!pull) throw new Error('no locator-handler pull is pending')
      pull({ uid })
    },
  }
}

it('page.removeLocatorHandler hides both acknowledgment forms and removes the registration', async () => {
  for (const acknowledgment of acknowledgments) {
    const host = handlerHost(acknowledgment)
    const runtime = loadPrelude({ seed: SEED, answer: host.answer })
    expect(
      await runtime.run(`
      const locator = page.locator('#overlay')
      page.handled = 0
      await page.addLocatorHandler(locator, () => { page.handled++ })
      return await page.removeLocatorHandler(locator) === undefined
    `),
    ).toBe(true)
    await turns()
    host.trigger(1)
    await turns()
    expect(await runtime.run('return page.handled')).toBe(0)
    expect(runtime.sent('page.removeLocatorHandler')).toHaveLength(1)
    expect(runtime.sent('page.resolveLocatorHandler')).toHaveLength(1)
  }
})

it('page.resolveLocatorHandler preserves callback completion and the times countdown for both forms', async () => {
  const results = []
  for (const acknowledgment of acknowledgments) {
    const host = handlerHost(acknowledgment)
    const runtime = loadPrelude({ seed: SEED, answer: host.answer })
    await runtime.run(`
      page.handled = 0
      await page.addLocatorHandler(page.locator('#overlay'), () => { page.handled++ }, { times: 1 })
    `)
    await turns()
    host.trigger(1)
    await turns()
    const handled = await runtime.run('return page.handled')
    const resolved = runtime.sent('page.resolveLocatorHandler')
    expect(handled).toBe(1)
    expect(resolved).toHaveLength(1)
    expect(resolved[0]?.input.remove).toBe(true)
    results.push({ handled, calls: json(resolved) })
    host.trigger(null)
    await turns()
  }
  expect(results[1]).toEqual(results[0])
})

it('drops ignored noWaitAfter options while retaining click and press navigation controls', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: ({ name }) =>
      name === 'locator.elementHandle'
        ? {
            $neo: 'element',
            page: { $neo: 'page', tab: 1007 },
            seq: 0,
            node: 5,
            display: '<input>',
          }
        : null,
  })
  await runtime.run(`
    const locator = page.locator('input')
    const options = { noWaitAfter: true }
    await locator.click(options)
    await locator.dblclick(options)
    await locator.hover(options)
    await locator.check(options)
    await locator.uncheck(options)
    await locator.dragTo(page.locator('#target'), options)
    await locator.fill('value', options)
    await locator.press('A', options)
    await locator.pressSequentially('text', options)
    const handle = await page.$('input')
    await handle.press('A', options)
    await handle.type('text', options)
  `)
  for (const op of [
    'locator.dblclick',
    'locator.hover',
    'locator.check',
    'locator.uncheck',
    'locator.dragTo',
    'locator.fill',
    'locator.pressSequentially',
  ]) {
    const calls = runtime.sent(op)
    expect(calls.length).toBeGreaterThan(0)
    for (const call of calls) expect(call.input.options).toEqual({})
  }
  for (const op of ['locator.click', 'locator.press']) {
    for (const call of runtime.sent(op))
      expect(call.input.options).toEqual({ noWaitAfter: true })
  }
})
