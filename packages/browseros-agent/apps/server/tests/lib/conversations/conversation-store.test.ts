import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { UIMessage } from 'ai'
import { DbConversationStore } from '../../../src/lib/conversations/conversation-store'
import { closeDb, initializeDb } from '../../../src/lib/db'

describe('DbConversationStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  test('round-trips the full message blob through get', async () => {
    const store = createStore()
    const messages = [userMessage('u1', 'hello'), assistantMessage('a1', 'hi')]
    await store.save({ id: CONVERSATION_ID, messages, targetType: 'browseros' })

    const detail = await store.get(CONVERSATION_ID)
    expect(detail?.messages).toEqual(messages)
    expect(detail?.targetType).toBe('browseros')
    expect(await store.get('00000000-0000-4000-8000-0000000000ff')).toBeNull()
  })

  test('list returns summaries without the message blob, newest first', async () => {
    const store = createStore()
    await store.save({
      id: idFor(1),
      messages: [userMessage('u1', 'first')],
      targetType: 'browseros',
    })
    await sleep(5)
    await store.save({
      id: idFor(2),
      messages: [userMessage('u2', 'second')],
      targetType: 'browseros',
    })

    const summaries = await store.list()
    expect(summaries.map((s) => s.id)).toEqual([idFor(2), idFor(1)])
    expect('messages' in summaries[0]).toBe(false)
    expect(summaries[0].lastUserMessage).toBe('second')
  })

  test('save upserts messages while preserving createdAt', async () => {
    const store = createStore()
    const first = await store.save({
      id: CONVERSATION_ID,
      messages: [userMessage('u1', 'one')],
      targetType: 'browseros',
    })
    await sleep(5)
    const second = await store.save({
      id: CONVERSATION_ID,
      messages: [userMessage('u1', 'one'), assistantMessage('a1', 'two')],
      targetType: 'browseros',
    })

    expect(second.createdAt).toBe(first.createdAt)
    expect(second.updatedAt).toBeGreaterThan(first.updatedAt)
    expect(second.lastMessagedAt).toBeGreaterThan(first.lastMessagedAt)
    expect((await store.get(CONVERSATION_ID))?.messages).toHaveLength(2)
  })

  test('lastUserMessage reflects the most recent user turn', async () => {
    const store = createStore()
    await store.save({
      id: CONVERSATION_ID,
      messages: [
        userMessage('u1', 'q1'),
        assistantMessage('a1', 'ans1'),
        userMessage('u2', 'q2'),
        assistantMessage('a2', 'ans2'),
      ],
      targetType: 'browseros',
    })

    const summaries = await store.list()
    expect(summaries[0].lastUserMessage).toBe('q2')
  })

  test('save persists agent metadata for acp display copies', async () => {
    const store = createStore()
    await store.save({
      id: CONVERSATION_ID,
      messages: [userMessage('u1', 'run this'), assistantMessage('a1', 'ok')],
      targetType: 'claude',
      agentId: 'agent-1',
    })
    // A later ACP turn overwrites with the full in-memory thread.
    await store.save({
      id: CONVERSATION_ID,
      messages: [
        userMessage('u1', 'run this'),
        assistantMessage('a1', 'ok'),
        userMessage('u2', 'next'),
      ],
      targetType: 'claude',
      agentId: 'agent-1',
    })

    const detail = await store.get(CONVERSATION_ID)
    expect(detail?.messages.map((m) => m.id)).toEqual(['u1', 'a1', 'u2'])
    expect(detail?.targetType).toBe('claude')
    expect(detail?.agentId).toBe('agent-1')
  })

  test('save preserves an explicit lastMessagedAt for imports', async () => {
    const store = createStore()
    await store.save({
      id: CONVERSATION_ID,
      messages: [userMessage('u1', 'legacy')],
      targetType: 'browseros',
      lastMessagedAt: 12345,
    })

    const summaries = await store.list()
    expect(summaries[0]?.lastMessagedAt).toBe(12345)
  })

  test('insertIfAbsent inserts when absent and skips when present', async () => {
    const store = createStore()

    expect(
      await store.insertIfAbsent({
        id: CONVERSATION_ID,
        messages: [userMessage('u1', 'hi')],
        targetType: 'browseros',
      }),
    ).not.toBeNull()
    expect(
      await store.insertIfAbsent({
        id: CONVERSATION_ID,
        messages: [userMessage('u2', 'again')],
        targetType: 'browseros',
      }),
    ).toBeNull()

    const detail = await store.get(CONVERSATION_ID)
    expect(detail?.messages.map((message) => message.id)).toEqual(['u1'])
  })

  test('delete removes the record', async () => {
    const store = createStore()
    await store.save({
      id: CONVERSATION_ID,
      messages: [userMessage('u1', 'hello')],
      targetType: 'browseros',
    })

    expect(await store.delete(CONVERSATION_ID)).toBe(true)
    expect(await store.delete(CONVERSATION_ID)).toBe(false)
    expect(await store.get(CONVERSATION_ID)).toBeNull()
  })

  test('history survives closing and reopening the database', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-conversations-durable-'))
    tempDirs.push(dir)
    const dbPath = join(dir, 'db', 'browseros.sqlite')
    const messages = [userMessage('u1', 'persist me')]

    const first = initializeDb({ dbPath })
    await new DbConversationStore({ db: first.db }).save({
      id: CONVERSATION_ID,
      messages,
      targetType: 'browseros',
    })
    closeDb()

    const reopened = initializeDb({ dbPath })
    const detail = await new DbConversationStore({ db: reopened.db }).get(
      CONVERSATION_ID,
    )
    expect(detail?.messages).toEqual(messages)
  })

  function createStore(): DbConversationStore {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-conversations-test-'))
    tempDirs.push(dir)
    const handle = initializeDb({ dbPath: join(dir, 'db', 'browseros.sqlite') })
    return new DbConversationStore({ db: handle.db })
  }
})

const CONVERSATION_ID = '00000000-0000-4000-8000-000000000001'

function idFor(n: number): string {
  return `00000000-0000-4000-8000-00000000000${n}`
}

function userMessage(id: string, text: string): UIMessage {
  return { id, role: 'user', parts: [{ type: 'text', text }] }
}

function assistantMessage(id: string, text: string): UIMessage {
  return { id, role: 'assistant', parts: [{ type: 'text', text }] }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
