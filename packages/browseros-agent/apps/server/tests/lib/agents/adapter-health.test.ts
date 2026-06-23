/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { AdapterHealthChecker } from '../../../src/lib/agents/adapters/health'

describe('AdapterHealthChecker', () => {
  it('reports Claude through host adapter detection', async () => {
    const health = await new AdapterHealthChecker({
      detectHostAdapter: async (adapter) => {
        expect(adapter).toBe('claude')
        return {
          healthy: true,
          checkedAt: 1234,
          readiness: 'ready',
          installState: 'installed',
          nativeCliState: 'present',
          authState: 'authenticated',
          version: 'claude 1.0.0',
          adapterLaunchSource: 'host-npx',
          packageCacheState: 'unknown',
        }
      },
    }).getHealth('claude')

    expect(health).toMatchObject({
      healthy: true,
      checkedAt: 1234,
      readiness: 'ready',
      installState: 'installed',
      authState: 'authenticated',
      adapterLaunchSource: 'host-npx',
    })
  })
})
