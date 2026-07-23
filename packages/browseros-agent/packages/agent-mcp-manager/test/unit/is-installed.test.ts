import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { isInstalled } from '../../src/api'

// isInstalled resolves per-agent config paths via
// resolveAgentMcpConfigPath. To keep the tests hermetic we point the
// test at a controlled path via a symlink-style trick: set env vars the
// catalog references so the resolved path lands under our tmp dir.
// The simplest approach here is to set $HOME to workspaceDir and rely
// on the fact that most catalog entries use $HOME/... on unix. For
// win32 test coverage see the CI matrix.

let workspaceDir: string
let originalHome: string | undefined
let originalUserProfile: string | undefined

beforeEach(async () => {
  workspaceDir = await mkdtemp(join(tmpdir(), 'acpx-installed-'))
  originalHome = process.env.HOME
  originalUserProfile = process.env.USERPROFILE
  process.env.HOME = workspaceDir
  // Redirect the win32 home token too so the catalog's win32
  // installCheckPaths / systemPaths (all `$USERPROFILE`-based) stay
  // hermetic under the tmp dir. Without this, tests running on a
  // win32 runner would silently probe the developer's real home.
  process.env.USERPROFILE = workspaceDir
})

afterEach(async () => {
  if (originalHome === undefined) delete process.env.HOME
  else process.env.HOME = originalHome
  if (originalUserProfile === undefined) delete process.env.USERPROFILE
  else process.env.USERPROFILE = originalUserProfile
  await rm(workspaceDir, { recursive: true, force: true })
})

describe('isInstalled', () => {
  test('returns installed: true when the config file exists', async () => {
    // Cursor's system path is $HOME/.cursor/mcp.json.
    await mkdir(join(workspaceDir, '.cursor'), { recursive: true })
    await Bun.write(
      join(workspaceDir, '.cursor', 'mcp.json'),
      JSON.stringify({ mcpServers: {} }),
    )
    const result = await isInstalled({ agents: ['cursor'] })
    expect(result.cursor).toBe(true)
  })

  test('returns installed: true when only the parent directory exists', async () => {
    // Fresh Cursor: ~/.cursor/ exists, mcp.json does not.
    await mkdir(join(workspaceDir, '.cursor'), { recursive: true })
    const result = await isInstalled({ agents: ['cursor'] })
    expect(result.cursor).toBe(true)
  })

  test('returns installed: false when neither exists', async () => {
    // Nothing under $HOME/.codex at all: agent not installed. Uses
    // codex because every one of its installCheckPaths lives under
    // $HOME, so redirecting HOME to a fresh tmp dir fully isolates
    // the probe from whatever real agents live on the runner.
    const result = await isInstalled({ agents: ['codex'] })
    expect(result.codex).toBe(false)
  })

  test('returns only the requested agents (no extra keys)', async () => {
    await mkdir(join(workspaceDir, '.cursor'), { recursive: true })
    const result = await isInstalled({ agents: ['cursor'] })
    expect(Object.keys(result)).toEqual(['cursor'])
  })

  test('handles duplicate agent ids in input (deduplicates)', async () => {
    await mkdir(join(workspaceDir, '.cursor'), { recursive: true })
    const result = await isInstalled({
      agents: ['cursor', 'cursor', 'cursor'],
    })
    expect(Object.keys(result)).toEqual(['cursor'])
    expect(result.cursor).toBe(true)
  })

  test('handles an empty agents list', async () => {
    const result = await isInstalled({ agents: [] })
    expect(result).toEqual({})
  })

  test('reports false when the OS cannot resolve any path candidate', async () => {
    // Clearing $HOME makes all $HOME/... candidates unresolvable on
    // this OS. Both installCheckPaths and systemPaths for codex
    // reference $HOME, so `resolveInstallCheckPaths` returns an
    // empty list and `resolveAgentMcpConfigPath` throws
    // UnresolvedConfigPathError, which isInstalled catches.
    delete process.env.HOME
    const result = await isInstalled({ agents: ['codex'] })
    expect(result.codex).toBe(false)
  })

  test('respects scope: project with projectRoot', async () => {
    // Cursor's project file is `.cursor/mcp.json` relative to project
    // root. Point projectRoot at workspaceDir; nothing exists yet.
    const before = await isInstalled({
      agents: ['cursor'],
      scope: 'project',
      projectRoot: workspaceDir,
    })
    expect(before.cursor).toBe(false)

    // Create the parent dir and check again.
    await mkdir(join(workspaceDir, '.cursor'), { recursive: true })
    const after = await isInstalled({
      agents: ['cursor'],
      scope: 'project',
      projectRoot: workspaceDir,
    })
    expect(after.cursor).toBe(true)
  })

  test('returns a plain object with only string keys (safe to iterate)', async () => {
    // Guard against future accidental use of Object.create(null) or a
    // Map. Consumers rely on `Object.keys`, `Object.entries`, and the
    // `in` operator.
    const result = await isInstalled({ agents: ['cursor'] })
    expect(Object.getPrototypeOf(result)).toBe(Object.prototype)
  })

  test('returns true when only an installCheckPaths entry exists (systemPath absent)', async () => {
    // Regression coverage for BrowserOS issue #1861: OpenCode's global
    // `opencode.json` is USER-CREATED. A fresh install has no
    // systemPath file or parent yet, but the catalog's
    // installCheckPaths point at OpenCode's data dirs. The probe must
    // recognize this state as "installed", or the /mcp page hides the
    // row post-Disconnect.
    await mkdir(join(workspaceDir, '.local', 'share', 'opencode'), {
      recursive: true,
    })
    const result = await isInstalled({ agents: ['opencode'] })
    expect(result.opencode).toBe(true)
  })

  test('returns true when the legacy installCheckPaths entry exists', async () => {
    // Older OpenCode installs live under ~/.opencode/. That directory
    // alone still counts as installed even without any of the
    // XDG-style systemPath files.
    await mkdir(join(workspaceDir, '.opencode'), { recursive: true })
    const result = await isInstalled({ agents: ['opencode'] })
    expect(result.opencode).toBe(true)
  })
})
