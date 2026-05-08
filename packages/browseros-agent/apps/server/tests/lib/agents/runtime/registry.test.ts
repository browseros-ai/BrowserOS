/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  type AgentRuntime,
  AgentRuntimeRegistry,
  getAgentRuntimeRegistry,
  resetAgentRuntimeRegistry,
} from '../../../../src/lib/agents/runtime'

function makeFakeRuntime(adapterId: string): AgentRuntime {
  return {
    descriptor: {
      adapterId,
      displayName: adapterId,
      kind: 'host-process',
      platforms: ['darwin'],
    },
    getStatusSnapshot: () => ({
      adapterId,
      state: 'cli_missing',
      isReady: false,
      lastError: null,
      lastErrorAt: null,
    }),
    subscribe: () => () => {},
    getCapabilities: () => [],
    executeAction: async () => {},
    buildExecArgv: () => '',
    getPerAgentHomeDir: () => `/tmp/${adapterId}`,
  }
}

describe('AgentRuntimeRegistry', () => {
  it('register + get + list round-trip', () => {
    const registry = new AgentRuntimeRegistry()
    const a = makeFakeRuntime('alpha')
    const b = makeFakeRuntime('beta')

    registry.register(a)
    registry.register(b)

    expect(registry.get('alpha')).toBe(a)
    expect(registry.get('beta')).toBe(b)
    expect(registry.list()).toEqual([a, b])
  })

  it('get returns null for unknown adapterId', () => {
    const registry = new AgentRuntimeRegistry()
    expect(registry.get('nope')).toBeNull()
  })

  it('register throws on duplicate adapterId', () => {
    const registry = new AgentRuntimeRegistry()
    registry.register(makeFakeRuntime('dup'))
    expect(() => registry.register(makeFakeRuntime('dup'))).toThrow(
      /already registered/,
    )
  })

  it('unregister removes the entry and returns true', () => {
    const registry = new AgentRuntimeRegistry()
    registry.register(makeFakeRuntime('x'))
    expect(registry.unregister('x')).toBe(true)
    expect(registry.get('x')).toBeNull()
    expect(registry.unregister('x')).toBe(false)
  })

  describe('singleton', () => {
    afterEach(() => {
      resetAgentRuntimeRegistry()
    })

    it('returns the same instance across calls', () => {
      const a = getAgentRuntimeRegistry()
      const b = getAgentRuntimeRegistry()
      expect(a).toBe(b)
    })

    it('reset clears the singleton', () => {
      const a = getAgentRuntimeRegistry()
      resetAgentRuntimeRegistry()
      const b = getAgentRuntimeRegistry()
      expect(a).not.toBe(b)
    })
  })
})
