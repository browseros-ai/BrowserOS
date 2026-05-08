/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  ClaudeRuntime,
  configureClaudeRuntime,
  getAgentRuntimeRegistry,
  getClaudeRuntime,
  prepareClaudeCodeContext,
  resetAgentRuntimeRegistry,
} from '../../../../src/lib/agents/runtime'

function makeAgent(id = 'agent-1') {
  return {
    id,
    name: 'Claude bot',
    adapter: 'claude' as const,
    sessionKey: `agent:${id}:main`,
    pinned: false,
    updatedAt: Date.now(),
    createdAt: Date.now(),
    modelId: 'claude-opus-4-5',
    reasoningEffort: 'medium',
    providerType: 'host-auth',
    providerName: null,
    baseUrl: null,
    apiKey: null,
    supportsImages: true,
  }
}

describe('ClaudeRuntime', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
    resetAgentRuntimeRegistry()
  })

  it('declares the canonical Claude descriptor', () => {
    const runtime = new ClaudeRuntime(
      { binaryName: 'claude' },
      { browserosDir: '/tmp/browseros' },
    )
    expect(runtime.descriptor.adapterId).toBe('claude')
    expect(runtime.descriptor.kind).toBe('host-process')
    expect(runtime.descriptor.platforms).toContain('darwin')
    expect(runtime.descriptor.platforms).toContain('linux')
  })

  it('getPerAgentHomeDir resolves the canonical agent home path', () => {
    const runtime = new ClaudeRuntime(
      { binaryName: 'claude' },
      { browserosDir: '/tmp/browseros' },
    )
    expect(runtime.getPerAgentHomeDir('agent-7')).toBe(
      '/tmp/browseros/agents/harness/agent-7/home',
    )
  })

  it('prepareTurnContext sets AGENT_HOME and not CODEX_HOME', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-claude-'))
    tempDirs.push(browserosDir)
    const prepared = await prepareClaudeCodeContext({
      browserosDir,
      agent: makeAgent('claude-agent'),
      sessionId: 'main',
      sessionKey: 'agent:claude-agent:main',
      cwdOverride: null,
      isSelectedCwd: false,
      message: 'hi',
    })
    expect(prepared.commandEnv).toEqual({
      AGENT_HOME: join(
        browserosDir,
        'agents',
        'harness',
        'claude-agent',
        'home',
      ),
    })
    expect(prepared.commandEnv).not.toHaveProperty('CODEX_HOME')
    expect(prepared.useBrowserosMcp).toBe(true)
  })

  describe('configureClaudeRuntime', () => {
    it('registers a runtime in the registry', () => {
      const browserosDir = '/tmp/browseros'
      const runtime = configureClaudeRuntime({ browserosDir })
      expect(runtime).toBeInstanceOf(ClaudeRuntime)
      expect(getClaudeRuntime()).toBe(runtime)
      expect(getAgentRuntimeRegistry().get('claude')).toBe(runtime)
    })

    it('throws on duplicate registration', () => {
      configureClaudeRuntime({ browserosDir: '/tmp/browseros' })
      expect(() =>
        configureClaudeRuntime({ browserosDir: '/tmp/browseros' }),
      ).toThrow(/already registered/)
    })
  })
})
