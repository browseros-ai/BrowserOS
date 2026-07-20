import { describe, expect, it } from 'bun:test'
import {
  TabTargetMap,
  type TabTargetSource,
  type TargetLifecycleInfo,
} from '../../src/lib/tab-targets'

class FakeSource implements TabTargetSource {
  currentEpoch = 1
  tabs: Array<{ tabId: number; targetId: string }> = []
  fallback = new Map<number, string>()
  listCalls = 0
  discoveryCalls = 0
  getCalls = 0
  private readonly created = new Set<(info: TargetLifecycleInfo) => void>()
  private readonly changed = new Set<(info: TargetLifecycleInfo) => void>()
  private readonly destroyed = new Set<(targetId: string) => void>()

  epoch(): number {
    return this.currentEpoch
  }

  async enableDiscovery(): Promise<void> {
    this.discoveryCalls++
  }

  async listTabs(): Promise<Array<{ tabId: number; targetId: string }>> {
    this.listCalls++
    return this.tabs
  }

  async getTab(tabId: number): Promise<{ tabId: number; targetId: string }> {
    this.getCalls++
    const targetId = this.fallback.get(tabId)
    if (!targetId) throw new Error('unknown tab')
    return { tabId, targetId }
  }

  onTargetCreated(handler: (info: TargetLifecycleInfo) => void): () => void {
    this.created.add(handler)
    return () => this.created.delete(handler)
  }

  onTargetInfoChanged(
    handler: (info: TargetLifecycleInfo) => void,
  ): () => void {
    this.changed.add(handler)
    return () => this.changed.delete(handler)
  }

  onTargetDestroyed(handler: (targetId: string) => void): () => void {
    this.destroyed.add(handler)
    return () => this.destroyed.delete(handler)
  }

  emitCreated(info: TargetLifecycleInfo): void {
    for (const handler of this.created) handler(info)
  }

  emitChanged(info: TargetLifecycleInfo): void {
    for (const handler of this.changed) handler(info)
  }

  emitDestroyed(targetId: string): void {
    for (const handler of this.destroyed) handler(targetId)
  }
}

describe('TabTargetMap', () => {
  it('rebuilds from live tabs and maintains both lookup directions', async () => {
    const source = new FakeSource()
    source.tabs = [
      { tabId: 11, targetId: 'target-a' },
      { tabId: 22, targetId: 'target-b' },
    ]
    const map = new TabTargetMap(source)

    await map.start()

    expect(await map.targetForTab(11)).toBe('target-a')
    expect(map.tabForTarget('target-b')).toBe(22)
    expect(source.discoveryCalls).toBe(1)
  })

  it('fills a lookup miss through Browser.getTabInfo and caches it', async () => {
    const source = new FakeSource()
    source.fallback.set(33, 'target-c')
    const map = new TabTargetMap(source)
    await map.start()

    expect(await map.targetForTab(33)).toBe('target-c')
    expect(await map.targetForTab(33)).toBe('target-c')
    expect(source.getCalls).toBe(1)
  })

  it('upserts from target lifecycle events once tabId is present', async () => {
    const source = new FakeSource()
    const map = new TabTargetMap(source)
    await map.start()

    source.emitCreated({ targetId: 'target-d' })
    expect(map.tabForTarget('target-d')).toBeUndefined()

    source.emitChanged({ targetId: 'target-d', tabId: 44 })
    expect(await map.targetForTab(44)).toBe('target-d')
  })

  it('inherits a popup from the live opener tab', async () => {
    const source = new FakeSource()
    source.tabs = [{ tabId: 11, targetId: 'opener-target' }]
    const inherited: Array<[number, number, string]> = []
    const map = new TabTargetMap(source, {
      inheritTabOwner: (openerTabId, tabId, targetId) => {
        inherited.push([openerTabId, tabId, targetId])
      },
    })
    await map.start()

    source.emitCreated({
      targetId: 'popup-target',
      tabId: 22,
      openerId: 'opener-target',
    })

    expect(inherited).toEqual([[11, 22, 'popup-target']])
    expect(await map.targetForTab(22)).toBe('popup-target')
  })

  it('resolves a destroyed target during grace and closes its claims immediately', async () => {
    const source = new FakeSource()
    source.tabs = [{ tabId: 55, targetId: 'target-e' }]
    const released: string[] = []
    const now = 1_000
    const map = new TabTargetMap(source, {
      releaseTargetClaims: async (targetId) => {
        released.push(targetId)
      },
      now: () => now,
    })
    await map.start()

    source.emitDestroyed('target-e')
    await Promise.resolve()

    expect(map.tabForTarget('target-e')).toBeUndefined()
    expect(await map.targetForTab(55)).toBe('target-e')
    expect(source.getCalls).toBe(0)
    expect(released).toEqual(['target-e'])
  })

  it('expires destroyed-target resolution after five minutes', async () => {
    const source = new FakeSource()
    source.tabs = [{ tabId: 56, targetId: 'target-expired' }]
    let now = 1_000
    const map = new TabTargetMap(source, { now: () => now })
    await map.start()

    source.emitDestroyed('target-expired')
    now += 5 * 60 * 1_000

    expect(await map.targetForTab(56)).toBeUndefined()
    expect(source.getCalls).toBe(1)
  })

  it('rebuilds before lookup after the CDP connection epoch changes', async () => {
    const source = new FakeSource()
    source.tabs = [{ tabId: 66, targetId: 'target-f' }]
    const released: string[] = []
    const map = new TabTargetMap(source, {
      releaseTargetClaims: (targetId) => {
        released.push(targetId)
      },
    })
    await map.start()

    source.currentEpoch++
    source.tabs = [{ tabId: 77, targetId: 'target-g' }]

    expect(await map.targetForTab(77)).toBe('target-g')
    expect(map.tabForTarget('target-f')).toBeUndefined()
    expect(released).toEqual(['target-f'])
    expect(source.listCalls).toBe(2)
    expect(source.discoveryCalls).toBe(2)
  })

  it('returns undefined when a tab cannot be resolved', async () => {
    const source = new FakeSource()
    const map = new TabTargetMap(source)
    await map.start()

    expect(await map.targetForTab(999)).toBeUndefined()
  })
})
