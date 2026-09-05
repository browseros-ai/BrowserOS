/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  BROWSEROS_ONBOARDING_API_VERSION,
  type BrowserOSImportProgress,
  type BrowserOSImportSource,
  type BrowserOSImportSourceResult,
  type BrowserOSOnboardingChrome,
  BrowserOSOnboardingMessage,
  type BrowserOSOnboardingState,
  type BrowserOSSetupState,
  type BrowserOSStartImportRequest,
} from './browseros-onboarding-api'
import { MOCK_BROWSEROS_IMPORT_SOURCES } from './onboarding-v2.helpers'

export interface BrowserOSOnboardingBridge {
  isMock: boolean
  complete(): void
  pageReady(): void
  refreshSources(): void
  retrySetup(): void
  registerReceiver(
    receiveState: (state: BrowserOSOnboardingState) => void,
  ): () => void
  startImport(request: BrowserOSStartImportRequest): void
}

interface BrowserOSOnboardingBridgeOptions {
  chrome?: BrowserOSOnboardingChrome | null
  mockTiming?: 'delayed' | 'sync'
  /** Explicit preview outcome; setup never infers readiness from a timer. */
  mockSetupResult?: Exclude<BrowserOSSetupState, 'idle'>
}

const MOCK_READY_DELAY_MS = 250
const MOCK_PROGRESS_DELAY_MS = 650
const MOCK_SUCCESS_DELAY_MS = 1300

function getHostWindow(): Window | undefined {
  if (typeof window === 'undefined') return undefined
  return window
}

function getChromeBridge(
  chromeOverride: BrowserOSOnboardingChrome | null | undefined,
): BrowserOSOnboardingChrome | null {
  if (chromeOverride !== undefined) return chromeOverride
  const candidate = (
    globalThis as typeof globalThis & {
      chrome?: BrowserOSOnboardingChrome
    }
  ).chrome
  return typeof candidate?.send === 'function' ? candidate : null
}

function createState(
  status: BrowserOSOnboardingState['status'],
  progress?: BrowserOSImportProgress,
  results?: BrowserOSImportSourceResult[],
): BrowserOSOnboardingState {
  return {
    apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
    status,
    sources: [...MOCK_BROWSEROS_IMPORT_SOURCES],
    ...(progress ? { progress } : {}),
    ...(results ? { results } : {}),
  }
}

function emitMockState(state: BrowserOSOnboardingState) {
  getHostWindow()?.browserosOnboarding?.receiveState(state)
}

function scheduleMockState(state: BrowserOSOnboardingState, delayMs: number) {
  const hostWindow = getHostWindow()
  const schedule = hostWindow?.setTimeout ?? globalThis.setTimeout
  schedule(() => emitMockState(state), delayMs)
}

function recommendedItemsFor(request: BrowserOSStartImportRequest) {
  if (request.items?.length) return request.items
  return (
    MOCK_BROWSEROS_IMPORT_SOURCES.find(
      (source) => source.id === request.sourceId,
    )?.recommendedItems ?? []
  )
}

function mockSourceFor(request: BrowserOSStartImportRequest) {
  return MOCK_BROWSEROS_IMPORT_SOURCES.find(
    (source) => source.id === request.sourceId,
  )
}

function sourceDisplayNameFor(
  request: BrowserOSStartImportRequest,
  source: BrowserOSImportSource | undefined,
) {
  return source?.displayName ?? request.sourceId
}

function emitMockImport(request: BrowserOSStartImportRequest, sync: boolean) {
  const items = recommendedItemsFor(request)
  const source = mockSourceFor(request)
  const sourceName = sourceDisplayNameFor(request, source)
  const started = createState(
    'importing',
    {
      currentItem: items[0],
      currentSourceId: request.sourceId,
      currentSourceName: sourceName,
      completedItems: [],
      totalItems: items.length,
      completedSources: 0,
      totalSources: 1,
    },
    [
      {
        sourceId: request.sourceId,
        displayName: sourceName,
        status: 'importing',
      },
    ],
  )
  const halfway = createState(
    'importing',
    {
      currentItem: items[1],
      currentSourceId: request.sourceId,
      currentSourceName: sourceName,
      completedItems: items.slice(0, 1),
      totalItems: items.length,
      completedSources: 0,
      totalSources: 1,
    },
    [
      {
        sourceId: request.sourceId,
        displayName: sourceName,
        status: 'importing',
      },
    ],
  )
  const succeeded = createState(
    'succeeded',
    {
      completedItems: items,
      totalItems: items.length,
      completedSources: 1,
      totalSources: 1,
    },
    [
      {
        sourceId: request.sourceId,
        displayName: sourceName,
        status: 'succeeded',
      },
    ],
  )

  emitMockState(started)
  if (sync) {
    emitMockState(halfway)
    emitMockState(succeeded)
    return
  }
  scheduleMockState(halfway, MOCK_PROGRESS_DELAY_MS)
  scheduleMockState(succeeded, MOCK_SUCCESS_DELAY_MS)
}

