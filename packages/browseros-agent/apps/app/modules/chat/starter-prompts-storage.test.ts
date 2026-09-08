import { beforeEach, describe, expect, it, mock } from 'bun:test'

type Listener = (value: unknown) => void
type FakeItem = {
  value: unknown
  pendingRead?: Promise<unknown>
  failWrite: boolean
  listeners: Set<Listener>
  writes: unknown[]
  emit: (value: unknown) => void
}

const items = new Map<string, FakeItem>()

// Simulate extension storage events crossing independently mounted app surfaces.
mock.module('@wxt-dev/storage', () => ({
  storage: {
    defineItem: (key: string, options: { fallback: unknown }) => {
      const item: FakeItem = {
        value: options.fallback,
        failWrite: false,
        listeners: new Set(),
        writes: [],
        emit(value) {
          item.value = value
          for (const listener of item.listeners) listener(value)
        },
      }
      items.set(key, item)
      return {
        getValue: () => item.pendingRead ?? Promise.resolve(item.value),
        setValue: async (value: unknown) => {
          if (item.failWrite) throw new Error('Storage unavailable')
          item.writes.push(value)
          item.emit(value)
        },
        watch: (listener: Listener) => {
          item.listeners.add(listener)
          return () => item.listeners.delete(listener)
        },
      }
    },
  },
}))

const { observeStarterPrompts, saveStarterPrompts } = await import(
  './starter-prompts-storage'
)
const { getDefaultStarterPrompts } = await import('./starter-prompts')

const agentItem = () => items.get('local:agentStarterPrompts') as FakeItem
const chatItem = () => items.get('local:chatStarterPrompts') as FakeItem
const settle = async () => {
  await Promise.resolve()
  await Promise.resolve()
}

beforeEach(() => {
  for (const item of items.values()) {
    item.listeners.clear()
    item.writes.length = 0
    item.pendingRead = undefined
    item.failWrite = false
    item.value = getDefaultStarterPrompts('agent')
  }
})

describe('starter prompt storage', () => {
  it('updates both surfaces while keeping the other mode untouched', async () => {
    const first: unknown[] = []
    const second: unknown[] = []
    const stopFirst = observeStarterPrompts(
      'agent',
      (v) => first.push(v),
      () => {},
    )
    const stopSecond = observeStarterPrompts(
      'agent',
      (v) => second.push(v),
      () => {},
    )
    await settle()
    const saved = getDefaultStarterPrompts('agent')
    saved[0] = { display: 'My shortcut', prompt: 'Read my current tab' }
    await saveStarterPrompts('agent', saved)
    expect(first.at(-1)).toEqual(saved)
    expect(second.at(-1)).toEqual(saved)
    expect(chatItem().writes).toEqual([])
    stopFirst()
    stopSecond()
  })

  it('does not let an older initial read overwrite a newer storage event', async () => {
    let finishRead!: (value: unknown) => void
    agentItem().pendingRead = new Promise((resolve) => {
      finishRead = resolve
    })
    const values: unknown[] = []
    const stop = observeStarterPrompts(
      'agent',
      (v) => values.push(v),
      () => {},
    )
    const newer = getDefaultStarterPrompts('agent')
    newer[0].display = 'Saved in another tab'
    agentItem().emit(newer)
    finishRead(getDefaultStarterPrompts('agent'))
    await settle()
    expect(values).toEqual([newer])
    stop()
  })

  it('unsubscribes and ignores a pending read after the surface closes', async () => {
    let finishRead!: (value: unknown) => void
    agentItem().pendingRead = new Promise((resolve) => {
      finishRead = resolve
    })
    const changed = mock(() => {})
    const stop = observeStarterPrompts('agent', changed, () => {})
    stop()
    finishRead(getDefaultStarterPrompts('agent'))
    agentItem().emit(getDefaultStarterPrompts('agent'))
    await settle()
    expect(changed).not.toHaveBeenCalled()
    expect(agentItem().listeners.size).toBe(0)
  })

  it('reports read failures so the editor cannot overwrite unread preferences', async () => {
    agentItem().pendingRead = Promise.reject(new Error('Storage unavailable'))
    const failed = mock(() => {})
    const stop = observeStarterPrompts('agent', () => {}, failed)
    await settle()
    expect(failed).toHaveBeenCalledTimes(1)
    stop()
  })

  it('rejects failed and invalid writes without replacing saved preferences', async () => {
    const original = agentItem().value
    const invalid = getDefaultStarterPrompts('agent')
    invalid[0].prompt = ' '
    await expect(saveStarterPrompts('agent', invalid)).rejects.toThrow()
    agentItem().failWrite = true
    await expect(
      saveStarterPrompts('agent', getDefaultStarterPrompts('agent')),
    ).rejects.toThrow('Storage unavailable')
    expect(agentItem().value).toBe(original)
    expect(agentItem().writes).toEqual([])
  })
})
