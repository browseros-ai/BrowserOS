/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { inheritTabOwnership } from '../services/session-tabs'
import { releaseClaimsForTarget } from '../services/tab-claims'
import { logger } from './logger'

export interface TargetLifecycleInfo {
  targetId: string
  tabId?: number
  openerId?: string
}

interface BrowserTabIdentity {
  tabId: number
  targetId: string
}

export interface TabTargetSource {
  epoch(): number
  enableDiscovery(): Promise<void>
  listTabs(): Promise<BrowserTabIdentity[]>
  getTab(tabId: number): Promise<BrowserTabIdentity>
  onTargetCreated(handler: (info: TargetLifecycleInfo) => void): () => void
  onTargetInfoChanged(handler: (info: TargetLifecycleInfo) => void): () => void
  onTargetDestroyed(handler: (targetId: string) => void): () => void
}

interface TabTargetMapOptions {
  releaseTargetClaims?: (targetId: string) => Promise<void> | void
  inheritTabOwner?: (
    openerTabId: number,
    tabId: number,
    targetId: string,
  ) => Promise<void> | void
  now?: () => number
}

/** Keeps teardown-time recorder flushes resolvable; claims still close immediately. */
const GRACE_MS = 5 * 60 * 1_000

/** Maintains the browser tab id to stable CDP target id identity boundary. */
export class TabTargetMap {
  private readonly targetByTab = new Map<number, string>()
  private readonly tabByTarget = new Map<string, number>()
  /**
   * Chrome tab ids increase monotonically within a browser session, so live
   * entries cannot alias these destroyed entries; lookups still prefer live.
   */
  private readonly recentlyDestroyed = new Map<
    number,
    { targetId: string; destroyedAt: number }
  >()
  private readonly unsubscribers: Array<() => void> = []
  private readonly releaseTargetClaims: (
    targetId: string,
  ) => Promise<void> | void
  private readonly inheritTabOwner: (
    openerTabId: number,
    tabId: number,
    targetId: string,
  ) => Promise<void> | void
  private readonly now: () => number
  private sourceEpoch = -1
  private rebuildPromise: Promise<void> | null = null

  constructor(
    private readonly source: TabTargetSource,
    options: TabTargetMapOptions = {},
  ) {
    this.releaseTargetClaims =
      options.releaseTargetClaims ?? releaseClaimsForTarget
    this.inheritTabOwner = options.inheritTabOwner ?? inheritTabOwnership
    this.now = options.now ?? Date.now
  }

  /** Subscribes to target events and seeds the map from the browser's live tabs. */
  async start(): Promise<void> {
    if (this.unsubscribers.length === 0) {
      this.unsubscribers.push(
        this.source.onTargetCreated((info) => this.onCreated(info)),
        this.source.onTargetInfoChanged((info) => this.upsert(info)),
        this.source.onTargetDestroyed((targetId) => this.remove(targetId)),
      )
    }
    await this.rebuild()
  }

  stop(): void {
    for (const unsubscribe of this.unsubscribers.splice(0)) unsubscribe()
  }

  /** Resolves a Chrome tab id, falling back to Browser.getTabInfo on a miss. */
  async targetForTab(tabId: number): Promise<string | undefined> {
    await this.rebuildAfterReconnect()
    const cached = this.targetByTab.get(tabId)
    if (cached) return cached

    const now = this.now()
    this.pruneRecentlyDestroyed(now)
    const destroyed = this.recentlyDestroyed.get(tabId)
    if (destroyed) return destroyed.targetId

    try {
      const tab = await this.source.getTab(tabId)
      this.upsert(tab)
      return tab.targetId
    } catch {
      return undefined
    }
  }

  tabForTarget(targetId: string): number | undefined {
    return this.tabByTarget.get(targetId)
  }

  private async rebuildAfterReconnect(): Promise<void> {
    if (this.source.epoch() !== this.sourceEpoch) await this.rebuild()
  }

  private rebuild(): Promise<void> {
    if (this.rebuildPromise) return this.rebuildPromise
    this.rebuildPromise = this.loadTabs().finally(() => {
      this.rebuildPromise = null
    })
    return this.rebuildPromise
  }

