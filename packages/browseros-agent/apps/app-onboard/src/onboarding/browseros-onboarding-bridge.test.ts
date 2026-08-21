import { afterEach, describe, expect, it } from 'bun:test'
import type { BrowserOSOnboardingState } from './browseros-onboarding-api'
import { BrowserOSOnboardingMessage } from './browseros-onboarding-api'
import { createBrowserOSOnboardingBridge } from './browseros-onboarding-bridge'
import { MOCK_BROWSEROS_IMPORT_SOURCES } from './onboarding-v2.helpers'

const originalWindow = globalThis.window
const originalChrome = (globalThis as typeof globalThis & { chrome?: unknown })
  .chrome

function installWindow() {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {},
  })
}

function restoreGlobal(name: 'chrome', value: unknown) {
  if (value === undefined) {
    delete (globalThis as typeof globalThis & { chrome?: unknown })[name]
    return
  }
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value,
  })
}

afterEach(() => {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: originalWindow,
  })
  restoreGlobal('chrome', originalChrome)
})

describe('createBrowserOSOnboardingBridge', () => {
  it('sends Chromium messages through the real chrome bridge', () => {
    installWindow()
    const sent: Array<[string, unknown[] | undefined]> = []
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: {
        send(message: string, args?: unknown[]) {
          sent.push([message, args])
        },
      },
    })

    const bridge = createBrowserOSOnboardingBridge()

    bridge.pageReady()
    bridge.refreshSources()
    bridge.startImport({ sourceId: 'source-0', items: ['history'] })
    bridge.complete()

    expect(bridge.isMock).toBe(false)
    expect(sent).toEqual([
      [BrowserOSOnboardingMessage.PAGE_READY, undefined],
      [BrowserOSOnboardingMessage.REFRESH_SOURCES, undefined],
      [
        BrowserOSOnboardingMessage.START_IMPORT,
        [{ sourceId: 'source-0', items: ['history'] }],
      ],
      [BrowserOSOnboardingMessage.COMPLETE, undefined],
    ])
  })

  it('does not send an empty explicit import item request', () => {
    installWindow()
    const sent: Array<[string, unknown[] | undefined]> = []
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: {
        send(message: string, args?: unknown[]) {
          sent.push([message, args])
        },
      },
    })

    createBrowserOSOnboardingBridge().startImport({
      sourceId: 'source-0',
      items: [],
    })

    expect(sent).toEqual([])
  })

  it('installs and cleans up the window state receiver it owns', () => {
    installWindow()
    const states: BrowserOSOnboardingState[] = []
    const bridge = createBrowserOSOnboardingBridge({ chrome: null })
    const cleanup = bridge.registerReceiver((state) => states.push(state))

    expect(window.browserosOnboarding).toBeDefined()
    window.browserosOnboarding?.receiveState({
      apiVersion: 1,
      status: 'ready',
      sources: [],
    })
    cleanup()

    expect(states[0]?.status).toBe('ready')
    expect(window.browserosOnboarding).toBeUndefined()
  })

  it('emits mock ready and import success states without chrome.send', () => {
    installWindow()
    const states: BrowserOSOnboardingState[] = []
    const bridge = createBrowserOSOnboardingBridge({
      chrome: null,
      mockTiming: 'sync',
    })
    bridge.registerReceiver((state) => states.push(state))

    bridge.pageReady()
    bridge.startImport({
      sourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
      items: MOCK_BROWSEROS_IMPORT_SOURCES[0].recommendedItems,
    })

    expect(bridge.isMock).toBe(true)
    expect(states.map((state) => state.status)).toEqual([
      'detecting',
      'ready',
      'importing',
      'importing',
      'succeeded',
    ])
    expect(states.at(-1)?.progress?.completedItems).toEqual(
      MOCK_BROWSEROS_IMPORT_SOURCES[0].recommendedItems,
    )
    expect(states[2]?.progress).toMatchObject({
      currentSourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
      currentSourceName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
      completedSources: 0,
      totalSources: 1,
    })
    expect(states[2]?.results).toEqual([
      {
        sourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
        displayName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
        status: 'importing',
      },
    ])
    expect(states.at(-1)?.progress).toMatchObject({
      completedSources: 1,
      totalSources: 1,
    })
    expect(states.at(-1)?.results).toEqual([
      {
        sourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
        displayName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
        status: 'succeeded',
      },
    ])
  })
})
