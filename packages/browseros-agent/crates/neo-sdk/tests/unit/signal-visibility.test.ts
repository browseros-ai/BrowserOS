/**
 * Playwright sends `isVisible` and `isHidden` with `kNoTimeout` (`client/frame.ts`), so they read
 * no `signal`: an aborted one is ignored, the read runs, and nothing listens to the signal. Neo's
 * reads do the same on every receiver.
 */
import { expect, test } from 'bun:test'
import { start } from './fake-runtime'

test('tests that visibility reads ignore a signal, as Playwright sends them with kNoTimeout', async () => {
  const { page, fake } = start()
  fake.answer = (message) =>
    ['isVisible', 'isHidden'].includes(message.method)
      ? { result: { value: message.method === 'isVisible' } }
      : undefined
  const touched: string[] = []
  const signal = {
    aborted: true,
    reason: new Error('stop'),
    addEventListener() {
      touched.push('addEventListener')
    },
    removeEventListener() {
      touched.push('removeEventListener')
    },
  }
  const observed = []
  for (const [family, target] of [
    ['locator', page.locator('#target')],
    ['page', page],
    ['frame', page.mainFrame()],
  ] as const) {
    for (const method of ['isVisible', 'isHidden'] as const) {
      const before = fake.messages.length
      const options = { signal } as never
      const args = family === 'locator' ? [options] : ['#target', options]
      const value = await Reflect.apply(target[method], target, args)
      observed.push({
        name: `${family}.${method}`,
        value,
        sent: fake.messages.length - before,
      })
    }
  }
  expect(observed).toEqual([
    { name: 'locator.isVisible', value: true, sent: 1 },
    { name: 'locator.isHidden', value: false, sent: 1 },
    { name: 'page.isVisible', value: true, sent: 1 },
    { name: 'page.isHidden', value: false, sent: 1 },
    { name: 'frame.isVisible', value: true, sent: 1 },
    { name: 'frame.isHidden', value: false, sent: 1 },
  ])
  expect(touched).toEqual([])
})
