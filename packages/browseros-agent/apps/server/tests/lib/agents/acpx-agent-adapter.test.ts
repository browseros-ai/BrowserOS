/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { prepareAcpxAgentContext } from '../../../src/lib/agents/acpx-agent-adapter'
import type { AgentDefinition } from '../../../src/lib/agents/agent-types'

describe('prepareAcpxAgentContext', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  function makeAgent(adapter: AgentDefinition['adapter']): AgentDefinition {
    return {
      id: `${adapter}-agent`,
      name: `${adapter} agent`,
      adapter,
      permissionMode: 'approve-all',
      sessionKey: `agent:${adapter}-agent:main`,
      createdAt: 1000,
      updatedAt: 1000,
    }
  }

  it('prepares Claude with BrowserOS memory, host auth, BrowserOS MCP, and fingerprinted session', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-adapters-'))
    tempDirs.push(browserosDir)
    const prepared = await prepareAcpxAgentContext({
      browserosDir,
      agent: makeAgent('claude'),
      sessionId: 'main',
      sessionKey: 'agent:claude-agent:main',
      cwdOverride: null,
      isSelectedCwd: false,
      message: 'remember this',
    })

    expect(prepared.commandEnv.AGENT_HOME).toContain('/claude-agent/home')
    expect(prepared.commandEnv).not.toHaveProperty('CLAUDE_CONFIG_DIR')
    expect(prepared.commandEnv).not.toHaveProperty('CODEX_HOME')
    expect(prepared.useBrowserosMcp).toBe(true)
    expect(prepared.openclawSessionKey).toBeNull()
    expect(prepared.runtimeSessionKey).toMatch(
      /^agent:claude-agent:main:[a-f0-9]{16}$/,
    )
    expect(prepared.runPrompt).toContain(
      'Available skills: browseros, memory, soul',
    )
    expect(
      await readFile(`${prepared.commandEnv.AGENT_HOME}/MEMORY.md`, 'utf8'),
    ).toContain('# MEMORY.md')
  })

  it('prepares Codex with CODEX_HOME and BrowserOS MCP', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-adapters-'))
    tempDirs.push(browserosDir)
    const prepared = await prepareAcpxAgentContext({
      browserosDir,
      agent: makeAgent('codex'),
      sessionId: 'main',
      sessionKey: 'agent:codex-agent:main',
      cwdOverride: null,
      isSelectedCwd: false,
      message: 'hi',
    })

    expect(prepared.commandEnv.AGENT_HOME).toContain('/codex-agent/home')
    expect(prepared.commandEnv.CODEX_HOME).toContain(
      '/codex-agent/runtime/codex-home',
    )
    expect(prepared.commandEnv).not.toHaveProperty('CLAUDE_CONFIG_DIR')
    expect(prepared.useBrowserosMcp).toBe(true)
    expect(prepared.openclawSessionKey).toBeNull()
    expect(prepared.runPrompt).toContain('AGENT_HOME=')
  })

  it('prepares OpenClaw without BrowserOS memory, host cwd, skills, or MCP', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-adapters-'))
    tempDirs.push(browserosDir)
    const ignoredSelectedCwd = join(browserosDir, 'missing-selected-workspace')
    const prepared = await prepareAcpxAgentContext({
      browserosDir,
      agent: makeAgent('openclaw'),
      sessionId: 'main',
      sessionKey: 'agent:openclaw-agent:main',
      cwdOverride: ignoredSelectedCwd,
      isSelectedCwd: true,
      message: 'browse',
    })

    expect(prepared.cwd).toBe(
      join(browserosDir, 'agents', 'harness', 'workspace'),
    )
    expect(prepared.commandEnv).toEqual({})
    expect(prepared.useBrowserosMcp).toBe(false)
    expect(prepared.openclawSessionKey).toBe('agent:openclaw-agent:main')
    expect(prepared.runtimeSessionKey).toBe('agent:openclaw-agent:main')
    expect(prepared.runPrompt).not.toContain('SOUL.md stores')
    expect(prepared.runPrompt).not.toContain('BrowserOS memory skill')
    expect(prepared.runPrompt).not.toContain('AGENT_HOME/MEMORY.md')
    expect(prepared.runPrompt).not.toContain('Available skills:')
  })
})
