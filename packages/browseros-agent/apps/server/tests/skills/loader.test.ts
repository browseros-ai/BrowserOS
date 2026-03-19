import { afterEach, beforeEach, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

let testDir: string
let builtinDir: string

beforeEach(async () => {
  testDir = await mkdtemp(join(tmpdir(), 'loader-test-'))
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

const { loadAllSkills, loadSkills } = await import('../../src/skills/loader')

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

const BUILTIN_DISABLED = `---
name: deep-research
description: Research a topic
metadata:
  display-name: Deep Research
  enabled: "false"
  version: "1.0"
---

# Deep Research
`

const USER_SKILL = `---
name: my-workflow
description: My custom workflow
metadata:
  display-name: My Workflow
  enabled: "true"
---

# My Workflow
`

describe('loader two-directory scanning', () => {
  it('marks builtin/ skills as builtIn: true', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )

    const skills = await loadAllSkills()
    const skill = skills.find((s) => s.id === 'summarize-page')
    assert.ok(skill)
    assert.strictEqual(skill.builtIn, true)
  })

  it('marks root skills as builtIn: false', async () => {
    await mkdir(join(testDir, 'my-workflow'), { recursive: true })
    await writeFile(join(testDir, 'my-workflow', 'SKILL.md'), USER_SKILL)

    const skills = await loadAllSkills()
    const skill = skills.find((s) => s.id === 'my-workflow')
    assert.ok(skill)
    assert.strictEqual(skill.builtIn, false)
  })

  it('merges skills from both directories', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    await mkdir(join(testDir, 'my-workflow'), { recursive: true })
    await writeFile(join(testDir, 'my-workflow', 'SKILL.md'), USER_SKILL)

    const skills = await loadAllSkills()
    assert.strictEqual(skills.length, 2)
  })

  it('skips builtin/ subdirectory when scanning root', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )

    const skills = await loadAllSkills()
    const dupes = skills.filter((s) => s.id === 'summarize-page')
    assert.strictEqual(dupes.length, 1)
    assert.strictEqual(dupes[0].builtIn, true)
  })

  it('loadSkills filters out disabled skills', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    await mkdir(join(builtinDir, 'deep-research'), { recursive: true })
    await writeFile(
      join(builtinDir, 'deep-research', 'SKILL.md'),
      BUILTIN_DISABLED,
    )

    const skills = await loadSkills()
    assert.strictEqual(skills.length, 1)
    assert.strictEqual(skills[0].id, 'summarize-page')
  })
})