/** Creates the Chromium WebUI bridge, falling back to mock state in Vite. */
export function createBrowserOSOnboardingBridge(
  options: BrowserOSOnboardingBridgeOptions = {},
): BrowserOSOnboardingBridge {
  const chromeBridge = getChromeBridge(options.chrome)
  const isMock = !chromeBridge
  const mockIsSync = options.mockTiming === 'sync'

  let currentState: BrowserOSOnboardingState | undefined
  let receiver: ((state: BrowserOSOnboardingState) => void) | undefined
  let completionRequested = false

  function updateState(state: BrowserOSOnboardingState) {
    // Older native snapshots omit setupState. Keep the local handoff visible
    // without changing importer status/error or requiring both releases at once.
    const setupState = state.setupState ?? currentState?.setupState
    currentState = { ...state, ...(setupState ? { setupState } : {}) }
    if (currentState.setupState && currentState.setupState !== 'idle') {
      completionRequested = true
    }
    receiver?.(currentState)
  }

  function requestSetup(message: BrowserOSOnboardingMessage) {
    const state = currentState ?? {
      apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
      status: 'idle',
      sources: [],
    }
    // Latch and notify before chrome.send: native can reply synchronously, and
    // its failure must win over optimistic preparing. Navigation belongs to
    // Chromium after extension readiness, even when setupState becomes ready.
    updateState({ ...state, setupState: 'preparing' })
    if (chromeBridge) {
      chromeBridge.send(message)
      return
    }
    if (options.mockSetupResult && options.mockSetupResult !== 'preparing') {
      updateState({ ...state, setupState: options.mockSetupResult })
    }
  }

  return {
    isMock,
    complete() {
      if (completionRequested) return
      requestSetup(BrowserOSOnboardingMessage.COMPLETE)
    },
    retrySetup() {
      // Import failure cannot enable this action. A pending retry consumes the
      // setup failure immediately, so rapid repeated clicks send only once.
      if (currentState?.setupState !== 'failed') return
      requestSetup(BrowserOSOnboardingMessage.RETRY_SETUP)
    },
    pageReady() {
      if (!isMock) {
        chromeBridge.send(BrowserOSOnboardingMessage.PAGE_READY)
        return
      }
      emitMockState(createState('detecting'))
      if (mockIsSync) {
        emitMockState(createState('ready'))
        return
      }
      scheduleMockState(createState('ready'), MOCK_READY_DELAY_MS)
    },
    refreshSources() {
      if (!isMock) {
        chromeBridge.send(BrowserOSOnboardingMessage.REFRESH_SOURCES)
        return
      }
      emitMockState(createState('detecting'))
      if (mockIsSync) {
        emitMockState(createState('ready'))
        return
      }
      scheduleMockState(createState('ready'), MOCK_READY_DELAY_MS)
    },
    registerReceiver(receiveState) {
      const hostWindow = getHostWindow()
      if (!hostWindow) return () => undefined
      const previousClient = hostWindow.browserosOnboarding
      const previousReceiver = receiver
      receiver = receiveState
      const client = { receiveState: updateState }
      hostWindow.browserosOnboarding = client
      // Effect reattachment (including StrictMode) must not lose a synchronous
      // pageReady snapshot or issue another completion request.
      if (currentState) receiveState(currentState)
      return () => {
        if (hostWindow.browserosOnboarding !== client) return
        receiver = previousReceiver
        if (previousClient) {
          hostWindow.browserosOnboarding = previousClient
          return
        }
        delete hostWindow.browserosOnboarding
      }
    },
    startImport(request) {
      if (request.items && request.items.length === 0) return
      if (!isMock) {
        chromeBridge.send(BrowserOSOnboardingMessage.START_IMPORT, [request])
        return
      }
      emitMockImport(request, mockIsSync)
    },
  }
}
