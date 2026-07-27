import { describe, expect, it } from 'bun:test'
import { runExclusive } from './conversation-write-queue'

describe('runExclusive', () => {
  it('serializes overlapping mutations instead of interleaving them', async () => {
    const order: string[] = []

    const first = runExclusive(async () => {
      order.push('a-start')
      await Promise.resolve()
      await Promise.resolve()
      order.push('a-end')
    })
    const second = runExclusive(async () => {
      order.push('b-start')
      order.push('b-end')
    })

    await Promise.all([first, second])
    expect(order).toEqual(['a-start', 'a-end', 'b-start', 'b-end'])
  })

  it('keeps draining the queue after a mutation rejects', async () => {
    await expect(
      runExclusive(async () => {
        throw new Error('boom')
      }),
    ).rejects.toThrow('boom')

    await expect(runExclusive(async () => 'ok')).resolves.toBe('ok')
  })
})
