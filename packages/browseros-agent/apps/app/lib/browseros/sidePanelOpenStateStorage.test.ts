import { describe, expect, it, mock } from 'bun:test'

const key = 'browseros.side_panel.per_window'
const saved: Record<string, unknown> = { [key]: true }
let writes = 0
mock.module('@wxt-dev/browser', () => ({
  browser: {
    runtime: {},
    storage: {
      local: {
        get: async (keys: string | string[]) =>
          Object.fromEntries(
            (typeof keys === 'string' ? [keys] : keys).map((key) => [
              key,
              saved[key],
            ]),
          ),
        set: async (values: Record<string, unknown>) => {
          writes++
          Object.assign(saved, values)
        },
      },
      onChanged: { addListener: () => {}, removeListener: () => {} },
    },
  },
}))
const { sidePanelPerWindowStorage } = await import(
  './sidePanelOpenStateStorage'
)
const { storage } = await import('@wxt-dev/storage')

describe('retired panel-sharing preference', () => {
  it('migrates existing true to false once, keeping the old key for compatibility', async () => {
    expect(await sidePanelPerWindowStorage.getValue()).toBe(false)
    expect(saved[key]).toBe(false)
    expect(await sidePanelPerWindowStorage.getMeta()).toMatchObject({ v: 2 })
    const writesAfterMigration = writes
    const migration = mock(() => false)
    const restarted = storage.defineItem<boolean>(`local:${key}`, {
      fallback: false,
      version: 2,
      migrations: { 2: migration },
    })
    expect(await restarted.getValue()).toBe(false)
    expect(migration).not.toHaveBeenCalled()
    expect(writes).toBe(writesAfterMigration)
  })
})
