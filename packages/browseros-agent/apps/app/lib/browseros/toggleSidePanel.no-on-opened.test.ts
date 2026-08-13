import { expect, it, mock } from 'bun:test'

mock.module('./sidePanelOpenStateStorage', () => ({
  sidePanelPerWindowStorage: {
    getValue: async () => false,
    setValue: async () => {},
  },
  openWindowSidePanelIdsStorage: {
    getValue: async () => [],
    setValue: async () => {},
  },
}))

it('registers onClosed when Chromium does not expose onOpened', async () => {
  let closeListenerRegistrations = 0
  globalThis.chrome = {
    sidePanel: {
      onClosed: {
        addListener: () => {
          closeListenerRegistrations += 1
        },
      },
    },
  } as unknown as typeof chrome

  const { registerSidePanelOpenStateListeners } = await import(
    './toggleSidePanel'
  )

  expect(() => registerSidePanelOpenStateListeners()).not.toThrow()
  expect(closeListenerRegistrations).toBe(1)
})
