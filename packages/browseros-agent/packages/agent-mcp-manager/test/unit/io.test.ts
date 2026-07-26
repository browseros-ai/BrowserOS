import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { applyPlan, readState } from '../../src/io/index'

let workspaceDir: string

beforeEach(async () => {
  workspaceDir = await mkdtemp(join(tmpdir(), 'acpx-io-'))
})

afterEach(async () => {
  await rm(workspaceDir, { recursive: true, force: true })
})

describe('readState', () => {
  test('empty workspace yields an empty manifest and no agent files', async () => {
    const state = await readState(workspaceDir)
    expect(state.manifest.servers).toEqual({})
    expect(state.agents).toEqual([])
    expect(state.manifestPath.endsWith('manifest.json')).toBe(true)
  })

  test('reads manifest when it exists', async () => {
    const manifestPath = join(workspaceDir, 'manifest.json')
    const manifest = {
      version: 1,
      servers: {
        gh: {
          name: 'gh',
          spec: { transport: 'stdio' as const, command: 'gh-mcp' },
          addedAt: '2026-07-06',
          links: {},
        },
      },
    }
    await Bun.write(manifestPath, JSON.stringify(manifest))
    const state = await readState(workspaceDir)
    expect(state.manifest.servers.gh?.name).toBe('gh')
  })

  test('reads per-agent config files when agents are requested with an override', async () => {
    const cursorPath = join(workspaceDir, 'cursor.json')
    await Bun.write(cursorPath, JSON.stringify({ mcpServers: {} }))
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: { cursor: cursorPath },
    })
    expect(state.agents).toHaveLength(1)
    expect(state.agents[0]?.agent).toBe('cursor')
    expect(state.agents[0]?.rawContent).toContain('mcpServers')
    expect(state.agents[0]?.exists).toBe(true)
  })

  test('missing agent config file surfaces as exists: false with empty content', async () => {
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: { cursor: join(workspaceDir, 'nope.json') },
    })
    expect(state.agents[0]?.exists).toBe(false)
    expect(state.agents[0]?.rawContent).toBe('')
  })

  test('empty-but-existing config file surfaces as exists: true', async () => {
    // Regression: `exists` used to be `rawContent.length > 0`, which
    // misclassified an empty file as missing. planRescan then reported
    // spurious "missing" drift for editors that touch a config file
    // empty before writing content.
    const emptyFile = join(workspaceDir, 'empty.json')
    await Bun.write(emptyFile, '')
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: { cursor: emptyFile },
    })
    expect(state.agents[0]?.exists).toBe(true)
    expect(state.agents[0]?.rawContent).toBe('')
  })

  test('parentExists: true when the config file exists', async () => {
    const cursorPath = join(workspaceDir, 'cursor.json')
    await Bun.write(cursorPath, JSON.stringify({ mcpServers: {} }))
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: { cursor: cursorPath },
    })
    expect(state.agents[0]?.exists).toBe(true)
    expect(state.agents[0]?.parentExists).toBe(true)
  })

  test('parentExists: true when only the parent directory exists (file missing)', async () => {
    // The workspaceDir (mkdtemp'd) exists; the file inside it does not.
    // This mirrors a freshly-launched-but-not-configured-yet agent.
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: { cursor: join(workspaceDir, 'not-created-yet.json') },
    })
    expect(state.agents[0]?.exists).toBe(false)
    expect(state.agents[0]?.parentExists).toBe(true)
  })

  test('parentExists: false when neither the file nor its parent exists', async () => {
    const state = await readState(workspaceDir, ['cursor'], {
      overrides: {
        cursor: join(workspaceDir, 'never-created-dir', 'child.json'),
      },
    })
    expect(state.agents[0]?.exists).toBe(false)
    expect(state.agents[0]?.parentExists).toBe(false)
  })
})

describe('applyPlan', () => {
  test('runs writeFile ops with atomic rename', async () => {
    const target = join(workspaceDir, 'nested', 'cursor.json')
    const result = await applyPlan({
      ops: [
        {
          kind: 'writeFile',
          path: target,
          content: '{"mcpServers":{}}',
          ensureDir: true,
        },
      ],
      nextManifest: { version: 1, servers: {} },
    })
    expect(result.writtenPaths).toEqual([target])
    const raw = await readFile(target, 'utf8')
    expect(raw).toBe('{"mcpServers":{}}')
  })

  test('applies multiple writeFile ops in the given order', async () => {
    const a = join(workspaceDir, 'a.json')
    const b = join(workspaceDir, 'b.json')
    await applyPlan({
      ops: [
        { kind: 'writeFile', path: a, content: 'aa' },
        { kind: 'writeFile', path: b, content: 'bb' },
      ],
      nextManifest: { version: 1, servers: {} },
    })
    expect(await readFile(a, 'utf8')).toBe('aa')
    expect(await readFile(b, 'utf8')).toBe('bb')
  })

  test('runs removeFile ops after writeFile ops', async () => {
    const target = join(workspaceDir, 'gone.json')
    await Bun.write(target, 'original')
    await applyPlan({
      ops: [{ kind: 'removeFile', path: target }],
      nextManifest: { version: 1, servers: {} },
    })
    await expect(stat(target)).rejects.toThrow()
  })

  test('removeFile is idempotent when the target does not exist', async () => {
    const missing = join(workspaceDir, 'never-existed.json')
    // Should not throw.
    await applyPlan({
      ops: [{ kind: 'removeFile', path: missing }],
      nextManifest: { version: 1, servers: {} },
    })
  })

  test('empty plan is a no-op', async () => {
    const res = await applyPlan({
      ops: [],
      nextManifest: { version: 1, servers: {} },
    })
    expect(res.writtenPaths).toEqual([])
    expect(res.removedPaths).toEqual([])
  })
})
