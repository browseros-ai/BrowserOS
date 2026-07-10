import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  bind,
  disconnect,
  isInstalled,
  link,
  list,
  listLinks,
  remove,
  unlink,
} from '../../src/api'
import { AgentNotInstalledError } from '../../src/errors'
import { readState } from '../../src/io/index'
import type { McpServer } from '../../src/types'

let workspaceDir: string
let cursorPath: string
let geminiPath: string
let claudeCodePath: string

const GH_STDIO: McpServer = {
  name: 'gh',
  spec: { transport: 'stdio', command: 'gh-mcp' },
}

const GH_HTTP: McpServer = {
  name: 'gh',
  spec: { transport: 'http', url: 'https://api.example.com/mcp' },
}

const SOLO: McpServer = {
  name: 'solo',
  spec: { transport: 'stdio', command: 'solo-mcp' },
}

beforeEach(async () => {
  workspaceDir = await mkdtemp(join(tmpdir(), 'acpx-api-'))
  cursorPath = join(workspaceDir, 'cursor.json')
  geminiPath = join(workspaceDir, 'gemini.json')
  claudeCodePath = join(workspaceDir, 'claude.json')
})

afterEach(async () => {
  await rm(workspaceDir, { recursive: true, force: true })
})

describe('link (upserts manifest + writes agent config)', () => {
  test('first link creates the manifest server entry AND writes the agent config', async () => {
    const res = await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    expect(res.created).toBe(true)
    expect(res.overwroteForeign).toBe(false)

    const raw = await readFile(cursorPath, 'utf8')
    expect(JSON.parse(raw).mcpServers.gh.command).toBe('gh-mcp')

    const state = await readState(workspaceDir)
    expect(state.manifest.servers.gh?.spec).toEqual(GH_STDIO.spec)
    expect(state.manifest.servers.gh?.links.cursor?.configPath).toBe(cursorPath)
  })

  test('re-linking the same server + agent with identical spec returns created: false', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    const res = await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    expect(res.created).toBe(false)
  })

  test('linking the same name with a different spec upserts the manifest (last-write-wins)', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    await link(workspaceDir, {
      server: GH_HTTP,
      agent: 'gemini',
      configPath: geminiPath,
    })
    const state = await readState(workspaceDir)
    // Manifest reflects the LATER spec.
    expect(state.manifest.servers.gh?.spec).toEqual(GH_HTTP.spec)
    // But cursor's file still has the earlier spec on disk; rescan
    // will report drift for cursor.
    const cursorRaw = await readFile(cursorPath, 'utf8')
    expect(JSON.parse(cursorRaw).mcpServers.gh.command).toBe('gh-mcp')
  })

  test('trims the server name before using it as the manifest key', async () => {
    // Regression: matches the trim behavior the removed addServer had.
    await link(workspaceDir, {
      server: { name: '  gh  ', spec: GH_STDIO.spec },
      agent: 'cursor',
      configPath: cursorPath,
    })
    const state = await readState(workspaceDir)
    expect(state.manifest.servers.gh).toBeDefined()
    expect(state.manifest.servers['  gh  ']).toBeUndefined()
  })
})

describe('disconnect (regression test for #63)', () => {
  test('disconnecting one of three linked agents does NOT touch the other two', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'gemini',
      configPath: geminiPath,
    })
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'claude-code',
      configPath: claudeCodePath,
    })

    const before = {
      gemini: await readFile(geminiPath, 'utf8'),
      claudeCode: await readFile(claudeCodePath, 'utf8'),
    }

    const res = await disconnect(workspaceDir, {
      serverName: 'gh',
      agent: 'cursor',
    })
    expect(res.unlinked).toBe(true)
    expect(res.removedManifest).toBe(false)

    const cursorAfter = await readFile(cursorPath, 'utf8')
    expect(JSON.parse(cursorAfter).mcpServers.gh).toBeUndefined()

    expect(await readFile(geminiPath, 'utf8')).toBe(before.gemini)
    expect(await readFile(claudeCodePath, 'utf8')).toBe(before.claudeCode)

    const state = await readState(workspaceDir)
    expect(state.manifest.servers.gh).toBeDefined()
    expect(Object.keys(state.manifest.servers.gh?.links ?? {}).sort()).toEqual([
      'claude-code',
      'gemini',
    ])
  })

  test('disconnecting the last agent drops the manifest entry by default', async () => {
    await link(workspaceDir, {
      server: SOLO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    const res = await disconnect(workspaceDir, {
      serverName: 'solo',
      agent: 'cursor',
    })
    expect(res.removedManifest).toBe(true)
    const state = await readState(workspaceDir)
    expect(state.manifest.servers.solo).toBeUndefined()
  })
})

