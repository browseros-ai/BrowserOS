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

it('registers onOpened when Chromium does not expose onClosed', async () => {
  let openListenerRegistrations = 0
  globalThis.chrome = {
    sidePanel: {
      onOpened: {
        addListener: () => {
          openListenerRegistrations += 1
        },
      },
    },
  } as unknown as typeof chrome

  const { registerSidePanelOpenStateListeners } = await import(
    './toggleSidePanel'
  )

  expect(() => registerSidePanelOpenStateListeners()).not.toThrow()
  expect(openListenerRegistrations).toBe(1)
})
