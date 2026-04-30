/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  FileMessageQueue,
  MessageQueueFullError,
} from '../../../src/lib/agents/message-queue'

let tmp: string
let queue: FileMessageQueue

beforeEach(async () => {
  tmp = await mkdtemp(join(tmpdir(), 'browseros-queue-'))
  queue = new FileMessageQueue({
    filePath: join(tmp, 'queues.json'),
    maxLength: 3,
  })
})

afterEach(async () => {
  await rm(tmp, { recursive: true, force: true })
})

describe('FileMessageQueue', () => {
  it('appends in FIFO order and pops oldest first', async () => {
    await queue.append('a', { message: 'one' })
    await queue.append('a', { message: 'two' })
    const popped = await queue.popOldest('a')
    expect(popped?.message).toBe('one')
    expect(await queue.list('a')).toEqual([
      expect.objectContaining({ message: 'two' }),
    ])
  })

  it('returns null when popping an empty queue', async () => {
    expect(await queue.popOldest('a')).toBeNull()
  })

  it('removes a single message by id', async () => {
    const first = await queue.append('a', { message: 'one' })
    await queue.append('a', { message: 'two' })
    const removed = await queue.remove('a', first.id)
    expect(removed).toBe(true)
    expect(await queue.list('a')).toEqual([
      expect.objectContaining({ message: 'two' }),
    ])
  })

  it('returns false when removing an unknown message id', async () => {
    await queue.append('a', { message: 'one' })
    expect(await queue.remove('a', 'nope')).toBe(false)
  })

  it('throws MessageQueueFullError when capacity is reached', async () => {
    await queue.append('a', { message: 'one' })
    await queue.append('a', { message: 'two' })
    await queue.append('a', { message: 'three' })
    await expect(queue.append('a', { message: 'four' })).rejects.toBeInstanceOf(
      MessageQueueFullError,
    )
  })

  it('pushFront bypasses the cap (recovery primitive)', async () => {
    await queue.append('a', { message: 'one' })
    await queue.append('a', { message: 'two' })
    await queue.append('a', { message: 'three' })
    await queue.pushFront('a', {
      id: 'recovered',
      createdAt: Date.now(),
      message: 'recovered',
    })
    expect((await queue.list('a')).map((q) => q.message)).toEqual([
      'recovered',
      'one',
      'two',
      'three',
    ])
  })

  it('persists across instances on the same file path', async () => {
    await queue.append('a', { message: 'survives' })
    const other = new FileMessageQueue({
      filePath: join(tmp, 'queues.json'),
      maxLength: 3,
    })
    expect((await other.list('a')).map((q) => q.message)).toEqual(['survives'])
  })

  it('agentsWithPendingMessages lists agents with non-empty queues', async () => {
    await queue.append('a', { message: 'x' })
    await queue.append('b', { message: 'y' })
    const pending = await queue.agentsWithPendingMessages()
    expect(pending.sort()).toEqual(['a', 'b'])
  })

  it('writes are atomic (temp file rename leaves no stray files)', async () => {
    await queue.append('a', { message: 'one' })
    const raw = await readFile(join(tmp, 'queues.json'), 'utf8')
    const parsed = JSON.parse(raw)
    expect(parsed.version).toBe(1)
    expect(parsed.queues.a[0].message).toBe('one')
  })
})
