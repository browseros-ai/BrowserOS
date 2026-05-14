import { afterEach, beforeEach, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
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
  getSkillsSourcesPath: () => join(testDir, 'sources.json'),
  getSkillsStatePath: () => join(testDir, 'state.json'),
}))

const {
  createSkill,
  deleteSkill,
  getSkill,
  updateSkill,
  listSkillSources,
  createSkillSource,
  updateSkillSource,
  deleteSkillSource,
} = await import('../../src/skills/service')
const { getSkillsSourcesPath, getSkillsStatePath } = await import(
  '../../src/lib/browseros-dir'
)
const {
  loadSkillsSources,
  loadSkillsState,
  saveSkillsSources,
  saveSkillsState,
} = await import('../../src/skills/state')

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

describe('skills state persistence', () => {
  it('persists external source registry and runtime state in BrowserOS dir', async () => {
    await saveSkillsSources({
      version: 1,
      sources: [
        {
          id: 'agents-home',
          type: 'external',
          path: '/home/test/.agents/skills',
          enabled: true,
          label: 'Agent Skills Home',
        },
      ],
    })
    await saveSkillsState({
      version: 1,
      skills: {
        'external:agents-home:compare-prices': { enabled: false },
      },
    })

    assert.deepStrictEqual(await loadSkillsSources(), {
      version: 1,
      sources: [
        {
          id: 'agents-home',
          type: 'external',
          path: '/home/test/.agents/skills',
          enabled: true,
          label: 'Agent Skills Home',
        },
      ],
    })
    assert.deepStrictEqual(await loadSkillsState(), {
      version: 1,
      skills: {
        'external:agents-home:compare-prices': { enabled: false },
      },
    })

    assert.strictEqual(
      (await readFile(getSkillsSourcesPath(), 'utf-8')).includes('agents-home'),
      true,
    )
    assert.strictEqual(
      (await readFile(getSkillsStatePath(), 'utf-8')).includes(
        'compare-prices',
      ),
      true,
    )
  })
})

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

const EXTERNAL_SKILL = `---
name: compare-prices
description: Compare prices across stores
metadata:
  display-name: Compare Prices
  enabled: "true"
  version: "1.0"
---

# Compare Prices
`

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

  it('rejects deleting external skill', async () => {
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

    await assert.rejects(
      () => deleteSkill('compare-prices'),
      /Cannot delete external skill/,
    )
  })
})

describe('external skill toggle', () => {
  it('updates external skill enabled state in BrowserOS state file without editing external content', async () => {
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

    const updated = await updateSkill('compare-prices', { enabled: false })
    assert.strictEqual(updated.enabled, false)
    assert.strictEqual(updated.sourceKind, 'external')

    // State file should have the override
    const state = await loadSkillsState()
    assert.strictEqual(
      state.skills['external:ext:compare-prices'].enabled,
      false,
    )

    // External SKILL.md should be untouched
    const externalContent = await readFile(
      join(externalDir, 'compare-prices', 'SKILL.md'),
      'utf-8',
    )
    assert.ok(externalContent.includes('enabled: "true"'))
  })

  it('rejects editing external skill content', async () => {
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

    await assert.rejects(
      () => updateSkill('compare-prices', { content: '# changed' }),
      /Cannot edit external skill/,
    )
  })

  it('rejects editing external skill name or description', async () => {
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

    await assert.rejects(
      () => updateSkill('compare-prices', { name: 'New Name' }),
      /Cannot edit external skill/,
    )
    await assert.rejects(
      () => updateSkill('compare-prices', { description: 'New Desc' }),
      /Cannot edit external skill/,
    )
  })
})

describe('source registry CRUD', () => {
  it('listSkillSources returns all sources from registry', async () => {
    await saveSkillsSources({
      version: 1,
      sources: [
        {
          id: 'agents-home',
          type: 'external',
          path: '/home/test/.agents/skills',
          enabled: true,
          label: 'Agent Skills',
        },
      ],
    })
    const sources = await listSkillSources()
    assert.strictEqual(sources.length, 1)
    assert.strictEqual(sources[0].id, 'agents-home')
    assert.strictEqual(sources[0].path, '/home/test/.agents/skills')
  })

  it('creates, updates, and deletes external source registry entries', async () => {
    const created = await createSkillSource({
      id: 'agents-home',
      path: '/home/test/.agents/skills',
      enabled: true,
      label: 'Agent Skills Home',
    })
    assert.strictEqual(created.id, 'agents-home')
    assert.strictEqual(created.type, 'external')
    assert.strictEqual(created.path, '/home/test/.agents/skills')

    const updated = await updateSkillSource('agents-home', {
      enabled: false,
      label: 'Renamed',
    })
    assert.strictEqual(updated.enabled, false)
    assert.strictEqual(updated.label, 'Renamed')

    const afterUpdate = await listSkillSources()
    assert.strictEqual(afterUpdate.length, 1)
    assert.strictEqual(afterUpdate[0].label, 'Renamed')

    await deleteSkillSource('agents-home')
    const afterDelete = await listSkillSources()
    assert.strictEqual(afterDelete.length, 0)
  })

  it('rejects creating duplicate source id', async () => {
    await createSkillSource({
      id: 'ext',
      path: '/home/test/ext',
      enabled: true,
    })
    await assert.rejects(
      () =>
        createSkillSource({
          id: 'ext',
          path: '/home/test/ext2',
          enabled: true,
        }),
      /already exists/,
    )
  })

  it('rejects updating non-existent source', async () => {
    await assert.rejects(
      () => updateSkillSource('nope', { enabled: false }),
      /not found/,
    )
  })

  it('rejects deleting non-existent source', async () => {
    await assert.rejects(() => deleteSkillSource('nope'), /not found/)
  })
})

describe('createSkill with source-aware fields', () => {
  it('sets sourceKind=local and sourceId=local on created skill', async () => {
    const skill = await createSkill({
      name: 'My Skill',
      description: 'Custom',
      content: '# Custom',
    })
    assert.strictEqual(skill.sourceKind, 'local')
    assert.strictEqual(skill.sourceId, 'local')
  })
})

describe('getSkill with source-aware fields', () => {
  it('returns sourceKind/sourceId for builtin skill', async () => {
    await mkdir(join(builtinDir, 'summarize-page'), { recursive: true })
    await writeFile(
      join(builtinDir, 'summarize-page', 'SKILL.md'),
      BUILTIN_SKILL,
    )
    const skill = await getSkill('summarize-page')
    assert.ok(skill)
    assert.strictEqual(skill.sourceKind, 'builtin')
    assert.strictEqual(skill.sourceId, 'builtin')
  })

  it('returns sourceKind/sourceId for external skill', async () => {
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
    const skill = await getSkill('compare-prices')
    assert.ok(skill)
    assert.strictEqual(skill.sourceKind, 'external')
    assert.strictEqual(skill.sourceId, 'ext')
  })
})
