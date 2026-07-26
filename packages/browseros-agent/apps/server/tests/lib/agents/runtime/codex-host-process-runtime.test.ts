/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  CodexRuntime,
  configureCodexRuntime,
  getAgentRuntimeRegistry,
  getCodexRuntime,
  prepareCodexContext,
  resetAgentRuntimeRegistry,
} from '../../../../src/lib/agents/runtime'

function makeAgent(id = 'agent-1') {
  return {
    id,
    name: 'Codex bot',
    adapter: 'codex' as const,
    sessionKey: `agent:${id}:main`,
    pinned: false,
    updatedAt: Date.now(),
    createdAt: Date.now(),
    modelId: 'gpt-5.5',
    reasoningEffort: 'medium',
    providerType: 'host-auth',
    providerName: null,
    baseUrl: null,
    apiKey: null,
    supportsImages: false,
  }
}

describe('CodexRuntime', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
    resetAgentRuntimeRegistry()
  })

  it('declares the canonical Codex descriptor', () => {
    const runtime = new CodexRuntime(
      { binaryName: 'codex' },
      { browserosDir: '/tmp/browseros' },
    )
    expect(runtime.descriptor.adapterId).toBe('codex')
    expect(runtime.descriptor.kind).toBe('host-process')
  })

  it('prepareTurnContext sets AGENT_HOME + CODEX_HOME and materializes codex home', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-codex-'))
    tempDirs.push(browserosDir)
    const prepared = await prepareCodexContext({
      browserosDir,
      agent: makeAgent('codex-agent'),
      sessionId: 'main',
      sessionKey: 'agent:codex-agent:main',
      cwdOverride: null,
      isSelectedCwd: false,
      message: 'hi',
    })
    expect(prepared.commandEnv.AGENT_HOME).toBe(
      join(browserosDir, 'agents', 'harness', 'codex-agent', 'home'),
    )
    expect(prepared.commandEnv.CODEX_HOME).toBe(
      join(
        browserosDir,
        'agents',
        'harness',
        'codex-agent',
        'runtime',
        'codex-home',
      ),
    )
    const codexHomeStat = await stat(prepared.commandEnv.CODEX_HOME)
    expect(codexHomeStat.isDirectory()).toBe(true)
    expect(prepared.useBrowserosMcp).toBe(true)
  })

  describe('configureCodexRuntime', () => {
    it('registers a runtime in the registry', () => {
      const runtime = configureCodexRuntime({ browserosDir: '/tmp/browseros' })
      expect(runtime).toBeInstanceOf(CodexRuntime)
      expect(getCodexRuntime()).toBe(runtime)
      expect(getAgentRuntimeRegistry().get('codex')).toBe(runtime)
    })

    it('throws on duplicate registration', () => {
      configureCodexRuntime({ browserosDir: '/tmp/browseros' })
      expect(() =>
        configureCodexRuntime({ browserosDir: '/tmp/browseros' }),
      ).toThrow(/already registered/)
    })
  })
})
