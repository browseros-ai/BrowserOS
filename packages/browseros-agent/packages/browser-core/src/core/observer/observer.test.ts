import { describe, expect, test } from 'bun:test'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { PageManager } from '../pages'
import type { AXNode } from '../snapshot/ax-types'
import type { FrameRegistry } from './frames'
import { Observer } from './observer'

function ax(
  nodeId: string,
  role: string,
  opts: Partial<AXNode> & { children?: string[] } = {},
): AXNode {
  const { children, ...rest } = opts
  return {
    nodeId,
    role: { type: 'role', value: role },
    childIds: children,
    ...rest,
  }
}

function name(value: string): AXNode['name'] {
  return { type: 'computedString', value }
}

interface StubState {
  loaderId: string
  url: string
  nodes: AXNode[]
  childLoaderId?: string
  childNodes?: AXNode[]
  failAxTree?: boolean
  onFrameTreeRead?: () => void
}

function stubObserverHarness(state: StubState): Observer {
  const session = {
    Page: {
      getFrameTree: async () => {
        state.onFrameTreeRead?.()
        return {
          frameTree: {
            frame: {
              id: 'main',
              loaderId: state.loaderId,
              url: state.url,
              domainAndRegistry: '',
              securityOrigin: '',
              mimeType: 'text/html',
              secureContextType: 'Secure',
              crossOriginIsolatedContextType: 'NotIsolated',
              gatedAPIFeatures: [],
            },
            childFrames:
              state.childLoaderId === undefined
                ? undefined
                : [
                    {
                      frame: {
                        id: 'child',
                        parentId: 'main',
                        loaderId: state.childLoaderId,
                        url: `${state.url}frame`,
                        domainAndRegistry: '',
                        securityOrigin: '',
                        mimeType: 'text/html',
                        secureContextType: 'Secure',
                        crossOriginIsolatedContextType: 'NotIsolated',
                        gatedAPIFeatures: [],
                      },
                    },
                  ],
          },
        }
      },
    },
    DOM: {
      describeNode: async () => ({
        node: { contentDocument: { frameId: 'child' } },
      }),
    },
    Accessibility: {
      getFullAXTree: async ({ frameId }: { frameId?: string } = {}) => {
        if (state.failAxTree) throw new Error('AX tree failed')
        return {
          nodes: frameId === 'child' ? (state.childNodes ?? []) : state.nodes,
        }
      },
    },
    Runtime: {
      evaluate: async () => ({ result: { value: [] } }),
    },
  } as unknown as ProtocolApi

  const pages = {
    getSession: async () => ({ targetId: 'target-1', session, url: state.url }),
    refresh: async () => ({ url: state.url }),
  } as unknown as PageManager

  const frames = {
    resolveFrameTarget: (_pageId: number, frameId?: string) => ({
      session,
      axParams: frameId === undefined ? {} : { frameId },
    }),
  } as unknown as FrameRegistry

  return new Observer(pages, frames, 1)
}

describe('Observer stable refs', () => {
  test('diff keeps unchanged same-document nodes matched after insertion', async () => {
    const state = {
      loaderId: 'loader-1',
      url: 'https://example.com/',
      nodes: [
        ax('1', 'RootWebArea', { children: ['2', '3'] }),
        ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
        ax('3', 'link', { name: name('B'), backendDOMNodeId: 2 }),
      ],
    }
    const observer = stubObserverHarness(state)

    await observer.snapshot()
    state.nodes = [
      ax('1', 'RootWebArea', { children: ['4', '2', '3'] }),
      ax('4', 'button', { name: name('X'), backendDOMNodeId: 3 }),
      ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
      ax('3', 'link', { name: name('B'), backendDOMNodeId: 2 }),
    ]

    const diff = await observer.diff()

    expect(diff.added).toBe(1)
    expect(diff.removed).toBe(0)
    expect(diff.text).toContain('+ button "X" [ref=e3]')
    expect(diff.text).toContain('1 added, 0 removed')
  })

  test('reload resets the public ref namespace', async () => {
    const state = {
      loaderId: 'loader-1',
      url: 'https://example.com/',
      nodes: [
        ax('1', 'RootWebArea', { children: ['2', '3'] }),
        ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
        ax('3', 'button', { name: name('B'), backendDOMNodeId: 2 }),
      ],
    }
    const observer = stubObserverHarness(state)

    await observer.snapshot()
    state.loaderId = 'loader-2'
    state.nodes = [
      ax('1', 'RootWebArea', { children: ['4'] }),
      ax('4', 'button', { name: name('Reloaded'), backendDOMNodeId: 10 }),
    ]

    const snapshot = await observer.snapshot()

    expect(snapshot.text).toBe('- button "Reloaded" [ref=e1]')
  })

  test('same-document navigation resets the public ref namespace', async () => {
    const state = {
      loaderId: 'loader-1',
      url: 'https://example.com/one',
      nodes: [
        ax('1', 'RootWebArea', { children: ['2', '3'] }),
        ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
        ax('3', 'button', { name: name('B'), backendDOMNodeId: 2 }),
      ],
    }
    const observer = stubObserverHarness(state)

    await observer.snapshot()
    state.url = 'https://example.com/two'
    state.nodes = [
      ax('1', 'RootWebArea', { children: ['4'] }),
      ax('4', 'button', { name: name('Navigated'), backendDOMNodeId: 10 }),
    ]

    const snapshot = await observer.snapshot()

    expect(snapshot.text).toBe('- button "Navigated" [ref=e1]')
  })

  test('failed captures do not replace the last committed refs', async () => {
    const state = {
      loaderId: 'loader-1',
      url: 'https://example.com/',
      nodes: [
        ax('1', 'RootWebArea', { children: ['2'] }),
        ax('2', 'button', { name: name('A'), backendDOMNodeId: 1 }),
      ],
      failAxTree: false,
    }
    const observer = stubObserverHarness(state)

    await observer.snapshot()
    state.failAxTree = true

    await expect(observer.snapshot()).rejects.toThrow('AX tree failed')
    expect(observer.lastRefs.get('e1')).toMatchObject({
      backendNodeId: 1,
      name: 'A',
    })
  })

  test('child-frame document churn falls back without failing the page capture', async () => {
    const state = {
      loaderId: 'loader-1',
      childLoaderId: 'child-loader-1',
      url: 'https://example.com/',
      nodes: [
        ax('1', 'RootWebArea', { children: ['2', '3'] }),
        ax('2', 'button', { name: name('Outer'), backendDOMNodeId: 1 }),
        ax('3', 'Iframe', { backendDOMNodeId: 2 }),
      ],
      childNodes: [
        ax('child-root', 'RootWebArea', { children: ['child-button'] }),
        ax('child-button', 'button', {
          name: name('Inner'),
          backendDOMNodeId: 1,
        }),
      ],
      onFrameTreeRead: undefined as (() => void) | undefined,
    }
    let frameTreeReads = 0
    state.onFrameTreeRead = () => {
      frameTreeReads++
      if (frameTreeReads === 2) state.childLoaderId = 'child-loader-2'
    }
    const observer = stubObserverHarness(state)

    const snapshot = await observer.snapshot()

    expect(snapshot.text).toBe(
      [
        '- button "Outer" [ref=e1]',
        '- iframe',
        '  - button "Inner" [ref=e2]',
      ].join('\n'),
    )
  })
})
