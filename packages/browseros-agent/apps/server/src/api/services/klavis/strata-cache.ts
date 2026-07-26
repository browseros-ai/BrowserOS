/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { logger } from '../../../lib/logger'
import type { KlavisClient } from './client'
import type { StrataCreateResponse } from './types'

const DEFAULT_TTL_MS = 60 * 60 * 1000

interface CacheEntry {
  strataServerUrl: string
  strataId: string
  addedServers: string[]
  serverKey: string
  expiresAt: number
}

function normalizeServers(servers: readonly string[]): string {
  return [...new Set(servers)].sort().join(',')
}

function keyOf(browserosId: string, normalized: string): string {
  // The serverKey check below makes compact hash-key collisions harmless.
  const hash = Bun.hash(normalized).toString(16).padStart(16, '0')
  return `${browserosId}|${hash}`
}

/** Caches immutable Strata metadata, never live MCP client sessions. */
export class KlavisStrataCache {
  private entries = new Map<string, Promise<CacheEntry>>()

  constructor(private ttlMs: number = DEFAULT_TTL_MS) {}

  async getOrFetch(
    client: KlavisClient,
    browserosId: string,
    servers: readonly string[],
  ): Promise<StrataCreateResponse> {
    const normalized = normalizeServers(servers)
    const key = keyOf(browserosId, normalized)
    const existing = this.entries.get(key)

    if (existing) {
      const resolved = await existing.catch(() => null)
      if (
        resolved &&
        resolved.serverKey === normalized &&
        Date.now() < resolved.expiresAt
      ) {
        logger.debug('Klavis strata cache hit', { key })
        return this.toResponse(resolved)
      }
      if (this.entries.get(key) === existing) {
        this.entries.delete(key)
      }
    }

    logger.debug('Klavis strata cache miss', {
      key,
      serverCount: servers.length,
    })
    const inflight = this.fetch(client, browserosId, servers, normalized)
    this.entries.set(key, inflight)

    try {
      return this.toResponse(await inflight)
    } catch (err) {
      if (this.entries.get(key) === inflight) {
        this.entries.delete(key)
      }
      throw err
    }
  }

  invalidate(browserosId: string): void {
    const prefix = `${browserosId}|`
    let dropped = 0
    for (const key of this.entries.keys()) {
      if (key.startsWith(prefix)) {
        this.entries.delete(key)
        dropped++
      }
    }
    if (dropped > 0) {
      logger.debug('Klavis strata cache invalidated', {
        browserosId: browserosId.slice(0, 12),
        dropped,
      })
    }
  }

  clear(): void {
    this.entries.clear()
  }

  private async fetch(
    client: KlavisClient,
    browserosId: string,
    servers: readonly string[],
    normalized: string,
  ): Promise<CacheEntry> {
    const result = await client.createStrata(browserosId, [...servers])
    return {
      strataServerUrl: result.strataServerUrl,
      strataId: result.strataId,
      addedServers: result.addedServers,
      serverKey: normalized,
      expiresAt: Date.now() + this.ttlMs,
    }
  }

  private toResponse(entry: CacheEntry): StrataCreateResponse {
    return {
      strataServerUrl: entry.strataServerUrl,
      strataId: entry.strataId,
      addedServers: entry.addedServers,
    }
  }
}
