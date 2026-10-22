import { expect, test } from 'bun:test'
import { createResources } from './lifecycle'

test('tests that interrupt during startup waits for and releases the new resource', async () => {
  const resources = createResources()
  const released: number[] = []
  let resolve!: (value: number) => void
  const starting = new Promise<number>((done) => {
    resolve = done
  })
  const acquiring = resources.own(
    () => starting,
    async (value) => {
      released.push(value)
    },
  )
  const stopped = resources.close()
  resolve(7)
  await expect(acquiring).rejects.toThrow('interrupted')
  await stopped
  await resources.close()
  expect(released).toEqual([7])
  await expect(
    resources.own(
      async () => 8,
      async () => {},
    ),
  ).rejects.toThrow('interrupted')
})

test('tests that teardown is reversed and continues after a cleanup failure', async () => {
  const resources = createResources()
  const released: number[] = []
  await resources.own(
    async () => 1,
    async (value) => {
      released.push(value)
    },
  )
  await resources.own(
    async () => 2,
    async (value) => {
      released.push(value)
      throw new Error('cleanup failure')
    },
  )
  await expect(resources.close()).rejects.toThrow('cleanup failed')
  expect(released).toEqual([2, 1])
})
