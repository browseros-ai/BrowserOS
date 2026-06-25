/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { AsyncMutex } from '../../src/lib/async-mutex'

describe('AsyncMutex', () => {
  test('serialises tasks submitted concurrently', async () => {
    const mutex = new AsyncMutex()
    const log: string[] = []
    const make = (name: string, delayMs: number) => () =>
      new Promise<string>((resolve) => {
        log.push(`start:${name}`)
        setTimeout(() => {
          log.push(`done:${name}`)
          resolve(name)
        }, delayMs)
      })

    const results = await Promise.all([
      mutex.run(make('a', 30)),
      mutex.run(make('b', 5)),
      mutex.run(make('c', 5)),
    ])

    expect(results).toEqual(['a', 'b', 'c'])
    // Each task fully completes before the next one starts.
    expect(log).toEqual([
      'start:a',
      'done:a',
      'start:b',
      'done:b',
      'start:c',
      'done:c',
    ])
  })

  test('a rejected task does not block subsequent ones', async () => {
    const mutex = new AsyncMutex()
    const ran: string[] = []
    const rejected = mutex.run(async () => {
      ran.push('reject')
      throw new Error('boom')
    })
    const ok = mutex.run(async () => {
      ran.push('after')
      return 'ok'
    })

    expect(rejected).rejects.toThrow('boom')
    expect(await ok).toBe('ok')
    expect(ran).toEqual(['reject', 'after'])
  })
})
