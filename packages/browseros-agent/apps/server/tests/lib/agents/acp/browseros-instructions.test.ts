/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  acpWorkspaceDir,
  BROWSEROS_ACP_INSTRUCTIONS,
  ensureAcpWorkspace,
} from '../../../../src/lib/agents/acp/browseros-instructions'

const dirs: string[] = []

afterEach(async () => {
  await Promise.all(
    dirs.splice(0).map((d) => rm(d, { recursive: true, force: true })),
  )
})

describe('acpWorkspaceDir', () => {
  it('is a stable path under the BrowserOS dir', () => {
    expect(acpWorkspaceDir('/home/x/.browseros')).toBe(
      '/home/x/.browseros/agents/acp-workspace',
    )
  })
})

describe('ensureAcpWorkspace', () => {
  it('creates the workspace and writes CLAUDE.md and AGENTS.md with the instructions', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'acp-instructions-'))
    dirs.push(browserosDir)

    const workspace = await ensureAcpWorkspace(browserosDir)
    expect(workspace).toBe(acpWorkspaceDir(browserosDir))

    const claude = await readFile(join(workspace, 'CLAUDE.md'), 'utf8')
    const agents = await readFile(join(workspace, 'AGENTS.md'), 'utf8')
    expect(claude).toBe(BROWSEROS_ACP_INSTRUCTIONS)
    expect(agents).toBe(BROWSEROS_ACP_INSTRUCTIONS)
    expect(claude).toContain('use only the MCP server named')
    expect(claude).toContain('browseros-neo')
  })

  it('is idempotent across repeated calls', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'acp-instructions-'))
    dirs.push(browserosDir)

    await ensureAcpWorkspace(browserosDir)
    const workspace = await ensureAcpWorkspace(browserosDir)
    expect(await readFile(join(workspace, 'CLAUDE.md'), 'utf8')).toBe(
      BROWSEROS_ACP_INSTRUCTIONS,
    )
  })
})
