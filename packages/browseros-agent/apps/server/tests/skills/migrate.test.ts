import { afterEach, beforeEach, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

let testDir: string
let builtinDir: string

beforeEach(async () => {
  testDir = await mkdtemp(join(tmpdir(), 'migrate-test-'))
  builtinDir = join(testDir, 'builtin')
  await mkdir(builtinDir, { recursive: true })
})

afterEach(async () => {
  await rm(testDir, { recursive: true, force: true })
})

mock.module('../../src/lib/browseros-dir', () => ({
  getSkillsDir: () => testDir,
  getBuiltinSkillsDir: () => builtinDir,
}))

const { migrateBuiltinSkills } = await import('../../src/skills/migrate')

const SKILL_CONTENT = `---
name: summarize-page
description: Summarize a page
metadata:
  display-name: Summarize Page
  enabled: "false"
  version: "1.0"
---

# Summarize Page
`

describe('migrateBuiltinSkills', () => {
  it('moves default skills from root to builtin/', async () => {
    await mkdir(join(testDir, 'summarize-page'), { recursive: true })
    await writeFile(join(testDir, 'summarize-page', 'SKILL.md'), SKILL_CONTENT)

    await migrateBuiltinSkills()

    const content = await readFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, SKILL_CONTENT)

    const oldExists = await stat(join(testDir, 'summarize-page'))
      .then(() => true)
      .catch(() => false)
    assert.strictEqual(oldExists, false)
  })

  it('does not move user-created skills', async () => {
    const userContent =
      '---\nname: my-workflow\ndescription: mine\n---\n# Mine\n'
    await mkdir(join(testDir, 'my-workflow'), { recursive: true })
    await writeFile(join(testDir, 'my-workflow', 'SKILL.md'), userContent)

    await migrateBuiltinSkills()

    const content = await readFile(
      join(testDir, 'my-workflow', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, userContent)
  })

  it('skips if builtin/ already has skills', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      SKILL_CONTENT,
    )
    await mkdir(join(testDir, 'deep-research'), { recursive: true })
    await writeFile(join(testDir, 'deep-research', 'SKILL.md'), SKILL_CONTENT)

    await migrateBuiltinSkills()

    const stillInRoot = await stat(join(testDir, 'deep-research'))
      .then(() => true)
      .catch(() => false)
    assert.strictEqual(stillInRoot, true)
  })

  it('is a no-op for fresh installs', async () => {
    await migrateBuiltinSkills()
    const entries = await readdir(builtinDir)
    assert.strictEqual(
      entries.filter((e: string) => !e.startsWith('.')).length,
      0,
    )
  })
})
