import { afterEach, beforeEach, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

let testDir: string
let builtinDir: string

beforeEach(async () => {
  testDir = await mkdtemp(join(tmpdir(), 'service-test-'))
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

const { createSkill, deleteSkill, getSkill, updateSkill } = await import(
  '../../src/skills/service'
)

const BUILTIN_SKILL = `---
name: summarize-page
description: Summarize a page
metadata:
  display-name: Summarize Page
  enabled: "true"
  version: "1.0"
---

# Summarize Page
`

describe('getSkill', () => {
  it('finds builtin skill with builtIn: true', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    const skill = await getSkill('summarize-page')
    assert.ok(skill)
    assert.strictEqual(skill.builtIn, true)
  })

  it('finds user skill with builtIn: false', async () => {
    await createSkill({
      name: 'My Skill',
      description: 'Custom',
      content: '# Custom',
    })
    const skill = await getSkill('my-skill')
    assert.ok(skill)
    assert.strictEqual(skill.builtIn, false)
  })
})

describe('createSkill', () => {
  it('creates in user directory with builtIn: false', async () => {
    const skill = await createSkill({
      name: 'My Skill',
      description: 'Custom',
      content: '# Custom',
    })
    assert.strictEqual(skill.builtIn, false)
    assert.ok(!skill.location.includes('builtin'))
  })

  it('rejects if id collides with builtin skill', async () => {
    await mkdir(join(builtinDir, 'my-skill'), { recursive: true })
    await writeFile(join(builtinDir, 'my-skill', 'SKILL.md'), BUILTIN_SKILL)
    await assert.rejects(
      () =>
        createSkill({
          name: 'My Skill',
          description: 'Custom',
          content: '# Custom',
        }),
      /already exists/,
    )
  })
})

describe('updateSkill', () => {
  it('updates builtin skill in place', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    const updated = await updateSkill('summarize-page', { enabled: false })
    assert.strictEqual(updated.enabled, false)
    assert.strictEqual(updated.builtIn, true)
  })
})

describe('deleteSkill', () => {
  it('deletes user skill', async () => {
    await createSkill({
      name: 'My Skill',
      description: 'Custom',
      content: '# Custom',
    })
    await deleteSkill('my-skill')
    assert.strictEqual(await getSkill('my-skill'), null)
  })

  it('rejects deleting builtin skill', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    await assert.rejects(
      () => deleteSkill('summarize-page'),
      /Cannot delete built-in skill/,
    )
  })
})
