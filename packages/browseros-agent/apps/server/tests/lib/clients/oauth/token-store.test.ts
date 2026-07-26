/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { OAuthTokenStore } from '../../../../src/lib/clients/oauth/token-store'
import { closeDb, initializeDb } from '../../../../src/lib/db'

describe('OAuthTokenStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('stores, updates, reads, reports status, and deletes provider tokens', () => {
    const store = createStore()

    store.upsertTokens('browseros-1', 'github-copilot', {
      accessToken: 'access-1',
      refreshToken: 'refresh-1',
      expiresAt: 1234,
      email: 'user@example.com',
      accountId: 'account-1',
    })

    expect(store.getTokens('browseros-1', 'github-copilot')).toEqual({
      accessToken: 'access-1',
      refreshToken: 'refresh-1',
      expiresAt: 1234,
      email: 'user@example.com',
      accountId: 'account-1',
    })
    expect(store.getStatus('browseros-1', 'github-copilot')).toEqual({
      authenticated: true,
      email: 'user@example.com',
      provider: 'github-copilot',
    })

    store.upsertTokens('browseros-1', 'github-copilot', {
      accessToken: 'access-2',
      refreshToken: '',
      expiresAt: 0,
    })

    expect(store.getTokens('browseros-1', 'github-copilot')).toEqual({
      accessToken: 'access-2',
      refreshToken: '',
      expiresAt: 0,
      email: undefined,
      accountId: undefined,
    })

    store.deleteTokens('browseros-1', 'github-copilot')

    expect(store.getTokens('browseros-1', 'github-copilot')).toBeNull()
    expect(store.getStatus('browseros-1', 'github-copilot')).toEqual({
      authenticated: false,
      email: undefined,
      provider: 'github-copilot',
    })
  })

  function createStore(): OAuthTokenStore {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-oauth-store-test-'))
    tempDirs.push(dir)
    const handle = initializeDb({
      dbPath: join(dir, 'db', 'browseros.sqlite'),
    })
    return new OAuthTokenStore(handle.db)
  }
})
