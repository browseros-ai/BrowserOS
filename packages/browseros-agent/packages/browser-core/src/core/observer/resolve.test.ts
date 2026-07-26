import { describe, expect, test } from 'bun:test'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { AXNode } from '../snapshot/ax-types'
import { RefMap } from '../snapshot/refs'
import { resolveRefEntry } from './resolve'

// Stubs the minimal CDP surface resolveRefEntry uses, so the two-tier logic + nth matching
// are exercised without a browser. (End-to-end staleness against a live DOM is covered by the
// integration test in apps/server/tests/browser/browser-core.)
function stubSession(opts: {
  live: Set<number>
  axTree?: AXNode[]
  releases?: string[]
}): ProtocolApi {
  return {
    DOM: {
      resolveNode: async ({ backendNodeId }: { backendNodeId: number }) =>
        opts.live.has(backendNodeId)
          ? { object: { objectId: `obj-${backendNodeId}` } }
          : Promise.reject(new Error('No node with given id')),
    },
    Accessibility: {
      getFullAXTree: async () => ({ nodes: opts.axTree ?? [] }),
    },
    Runtime: {
      releaseObject: async ({ objectId }: { objectId: string }) => {
        opts.releases?.push(objectId)
      },
    },
  } as unknown as ProtocolApi
}

function axButton(nodeId: string, name: string, backendId: number): AXNode {
  return {
    nodeId,
    role: { type: 'role', value: 'button' },
    name: { type: 'computedString', value: name },
    backendDOMNodeId: backendId,
  }
}

describe('resolveRefEntry', () => {
  test('tier 1 returns the cached backendNodeId when still live', async () => {
    const releases: string[] = []
    const refs = new RefMap()
    const ref = refs.mint({ backendNodeId: 10, role: 'button', name: 'OK' })
    const resolved = await resolveRefEntry(
      stubSession({ live: new Set([10]), releases }),
      refs.get(ref) as never,
    )
    expect(resolved.backendNodeId).toBe(10)
    expect(releases).toEqual(['obj-10'])
  })

  test('tier 2 re-queries by role+name+nth when the cached node is stale', async () => {
    const refs = new RefMap()
    refs.mint({ backendNodeId: 10, role: 'button', name: 'OK' }) // nth 0
    const second = refs.mint({ backendNodeId: 11, role: 'button', name: 'OK' }) // nth 1

    // 10 and 11 are gone; the DOM now exposes fresh ids 20, 21 in order.
    const session = stubSession({
      live: new Set([20, 21]),
      axTree: [
        {
          nodeId: 'root',
          role: { type: 'role', value: 'RootWebArea' },
          childIds: ['a', 'b'],
        },
        axButton('a', 'OK', 20),
        axButton('b', 'OK', 21),
      ],
    })

    const resolved = await resolveRefEntry(session, refs.get(second) as never)
    // nth 1 must map to the second matching node (21), not the first.
    expect(resolved.backendNodeId).toBe(21)
    // cache was refreshed
    expect(refs.get(second)?.backendNodeId).toBe(21)
  })

  test('throws when a stale ref cannot be re-found', async () => {
    const refs = new RefMap()
    const ref = refs.mint({ backendNodeId: 10, role: 'button', name: 'Gone' })
    await expect(
      resolveRefEntry(
        stubSession({ live: new Set(), axTree: [] }),
        refs.get(ref) as never,
      ),
    ).rejects.toThrow(/Stale ref/)
  })
})
