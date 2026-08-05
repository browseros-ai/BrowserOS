/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadBrowserOsSkill } from '../../../../src/lib/agents/acp/browseros-skill'

const temporaryDirectories: string[] = []

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((path) => rm(path, { recursive: true, force: true })),
  )
})

describe('loadBrowserOsSkill', () => {
  it('loads the canonical repository skill in development', async () => {
    const skill = await loadBrowserOsSkill()

    expect(skill).toStartWith('---\nname: browserclaw\n')
    expect(skill).toContain("use BrowserOS neo's tools")
    expect(skill).toContain('prefer it over other browser surfaces')
  })

  it('prefers the packaged resource copy', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-skill-'))
    temporaryDirectories.push(resourcesDir)
    const skillDir = join(resourcesDir, 'skills', 'browserclaw')
    await mkdir(skillDir, { recursive: true })
    const packaged = [
      '---',
      'name: browserclaw',
      'description: Packaged BrowserOS skill',
      '---',
      'packaged instructions',
      '',
    ].join('\n')
    await writeFile(join(skillDir, 'SKILL.md'), packaged)

    expect(await loadBrowserOsSkill(resourcesDir)).toBe(packaged)
  })
})
