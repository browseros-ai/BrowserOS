/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { KlavisStrataCache } from '../../../../src/api/services/klavis/strata-cache'
import type {
  KlavisClient,
  StrataCreateResponse,
} from '../../../../src/lib/clients/klavis/klavis-client'

class StubKlavisClient {
  callCount = 0
  delayMs = 0
  shouldThrowOnce = false
  lastServers: string[] | null = null

  async createStrata(
    userId: string,
    servers: string[],
  ): Promise<StrataCreateResponse> {
    this.callCount++
    this.lastServers = servers
    if (this.shouldThrowOnce) {
      this.shouldThrowOnce = false
      throw new Error('boom')
    }
    if (this.delayMs > 0) {
      await new Promise((r) => setTimeout(r, this.delayMs))
    }
    return {
      strataServerUrl: `https://strata.test/${userId}/${servers.join('-')}`,
      strataId: `strata_${userId}`,
      addedServers: servers,
    }
  }
}

const asClient = (stub: StubKlavisClient): KlavisClient =>
  stub as unknown as KlavisClient

describe('KlavisStrataCache', () => {
  it('cache hit returns the same value without re-calling the client', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    const a = await cache.getOrFetch(asClient(client), 'user1', ['Gmail'])
    const b = await cache.getOrFetch(asClient(client), 'user1', ['Gmail'])
    expect(client.callCount).toBe(1)
    expect(a.strataServerUrl).toBe(b.strataServerUrl)
    expect(a.strataId).toBe(b.strataId)
  })

  it('normalizes server order — [Gmail, Linear] === [Linear, Gmail]', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'u', ['Gmail', 'Linear'])
    await cache.getOrFetch(asClient(client), 'u', ['Linear', 'Gmail'])
    expect(client.callCount).toBe(1)
  })

  it('dedupes duplicate server names within one call', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'u', ['Gmail', 'Gmail'])
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    expect(client.callCount).toBe(1)
  })

  it('different user gets a separate cache entry', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'userA', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'userB', ['Gmail'])
    expect(client.callCount).toBe(2)
  })

  it('different server set (same user) gets a separate cache entry', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'u', ['Gmail', 'Linear'])
    expect(client.callCount).toBe(2)
  })

  it('concurrent misses share a single in-flight Promise', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    client.delayMs = 30
    const [a, b, c] = await Promise.all([
      cache.getOrFetch(asClient(client), 'u', ['Gmail']),
      cache.getOrFetch(asClient(client), 'u', ['Gmail']),
      cache.getOrFetch(asClient(client), 'u', ['Gmail']),
    ])
    expect(client.callCount).toBe(1)
    expect(a.strataId).toBe(b.strataId)
    expect(b.strataId).toBe(c.strataId)
  })

  it('TTL expiry triggers a fresh fetch', async () => {
    const cache = new KlavisStrataCache(10) // 10 ms TTL
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    await new Promise((r) => setTimeout(r, 25))
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    expect(client.callCount).toBe(2)
  })

  it('invalidate(userA) drops only userA entries', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'userA', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'userB', ['Gmail'])
    cache.invalidate('userA')
    await cache.getOrFetch(asClient(client), 'userA', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'userB', ['Gmail'])
    expect(client.callCount).toBe(3) // userA: cold + cold, userB: cold + hit
  })

  it('invalidate while a fetch is in flight does not store the result', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    client.delayMs = 30
    const inflight = cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    cache.invalidate('u')
    const result = await inflight
    expect(result.strataId).toBe('strata_u')
    // Next call should not see the post-invalidate write — must re-fetch.
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    expect(client.callCount).toBe(2)
  })

  it('rejected fetches do not poison the cache', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    client.shouldThrowOnce = true
    await expect(
      cache.getOrFetch(asClient(client), 'u', ['Gmail']),
    ).rejects.toThrow('boom')
    await cache.getOrFetch(asClient(client), 'u', ['Gmail'])
    expect(client.callCount).toBe(2)
  })

  it('clear() drops all entries', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    await cache.getOrFetch(asClient(client), 'userA', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'userB', ['Linear'])
    cache.clear()
    await cache.getOrFetch(asClient(client), 'userA', ['Gmail'])
    await cache.getOrFetch(asClient(client), 'userB', ['Linear'])
    expect(client.callCount).toBe(4)
  })

  it('passes a defensive copy of the servers array to the client', async () => {
    const cache = new KlavisStrataCache()
    const client = new StubKlavisClient()
    const input: readonly string[] = ['Gmail', 'Linear']
    await cache.getOrFetch(asClient(client), 'u', input)
    expect(client.lastServers).not.toBe(input)
    expect(client.lastServers).toEqual(['Gmail', 'Linear'])
  })
})
