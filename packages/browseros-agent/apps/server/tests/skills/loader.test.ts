import { afterEach, beforeEach, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

let testDir: string
let builtinDir: string
let sourcesPath: string
let statePath: string

beforeEach(async () => {
  testDir = await mkdtemp(join(tmpdir(), 'loader-test-'))
  builtinDir = join(testDir, 'builtin')
  sourcesPath = join(testDir, 'sources.json')
  statePath = join(testDir, 'state.json')
  await mkdir(builtinDir, { recursive: true })
})

afterEach(async () => {
  await rm(testDir, { recursive: true, force: true })
})

mock.module('../../src/lib/browseros-dir', () => ({
  getSkillsDir: () => testDir,
  getBuiltinSkillsDir: () => builtinDir,
  getSkillsSourcesPath: () => sourcesPath,
  getSkillsStatePath: () => statePath,
}))

const { loadAllSkills, loadSkills } = await import('../../src/skills/loader')
const { saveSkillsSources, saveSkillsState } = await import(
  '../../src/skills/state'
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

const EXTERNAL_SKILL = `---
name: compare-prices
description: Compare prices across stores
metadata:
  display-name: Compare Prices
  enabled: "true"
---

# Compare Prices
`

describe('source-aware loader', () => {
  it('scans external source directories from sources registry', async () => {
    const externalDir = join(testDir, 'external')
    await mkdir(join(externalDir, 'compare-prices'), { recursive: true })
    await writeFile(
      join(externalDir, 'compare-prices', 'SKILL.md'),
      EXTERNAL_SKILL,
    )

    await saveSkillsSources({
      version: 1,
      sources: [
        {
          id: 'ext',
          type: 'external',
          path: externalDir,
          enabled: true,
          label: 'External',
        },
      ],
    })

    const skills = await loadAllSkills()
    const ext = skills.find((s) => s.id === 'compare-prices')
    assert.ok(ext, 'should find external skill')
    assert.strictEqual(ext.sourceKind, 'external')
    assert.strictEqual(ext.sourceId, 'ext')
    assert.strictEqual(ext.sourceLabel, 'External')
    assert.strictEqual(ext.builtIn, false)
  })

  it('sets sourceKind=builtin and sourceId=builtin for builtin skills', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )

    const skills = await loadAllSkills()
    const skill = skills.find((s) => s.id === 'summarize-page')
    assert.ok(skill)
    assert.strictEqual(skill.sourceKind, 'builtin')
    assert.strictEqual(skill.sourceId, 'builtin')
  })

  it('sets sourceKind=local and sourceId=local for user skills', async () => {
    await mkdir(join(testDir, 'my-workflow'), { recursive: true })
    await writeFile(join(testDir, 'my-workflow', 'SKILL.md'), USER_SKILL)

    const skills = await loadAllSkills()
    const skill = skills.find((s) => s.id === 'my-workflow')
    assert.ok(skill)
    assert.strictEqual(skill.sourceKind, 'local')
    assert.strictEqual(skill.sourceId, 'local')
  })

  it('blocks duplicate skill ids across builtin and external sources', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )

    const externalDir = join(testDir, 'external')
    await mkdir(join(externalDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(externalDir, 'summarize-page', 'SKILL.md'),
      EXTERNAL_SKILL.replace('compare-prices', 'summarize-page').replace(
        'Compare Prices',
        'Ext Summary',
      ),
    )

    await saveSkillsSources({
      version: 1,
      sources: [
        {
          id: 'ext',
          type: 'external',
          path: externalDir,
          enabled: true,
          label: 'Ext',
        },
      ],
    })

    const all = await loadAllSkills()
    const dupes = all.filter((s) => s.id === 'summarize-page')
    assert.strictEqual(dupes.length, 2)
    assert.ok(dupes[0].conflict, 'first duplicate should have conflict')
    assert.ok(dupes[1].conflict, 'second duplicate should have conflict')
    assert.strictEqual(dupes[0].conflict?.kind, 'duplicate-id')
    assert.strictEqual(dupes[0].conflict?.collisions.length, 2)
  })

  it('overlays runtime state for external skills without mutating SKILL.md', async () => {
    const externalDir = join(testDir, 'external')
    await mkdir(join(externalDir, 'compare-prices'), { recursive: true })
    await writeFile(
      join(externalDir, 'compare-prices', 'SKILL.md'),
      EXTERNAL_SKILL,
    )

    await saveSkillsSources({
      version: 1,
      sources: [
        {
          id: 'ext',
          type: 'external',
          path: externalDir,
          enabled: true,
          label: 'Ext',
        },
      ],
    })
    await saveSkillsState({
      version: 1,
      skills: { 'external:ext:compare-prices': { enabled: false } },
    })

    const all = await loadAllSkills()
    const ext = all.find((s) => s.id === 'compare-prices')
    assert.ok(ext)
    assert.strictEqual(
      ext.enabled,
      false,
      'runtime state should disable the skill',
    )

    // SKILL.md must NOT be mutated
    const disk = await readFile(
      join(externalDir, 'compare-prices', 'SKILL.md'),
      'utf-8',
    )
    assert.ok(
      disk.includes('enabled: "true"'),
      'disk SKILL.md should still have enabled: true',
    )
  })

  it('loadSkills filters out disabled and conflicted skills', async () => {
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

    // Create conflict
    const externalDir = join(testDir, 'external')
    await mkdir(join(externalDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(externalDir, 'summarize-page', 'SKILL.md'),
      EXTERNAL_SKILL.replace('compare-prices', 'summarize-page').replace(
        'Compare Prices',
        'Ext',
      ),
    )

    await saveSkillsSources({
      version: 1,
      sources: [
        { id: 'ext', type: 'external', path: externalDir, enabled: true },
      ],
    })

    const resolved = await loadSkills()
    assert.strictEqual(
      resolved.length,
      0,
      'disabled deep-research + conflicted summarize-page = 0 enabled skills',
    )
  })

  it('skips disabled external sources', async () => {
    const externalDir = join(testDir, 'external')
    await mkdir(join(externalDir, 'compare-prices'), { recursive: true })
    await writeFile(
      join(externalDir, 'compare-prices', 'SKILL.md'),
      EXTERNAL_SKILL,
    )

    await saveSkillsSources({
      version: 1,
      sources: [
        { id: 'ext', type: 'external', path: externalDir, enabled: false },
      ],
    })

    const skills = await loadAllSkills()
    assert.strictEqual(skills.length, 0, 'disabled source should be skipped')
  })
})
