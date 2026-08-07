import { afterEach, describe, expect, it, mock } from 'bun:test'
import { useFocusBrowserTab } from './focus.hooks'

const originalChrome = Object.getOwnPropertyDescriptor(globalThis, 'chrome')

afterEach(() => {
  if (originalChrome) {
    Object.defineProperty(globalThis, 'chrome', originalChrome)
  } else {
    Reflect.deleteProperty(globalThis, 'chrome')
  }
})

describe('useFocusBrowserTab', () => {
  it('activates the selected id and focuses its returned owning window', async () => {
    const tabs = new Map([
      [41, { id: 41, url: 'https://same.example/', windowId: 4 }],
      [42, { id: 42, url: 'https://same.example/', windowId: 9 }],
    ])
    const updateTab = mock(async (browserTabId: number) =>
      tabs.get(browserTabId),
    )
    const updateWindow = mock(async () => undefined)
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: {
        tabs: { update: updateTab },
        windows: { update: updateWindow },
      },
    })

    const result = await useFocusBrowserTab.mutationFn({ browserTabId: 42 })

    expect(updateTab).toHaveBeenCalledWith(42, { active: true })
    expect(updateWindow).toHaveBeenCalledWith(9, { focused: true })
    expect(result).toEqual({ browserTabId: 42, windowId: 9 })
  })

  it('propagates a closed or missing tab error to the caller', async () => {
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: {
        tabs: {
          update: async () => {
            throw new Error('No tab with id: 42')
          },
        },
        windows: { update: async () => undefined },
      },
    })

    expect(useFocusBrowserTab.mutationFn({ browserTabId: 42 })).rejects.toThrow(
      'No tab with id: 42',
    )
  })
})
