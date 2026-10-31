import { expect, test } from 'bun:test'
import { waitUntil } from './__helpers__/e2e'

test('bounds unresolved probe', async () => {
  await expect(
    waitUntil('hung', [], () => new Promise(() => {}), 20),
  ).rejects.toThrow('Timed out waiting for hung')
})
test('rejects late readiness', async () => {
  await expect(
    waitUntil(
      'late',
      [],
      async () => {
        await Bun.sleep(80)
        return true
      },
      20,
    ),
  ).rejects.toThrow('Timed out')
})
test('preserves rejection', async () => {
  await expect(
    waitUntil(
      'failed',
      [],
      async () => {
        throw Error('probe failed')
      },
      100,
    ),
  ).rejects.toThrow('probe failed')
})
test('accepts timely readiness', async () => {
  await waitUntil('ready', [], async () => true, 100)
})
test('bounds repeatedly false probes', async () => {
  await expect(waitUntil('false', [], async () => false, 20)).rejects.toThrow(
    'Timed out',
  )
})
test('zero timeout never starts probe', async () => {
  let called = false
  await expect(
    waitUntil(
      'zero',
      [],
      async () => {
        called = true
        return true
      },
      0,
    ),
  ).rejects.toThrow('Timed out')
  expect(called).toBe(false)
})
