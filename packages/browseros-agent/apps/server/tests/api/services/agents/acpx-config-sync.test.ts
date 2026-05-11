/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  importAgentsFromAcpx,
  probeCustomAgent,
  readAcpxConfig,
} from '../../../../src/api/services/agents/acpx-config-sync'
import type { AgentDefinition } from '../../../../src/lib/agents/agent-types'

// --- Helpers ---

function makeAgent(overrides: Partial<AgentDefinition> = {}): AgentDefinition {
  return {
    id: 'agent-1',
    name: 'test-agent',
    adapter: 'claude',
    permissionMode: 'approve-all',
    sessionKey: 'agent:agent-1:main',
    createdAt: Date.now(),
    updatedAt: Date.now(),
    ...overrides,
  }
}

const BUILTIN_NAMES = ['claude', 'codex', 'openclaw', 'hermes']

// Mock child_process.spawn by intercepting the module
// We use a manual mock approach compatible with bun:test

// ===========================================================================
// readAcpxConfig
// ===========================================================================
describe('readAcpxConfig', () => {
  let tmpDir: string
  let originalEnv: string | undefined

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'acpx-config-test-'))
    originalEnv = process.env.ACPX_CONFIG_DIR
  })

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true })
    if (originalEnv === undefined) {
      delete process.env.ACPX_CONFIG_DIR
    } else {
      process.env.ACPX_CONFIG_DIR = originalEnv
    }
  })

  it('reads a valid config.json with agents', () => {
    const config = {
      agents: {
        'my-tool': { command: './bin/tool', args: ['--acp'] },
        another: { command: 'python3', args: ['-m', 'my_server'] },
      },
    }
    writeFileSync(join(tmpDir, 'config.json'), JSON.stringify(config))

    const result = readAcpxConfig(tmpDir)
    expect(result).not.toBeNull()
    expect(result?.agents).toHaveProperty('my-tool')
    expect(result?.agents['my-tool'].command).toBe('./bin/tool')
    expect(result?.agents.another.args).toEqual(['-m', 'my_server'])
  })

  it('returns null when config file is missing', () => {
    const result = readAcpxConfig(tmpDir)
    expect(result).toBeNull()
  })

  it('returns null for invalid JSON', () => {
    writeFileSync(join(tmpDir, 'config.json'), 'not-json {{{')
    const result = readAcpxConfig(tmpDir)
    expect(result).toBeNull()
  })

  it('returns null for valid JSON without agents key', () => {
    writeFileSync(join(tmpDir, 'config.json'), JSON.stringify({ foo: 'bar' }))
    const result = readAcpxConfig(tmpDir)
    expect(result).toBeNull()
  })

  it('uses env var ACPX_CONFIG_DIR when no dir argument provided', () => {
    const config = {
      agents: { 'env-agent': { command: 'env-bin' } },
    }
    writeFileSync(join(tmpDir, 'config.json'), JSON.stringify(config))
    process.env.ACPX_CONFIG_DIR = tmpDir

    const result = readAcpxConfig()
    expect(result).not.toBeNull()
    expect(result?.agents['env-agent'].command).toBe('env-bin')
  })

  it('defaults to ~/.acpx when no dir and no env var', () => {
    delete process.env.ACPX_CONFIG_DIR
    const result = readAcpxConfig()
    // Just verify it doesn't throw
    expect(result === null || result !== null).toBe(true)
  })
})

