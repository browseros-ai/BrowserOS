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

describe('native setup lifecycle', () => {
  function nativeBridge(onSend?: (message: string) => void) {
    installWindow()
    const sent: string[] = []
    const states: BrowserOSOnboardingState[] = []
    const bridge = createBrowserOSOnboardingBridge({
      chrome: {
        send(message) {
          sent.push(message)
          onSend?.(message)
        },
      },
    })
    bridge.registerReceiver((state) => states.push(state))
    const receive = (state: Partial<BrowserOSOnboardingState>) => {
      window.browserosOnboarding?.receiveState({
        apiVersion: 1,
        status: 'ready',
        sources: [],
        ...state,
      })
    }
    return { bridge, sent, states, receive }
  }

  it('shows preparing before sending COMPLETE and sends completion only once', () => {
    const { bridge, sent, states } = nativeBridge((message) => {
      if (message === BrowserOSOnboardingMessage.COMPLETE) {
        expect(states.at(-1)?.setupState).toBe('preparing')
      }
    })
    bridge.complete()
    bridge.complete()
    expect(sent).toEqual([BrowserOSOnboardingMessage.COMPLETE])
    expect(states.at(-1)?.setupState).toBe('preparing')
  })

  it('keeps native synchronous failure visible after COMPLETE', () => {
    const { bridge, states } = nativeBridge((message) => {
      if (message === BrowserOSOnboardingMessage.COMPLETE) {
        window.browserosOnboarding?.receiveState({
          apiVersion: 1,
          status: 'ready',
          sources: [],
          setupState: 'failed',
        })
      }
    })
    bridge.complete()
    expect(states.at(-1)?.setupState).toBe('failed')
  })

  it('allows one Retry per native setup failure and returns to preparing immediately', () => {
    const { bridge, sent, states, receive } = nativeBridge()
    bridge.retrySetup()
    receive({
      status: 'failed',
      error: { code: 'import_failed', message: 'Import failed' },
    })
    bridge.retrySetup()
    expect(sent).toEqual([])
    receive({ setupState: 'failed' })
    bridge.complete()
    expect(sent).toEqual([])
    bridge.retrySetup()
    bridge.retrySetup()
    bridge.complete()
    expect(sent).toEqual([BrowserOSOnboardingMessage.RETRY_SETUP])
    expect(states.at(-1)?.setupState).toBe('preparing')
    receive({ setupState: 'failed' })
    bridge.retrySetup()
    expect(sent).toEqual([
      BrowserOSOnboardingMessage.RETRY_SETUP,
      BrowserOSOnboardingMessage.RETRY_SETUP,
    ])
  })

  it('preserves a synchronous retry failure instead of overwriting it with preparing', () => {
    const { bridge, states, receive } = nativeBridge((message) => {
      if (message === BrowserOSOnboardingMessage.RETRY_SETUP) {
        window.browserosOnboarding?.receiveState({
          apiVersion: 1,
          status: 'ready',
          sources: [],
          setupState: 'failed',
        })
      }
    })
    receive({ setupState: 'failed' })
    bridge.retrySetup()
    expect(states.at(-1)?.setupState).toBe('failed')
  })

  for (const setupState of ['preparing', 'failed', 'ready'] as const) {
    it(`restores native ${setupState} on page-ready without completing again`, () => {
      const { bridge, states, sent } = nativeBridge((message) => {
        if (message === BrowserOSOnboardingMessage.PAGE_READY) {
          window.browserosOnboarding?.receiveState({
            apiVersion: 1,
            status: 'ready',
            sources: [],
            setupState,
          })
        }
      })
      bridge.pageReady()
      expect(states.at(-1)?.setupState).toBe(setupState)
      bridge.complete()
      expect(sent).toEqual([BrowserOSOnboardingMessage.PAGE_READY])
    })
  }

  it('keeps importer snapshots unchanged and tolerates legacy native states', () => {
    const { bridge, states, receive } = nativeBridge()
    const error = { code: 'import_failed', message: 'Import failed' }
    receive({ status: 'failed', error })
    expect(states.at(-1)?.setupState).toBeUndefined()
    bridge.complete()
    expect(states.at(-1)).toMatchObject({
      status: 'failed',
      error,
      setupState: 'preparing',
    })
    receive({ status: 'completed' })
    expect(states.at(-1)).toMatchObject({
      status: 'completed',
      setupState: 'preparing',
    })
  })

  it('replays the latest snapshot when React reattaches its receiver', () => {
    const { bridge, receive } = nativeBridge()
    receive({ setupState: 'failed' })
    const cleanup = bridge.registerReceiver(() => {})
    cleanup()
    const states: BrowserOSOnboardingState[] = []
    bridge.registerReceiver((state) => states.push(state))
    expect(states.at(-1)?.setupState).toBe('failed')
  })

  for (const mockSetupResult of ['preparing', 'failed', 'ready'] as const) {
    it(`mocks ${mockSetupResult} without readiness timers or navigation`, () => {
      installWindow()
      const states: BrowserOSOnboardingState[] = []
      const bridge = createBrowserOSOnboardingBridge({
        chrome: null,
        mockTiming: 'sync',
        mockSetupResult,
      })
      bridge.registerReceiver((state) => states.push(state))
      bridge.pageReady()
      states.length = 0
      bridge.complete()
      bridge.complete()
      expect(states[0]?.setupState).toBe('preparing')
      expect(states.at(-1)?.setupState).toBe(mockSetupResult)
      expect(states.every((state) => state.status === 'ready')).toBe(true)
      if (mockSetupResult === 'failed') {
        states.length = 0
        bridge.retrySetup()
        expect(states.map((state) => state.setupState)).toEqual([
          'preparing',
          'failed',
        ])
      }
    })
  }
})