  private async loadTabs(): Promise<void> {
    await this.source.enableDiscovery()
    const tabs = await this.source.listTabs()
    const liveTargets = new Set(tabs.map((tab) => tab.targetId))
    for (const targetId of this.tabByTarget.keys()) {
      if (!liveTargets.has(targetId)) this.remove(targetId)
    }
    this.targetByTab.clear()
    this.tabByTarget.clear()
    for (const tab of tabs) this.upsert(tab)
    this.sourceEpoch = this.source.epoch()
  }

  private upsert(info: TargetLifecycleInfo): void {
    if (info.tabId === undefined) return

    const previousTarget = this.targetByTab.get(info.tabId)
    if (previousTarget && previousTarget !== info.targetId) {
      this.tabByTarget.delete(previousTarget)
    }
    const previousTab = this.tabByTarget.get(info.targetId)
    if (previousTab !== undefined && previousTab !== info.tabId) {
      this.targetByTab.delete(previousTab)
    }
    this.targetByTab.set(info.tabId, info.targetId)
    this.tabByTarget.set(info.targetId, info.tabId)
  }

  private onCreated(info: TargetLifecycleInfo): void {
    this.upsert(info)
    if (info.tabId === undefined || info.openerId === undefined) return
    const openerTabId = this.tabByTarget.get(info.openerId)
    if (openerTabId === undefined || openerTabId === info.tabId) return
    try {
      const inherited = this.inheritTabOwner(
        openerTabId,
        info.tabId,
        info.targetId,
      )
      if (inherited) {
        void inherited.catch((error) =>
          logger.warn('popup tab ownership inheritance failed', {
            openerTabId,
            tabId: info.tabId,
            error: error instanceof Error ? error.message : String(error),
          }),
        )
      }
    } catch (error) {
      logger.warn('popup tab ownership inheritance failed', {
        openerTabId,
        tabId: info.tabId,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  private remove(targetId: string): void {
    const tabId = this.tabByTarget.get(targetId)
    if (tabId !== undefined) {
      this.targetByTab.delete(tabId)
      const now = this.now()
      this.pruneRecentlyDestroyed(now)
      this.recentlyDestroyed.set(tabId, { targetId, destroyedAt: now })
    }
    this.tabByTarget.delete(targetId)
    try {
      const release = this.releaseTargetClaims(targetId)
      if (release) {
        void release.catch((error) => logReleaseFailure(targetId, error))
      }
    } catch (error) {
      logReleaseFailure(targetId, error)
    }
  }

  private pruneRecentlyDestroyed(now: number): void {
    for (const [tabId, entry] of this.recentlyDestroyed) {
      if (now - entry.destroyedAt >= GRACE_MS) {
        this.recentlyDestroyed.delete(tabId)
      }
    }
  }
}

let activeMap: TabTargetMap | null = null

/** Starts the process-wide tab identity map for the live browser session. */
export async function initializeTabTargets(
  session: BrowserSession,
): Promise<TabTargetMap> {
  activeMap?.stop()
  const map = new TabTargetMap(sourceFromBrowserSession(session))
  activeMap = map
  await map.start()
  return map
}

export function getTabTargetMap(): TabTargetMap | null {
  return activeMap
}

export function stopTabTargets(): void {
  activeMap?.stop()
  activeMap = null
}

function sourceFromBrowserSession(session: BrowserSession): TabTargetSource {
  return {
    epoch: () => session.connectionEpoch(),
    enableDiscovery: () =>
      session.protocol.Target.setDiscoverTargets({ discover: true }),
    listTabs: async () =>
      (await session.protocol.Browser.getTabs({ includeHidden: true })).tabs,
    getTab: async (tabId) =>
      (await session.protocol.Browser.getTabInfo({ tabId })).tab,
    onTargetCreated: (handler) =>
      session.protocol.Target.on('targetCreated', ({ targetInfo }) =>
        handler(targetInfo),
      ),
    onTargetInfoChanged: (handler) =>
      session.protocol.Target.on('targetInfoChanged', ({ targetInfo }) =>
        handler(targetInfo),
      ),
    onTargetDestroyed: (handler) =>
      session.protocol.Target.on('targetDestroyed', ({ targetId }) =>
        handler(targetId),
      ),
  }
}

function logReleaseFailure(targetId: string, error: unknown): void {
  logger.warn('failed to release claims for destroyed target', {
    targetId,
    error: error instanceof Error ? error.message : String(error),
  })
}
