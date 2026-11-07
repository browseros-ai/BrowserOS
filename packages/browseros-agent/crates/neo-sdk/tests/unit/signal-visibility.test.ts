import { expect, test } from 'bun:test'
import { start } from './fake-runtime'

test('S1 requires signal refusal for visibility APIs whose pinned client accepts TimeoutOptions', async () => {
  const { page, fake } = start()
  fake.answer = (message) =>
    ['isVisible', 'isHidden'].includes(message.method)
      ? { result: { value: message.method === 'isVisible' } }
      : undefined
  const observed = []
  for (const [family, target] of [
    ['locator', page.locator('#target')],
    ['page', page],
    ['frame', page.mainFrame()],
  ] as const) {
    for (const method of ['isVisible', 'isHidden'] as const) {
      const before = fake.messages.length
      const options = { signal: { aborted: true } } as never
      const args = family === 'locator' ? [options] : ['#target', options]
      let value: unknown, error: unknown
      try {
        value = await Reflect.apply(target[method], target, args)
      } catch (caught) {
        error = caught
      }
      observed.push({
        name: `${family}.${method}`,
        value,
        error: error instanceof Error ? error.name : null,
        refused:
          error instanceof Error &&
          error.name === 'Error' &&
          /\bsignal\b/.test(error.message),
        sent: fake.messages.length - before,
      })
    }
  }
  console.log(JSON.stringify(observed))
  expect(observed.filter((row) => !row.refused || row.sent !== 0)).toEqual([])
})