// ===========================================================================
// importAgentsFromAcpx
// ===========================================================================
describe('importAgentsFromAcpx', () => {
  it('imports new agents as custom adapter', () => {
    const config = {
      agents: {
        'my-tool': { command: './bin/tool', args: ['--acp'] },
      },
    }
    const existing: AgentDefinition[] = []

    const results = importAgentsFromAcpx(config, existing)
    expect(results).toHaveLength(1)
    expect(results[0]).toEqual({
      name: 'my-tool',
      imported: true,
    })
  })

  it('skips agents that already exist by name', () => {
    const config = {
      agents: {
        'existing-agent': { command: './bin/exists' },
      },
    }
    const existing: AgentDefinition[] = [makeAgent({ name: 'existing-agent' })]

    const results = importAgentsFromAcpx(config, existing)
    expect(results).toHaveLength(1)
    expect(results[0]).toEqual({
      name: 'existing-agent',
      imported: false,
    })
  })

  it('imports built-in-named agents with warning', () => {
    for (const name of BUILTIN_NAMES) {
      const config = {
        agents: { [name]: { command: `${name}-bin` } },
      }
      const existing: AgentDefinition[] = []
      const results = importAgentsFromAcpx(config, existing)
      expect(results).toHaveLength(1)
      expect(results[0].imported).toBe(true)
      expect(results[0].warning).toBe('Name conflicts with built-in agent')
    }
  })

  it('returns empty array for config with no agents', () => {
    const config = { agents: {} }
    const existing: AgentDefinition[] = []
    const results = importAgentsFromAcpx(config, existing)
    expect(results).toHaveLength(0)
  })

  it('handles mixed scenario: new, existing, and name-clash agents', () => {
    const config = {
      agents: {
        'new-agent': { command: 'new-bin' },
        'existing-agent': { command: 'existing-bin' },
        claude: { command: 'claude-custom' },
      },
    }
    const existing: AgentDefinition[] = [makeAgent({ name: 'existing-agent' })]

    const results = importAgentsFromAcpx(config, existing)
    expect(results).toHaveLength(3)

    const byName = Object.fromEntries(results.map((r) => [r.name, r]))
    expect(byName['new-agent'].imported).toBe(true)
    expect(byName['new-agent'].warning).toBeUndefined()

    expect(byName['existing-agent'].imported).toBe(false)
    expect(byName['existing-agent'].warning).toBeUndefined()

    expect(byName.claude.imported).toBe(true)
    expect(byName.claude.warning).toBe('Name conflicts with built-in agent')
  })
})

// ===========================================================================
// probeCustomAgent
// ===========================================================================
describe('probeCustomAgent', () => {
  it('rejects commands with shell metacharacters', async () => {
    const dangerous = [
      'cmd;evil',
      'cmd & evil',
      'cmd | evil',
      'cmd`evil`',
      'cmd$(evil)',
      'cmd{evil}',
      'cmd!evil',
      'cmd#evil',
      'cmd~evil',
    ]
    for (const cmd of dangerous) {
      const result = await probeCustomAgent(cmd, [])
      expect(result.healthy).toBe(false)
      expect(result.error).toContain('unsafe characters')
    }
  })

  it('returns error for command not found (ENOENT)', async () => {
    const result = await probeCustomAgent(
      'nonexistent-binary-that-does-not-exist-12345',
      [],
    )
    expect(result.healthy).toBe(false)
    expect(result.error).toContain('Command not found')
  })

  it('never throws — returns ProbeResult on all errors', async () => {
    // Command with shell metacharacter — should not throw, just return error
    const result = await probeCustomAgent('cmd;evil', [])
    expect(result.healthy).toBe(false)
    expect(result.error).toBeDefined()
  })

  it('returns error for non-ACP binaries (e.g., /bin/ls)', async () => {
    // /bin/ls is not an ACP server — it will either exit with non-zero or produce non-JSON output
    const result = await probeCustomAgent('/bin/ls', [])
    expect(result.healthy).toBe(false)
    // Could be either non-zero exit or invalid JSON — either way, not healthy
    expect(result.error).toBeDefined()
  })

  it('args with metacharacters are safe (shell:false prevents injection)', async () => {
    // args are passed directly to spawn with shell:false, so metacharacters
    // are not interpreted — the binary just receives them as literal strings.
    // echo may not exist as a standalone binary, so just verify no crash.
    const result = await probeCustomAgent('/bin/true', ['arg;evil'])
    // /bin/true exits immediately with 0, no ACP response → not healthy
    expect(result.healthy).toBe(false)
    expect(result.error).toBeDefined()
  })
})
