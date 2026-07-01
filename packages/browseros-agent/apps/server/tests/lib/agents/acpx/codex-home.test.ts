/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { materializeCodexHome } from '../../../../src/lib/agents/acpx/codex-home'
import {
  ensureRuntimeSkills,
  resolveAgentRuntimePaths,
} from '../../../../src/lib/agents/acpx/runtime-context'

describe('materializeCodexHome', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('uses auth and instructions without copying Codex config files', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-context-'))
    const sourceCodexHome = await mkdtemp(
      join(tmpdir(), 'browseros-codex-src-'),
    )
    tempDirs.push(browserosDir, sourceCodexHome)
    await writeFile(join(sourceCodexHome, 'auth.json'), '{"ok":true}\n')
    await writeFile(join(sourceCodexHome, 'instructions.md'), 'be concise\n')
    await writeFile(join(sourceCodexHome, 'config.json'), '{"model":"test"}\n')
    await writeFile(
      join(sourceCodexHome, 'config.toml'),
      '[features.multi_agent_v2]\nenabled = false\n',
    )
    const paths = resolveAgentRuntimePaths({ browserosDir, agentId: 'agent-1' })
    const skills = await ensureRuntimeSkills(paths.runtimeSkillsDir)

    await materializeCodexHome({ paths, skillNames: skills, sourceCodexHome })

    const auth = await lstat(join(paths.codexHome, 'auth.json'))
    expect(auth.isSymbolicLink()).toBe(true)
    expect(
      await readFile(join(paths.codexHome, 'instructions.md'), 'utf8'),
    ).toBe('be concise\n')
    await expect(
      readFile(join(paths.codexHome, 'config.json'), 'utf8'),
    ).rejects.toThrow(/ENOENT/)
    await expect(
      readFile(join(paths.codexHome, 'config.toml'), 'utf8'),
    ).rejects.toThrow(/ENOENT/)
    expect(
      await readFile(
        join(paths.codexHome, 'skills', 'browseros', 'SKILL.md'),
        'utf8',
      ),
    ).toContain('BrowserOS MCP')
  })

  it('rejects non-file Codex auth sources instead of silently skipping auth', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-context-'))
    const sourceCodexHome = await mkdtemp(
      join(tmpdir(), 'browseros-codex-src-'),
    )
    tempDirs.push(browserosDir, sourceCodexHome)
    await mkdir(join(sourceCodexHome, 'auth.json'))
    const paths = resolveAgentRuntimePaths({ browserosDir, agentId: 'agent-1' })
    const skills = await ensureRuntimeSkills(paths.runtimeSkillsDir)

    await expect(
      materializeCodexHome({ paths, skillNames: skills, sourceCodexHome }),
    ).rejects.toThrow(/auth\.json/)
  })

  it('ignores non-file Codex config sources', async () => {
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-context-'))
    const sourceCodexHome = await mkdtemp(
      join(tmpdir(), 'browseros-codex-src-'),
    )
    tempDirs.push(browserosDir, sourceCodexHome)
    await mkdir(join(sourceCodexHome, 'config.toml'))
    const paths = resolveAgentRuntimePaths({ browserosDir, agentId: 'agent-1' })
    const skills = await ensureRuntimeSkills(paths.runtimeSkillsDir)

    await materializeCodexHome({ paths, skillNames: skills, sourceCodexHome })

    await expect(
      readFile(join(paths.codexHome, 'config.toml'), 'utf8'),
    ).rejects.toThrow(/ENOENT/)
  })
})