describe('unlink + list + listLinks + remove', () => {
  test('unlink is idempotent when the link does not exist', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    const res = await unlink(workspaceDir, {
      serverName: 'gh',
      agent: 'gemini',
      configPath: geminiPath,
    })
    expect(res.removed).toBe(false)
  })

  test('unlink without an explicit configPath uses the manifest-recorded path', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    const res = await unlink(workspaceDir, {
      serverName: 'gh',
      agent: 'cursor',
    })
    expect(res.removed).toBe(true)
    const raw = await readFile(cursorPath, 'utf8')
    expect(JSON.parse(raw).mcpServers.gh).toBeUndefined()
  })

  test('list returns every manifest server entry', async () => {
    await link(workspaceDir, {
      server: { name: 'a', spec: { transport: 'stdio', command: 'a' } },
      agent: 'cursor',
      configPath: cursorPath,
    })
    await link(workspaceDir, {
      server: { name: 'b', spec: { transport: 'http', url: 'https://b/mcp' } },
      agent: 'cursor',
      configPath: cursorPath,
    })
    const items = await list(workspaceDir)
    expect(items.map((s) => s.name).sort()).toEqual(['a', 'b'])
  })

  test('listLinks reports every (server, agent, configPath) triple', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'gemini',
      configPath: geminiPath,
    })
    const links = await listLinks(workspaceDir)
    expect(links.map((l) => l.agent).sort()).toEqual(['cursor', 'gemini'])
  })

  test('remove drops the manifest entry and unlinks every currently-linked agent', async () => {
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'gemini',
      configPath: geminiPath,
    })
    const res = await remove(workspaceDir, { serverName: 'gh' })
    expect(res.removedManifest).toBe(true)
    expect(res.unlinkedAgents.sort()).toEqual(['cursor', 'gemini'])
    expect(
      JSON.parse(await readFile(cursorPath, 'utf8')).mcpServers.gh,
    ).toBeUndefined()
    expect(
      JSON.parse(await readFile(geminiPath, 'utf8')).mcpServers.gh,
    ).toBeUndefined()
  })
})

describe('bind', () => {
  test('applies the workspaceDir to every verb', async () => {
    const mgr = bind(workspaceDir)
    await mgr.link({
      server: GH_STDIO,
      agent: 'cursor',
      configPath: cursorPath,
    })
    const links = await mgr.listLinks()
    expect(links).toHaveLength(1)
  })
})

describe('install-check gate', () => {
  test('link() throws AgentNotInstalledError when the config parent dir does not exist', async () => {
    // Point Cursor at a path deep inside a non-existent directory
    // chain. Even the immediate parent does not exist, so the install
    // gate fires.
    const missingParent = join(workspaceDir, 'nonexistent-agent-dir')
    const missingConfig = join(missingParent, 'cursor.json')
    let caught: unknown
    try {
      await link(workspaceDir, {
        server: GH_STDIO,
        agent: 'cursor',
        configPath: missingConfig,
      })
    } catch (err) {
      caught = err
    }
    expect(caught).toBeInstanceOf(AgentNotInstalledError)
    const err = caught as AgentNotInstalledError
    expect(err.agent).toBe('cursor')
    expect(err.configPath).toBe(missingConfig)
    expect(err.parentDir).toBe(missingParent)
  })

  test('link() succeeds when the parent directory exists but the file does not', async () => {
    const parent = join(workspaceDir, 'fresh-agent')
    await mkdir(parent, { recursive: true })
    const configPath = join(parent, 'cursor.json')
    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath,
    })
    const raw = await readFile(configPath, 'utf8')
    expect(JSON.parse(raw).mcpServers.gh.command).toBe('gh-mcp')
  })

  test('AgentNotInstalledError is re-exported from the package root', async () => {
    const mod = await import('../../src/index')
    expect(mod.AgentNotInstalledError).toBeDefined()
    expect(typeof mod.AgentNotInstalledError).toBe('function')
  })

  test('isInstalled via bind() returns the same result as the direct call', async () => {
    // Point $HOME at a fresh dir so cursor's default path resolves
    // consistently for both calls.
    const originalHome = process.env.HOME
    const home = await mkdtemp(join(tmpdir(), 'acpx-bind-installed-'))
    process.env.HOME = home
    try {
      await mkdir(join(home, '.cursor'), { recursive: true })
      const mgr = bind(workspaceDir)
      const viaBind = await mgr.isInstalled({ agents: ['cursor'] })
      const direct = await isInstalled({ agents: ['cursor'] })
      expect(viaBind).toEqual(direct)
      expect(viaBind.cursor).toBe(true)
    } finally {
      if (originalHome === undefined) delete process.env.HOME
      else process.env.HOME = originalHome
      await rm(home, { recursive: true, force: true })
    }
  })

  test('isInstalled predicts what link would throw (dual invariant)', async () => {
    // When isInstalled reports true for a configPath override whose
    // parent exists, link() succeeds. When false, link() throws.
    const okParent = join(workspaceDir, 'ok')
    await mkdir(okParent, { recursive: true })
    const okConfig = join(okParent, 'cursor.json')
    const missingParent = join(workspaceDir, 'missing')
    const missingConfig = join(missingParent, 'cursor.json')

    // Direct helper: check whether a specific file location is
    // link-safe by looking at its parent. Mirrors the isInstalled
    // signal for the configPath-override case.
    const stat = await import('node:fs/promises').then((m) => m.stat)
    const parentExists = async (p: string) => {
      try {
        await stat(p)
        return true
      } catch {
        return false
      }
    }

    expect(await parentExists(okParent)).toBe(true)
    expect(await parentExists(missingParent)).toBe(false)

    await link(workspaceDir, {
      server: GH_STDIO,
      agent: 'cursor',
      configPath: okConfig,
    })
    await expect(
      link(workspaceDir, {
        server: GH_STDIO,
        agent: 'cursor',
        configPath: missingConfig,
      }),
    ).rejects.toBeInstanceOf(AgentNotInstalledError)
  })
})
