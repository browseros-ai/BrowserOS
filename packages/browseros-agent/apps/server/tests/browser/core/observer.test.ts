import { describe, expect, test } from 'bun:test'
import type { FrameRegistry } from '@browseros/browser-core/core/observer/frames'
import { Observer } from '@browseros/browser-core/core/observer/observer'
import type {
  PageManager,
  PageSession,
} from '@browseros/browser-core/core/pages'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

type HarnessOptions = {
  frameTreeUrl?: string
  frameTreeFragment?: string
  frameTreeError?: Error
  refreshedUrl?: string
  refreshError?: Error
}

function createObserverHarness(opts: HarnessOptions = {}) {
  const runtimeExpressions: string[] = []
  let frameTreeCalls = 0
  let refreshCalls = 0

  const session = {
    Page: {
      getFrameTree: async () => {
        frameTreeCalls++
        if (opts.frameTreeError) throw opts.frameTreeError
        return {
          frameTree: {
            frame: {
              id: 'main',
              loaderId: 'loader',
              url: opts.frameTreeUrl ?? 'https://frame.example/',
              urlFragment: opts.frameTreeFragment,
              domainAndRegistry: '',
              securityOrigin: '',
              mimeType: 'text/html',
              secureContextType: 'Secure',
              crossOriginIsolatedContextType: 'NotIsolated',
              gatedAPIFeatures: [],
            },
          },
        }
      },
    },
    Runtime: {
      evaluate: async ({ expression }: { expression: string }) => {
        runtimeExpressions.push(expression)
        if (expression === 'location.href') {
          return { result: { value: 'https://runtime.example/' } }
        }
        return { result: { value: [] } }
      },
    },
    Accessibility: {
      getFullAXTree: async () => ({
        nodes: [
          {
            nodeId: 'root',
            role: { type: 'role', value: 'RootWebArea' },
            childIds: ['button'],
          },
          {
            nodeId: 'button',
            role: { type: 'role', value: 'button' },
            name: { type: 'computedString', value: 'Save' },
            backendDOMNodeId: 42,
          },
        ],
      }),
    },
  } as unknown as ProtocolApi

  const pages = {
    getSession: async (): Promise<PageSession> => ({
      targetId: 'target-1',
      session,
      url: 'https://cached.example/',
    }),
    refresh: async () => {
      refreshCalls++
      if (opts.refreshError) throw opts.refreshError
      if (opts.refreshedUrl === undefined) return undefined
      return { url: opts.refreshedUrl }
    },
  } as unknown as PageManager

  const frames = {
    resolveFrameTarget: () => ({ session, axParams: {} }),
  } as unknown as FrameRegistry

  return {
    observer: new Observer(pages, frames, 1),
    get frameTreeCalls() {
      return frameTreeCalls
    },
    get refreshCalls() {
      return refreshCalls
    },
    runtimeExpressions,
  }
}

describe('Observer URL lookup', () => {
  test('reads the main-frame URL through Page.getFrameTree without location.href evaluation', async () => {
    const harness = createObserverHarness({
      frameTreeUrl: 'https://frame.example/path',
      refreshedUrl: 'https://registry.example/',
    })

    const snapshot = await harness.observer.snapshot()

    expect(snapshot.url).toBe('https://frame.example/path')
    expect(harness.frameTreeCalls).toBe(2)
    expect(harness.refreshCalls).toBe(0)
    expect(harness.runtimeExpressions).not.toContain('location.href')
  })

  test('preserves URL fragments from frame tree metadata', async () => {
    const harness = createObserverHarness({
      frameTreeUrl: 'https://frame.example/path',
      frameTreeFragment: '#section',
    })

    const snapshot = await harness.observer.snapshot()

    expect(snapshot.url).toBe('https://frame.example/path#section')
    expect(harness.runtimeExpressions).not.toContain('location.href')
  })

  test('falls back to the refreshed tab registry URL when frame tree URL lookup fails', async () => {
    const harness = createObserverHarness({
      frameTreeError: new Error('detached target'),
      refreshedUrl: 'https://registry.example/current',
    })

    const snapshot = await harness.observer.snapshot()

    expect(snapshot.url).toBe('https://registry.example/current')
    expect(harness.frameTreeCalls).toBe(2)
    expect(harness.refreshCalls).toBe(2)
    expect(harness.runtimeExpressions).not.toContain('location.href')
  })

  test('returns unknown when frame tree and tab registry fallbacks both fail', async () => {
    const harness = createObserverHarness({
      frameTreeError: new Error('detached target'),
      refreshError: new Error('registry unavailable'),
    })

    const snapshot = await harness.observer.snapshot()

    expect(snapshot.url).toBe('unknown')
    expect(harness.frameTreeCalls).toBe(2)
    expect(harness.refreshCalls).toBe(2)
    expect(harness.runtimeExpressions).not.toContain('location.href')
  })
})
