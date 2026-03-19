import { afterAll, beforeAll, describe, it, mock } from 'bun:test'
import assert from 'node:assert'
import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

let testDir: string
let builtinDir: string

mock.module('../../src/lib/browseros-dir', () => ({
  getSkillsDir: () => testDir,
  getBuiltinSkillsDir: () => builtinDir,
}))

mock.module('../../src/env', () => ({
  INLINED_ENV: {
    SKILLS_CATALOG_URL: 'https://cdn.browseros.com/skills/v1/catalog.json',
  },
}))

const { syncBuiltinSkills } = await import('../../src/skills/remote-sync')

beforeAll(async () => {
  testDir = join(tmpdir(), `flow-test-${Date.now()}`)
  builtinDir = join(testDir, 'builtin')
  await mkdir(builtinDir, { recursive: true })
})

afterAll(async () => {
  await rm(testDir, { recursive: true, force: true })
})

describe('Flow tests against live CDN', () => {
  it('syncs all skills from CDN on fresh install', async () => {
    await syncBuiltinSkills()
    const entries = await readdir(builtinDir)
    const skills = entries.filter((e) => !e.startsWith('.'))
    assert.strictEqual(skills.length, 12)
  })

  it('preserves disabled state during sync', async () => {
    const skillPath = join(builtinDir, 'summarize-page', 'SKILL.md')
    let content = await readFile(skillPath, 'utf-8')

    content = content.replace(/enabled: "true"/, 'enabled: "false"')
    content = content.replace(/version: "1.0"/, 'version: "0.9"')
    await writeFile(skillPath, content)

    await syncBuiltinSkills()

    const afterSync = await readFile(skillPath, 'utf-8')
    assert.ok(
      afterSync.includes('enabled: "false"') ||
        afterSync.includes("enabled: 'false'"),
      'disabled state should be preserved',
    )
  })

  it('reinstalls deleted builtin skill', async () => {
    await rm(join(builtinDir, 'save-page'), { recursive: true })
    await syncBuiltinSkills()
    const content = await readFile(
      join(builtinDir, 'save-page', 'SKILL.md'),
      'utf-8',
    )
    assert.ok(content.includes('name: save-page'))
  })

  it('never touches user-created skill in root', async () => {
    const customDir = join(testDir, 'my-workflow')
    await mkdir(customDir, { recursive: true })
    const custom = '---\nname: my-workflow\ndescription: custom\n---\n# Mine\n'
    await writeFile(join(customDir, 'SKILL.md'), custom)

    await syncBuiltinSkills()

    const afterSync = await readFile(join(customDir, 'SKILL.md'), 'utf-8')
    assert.strictEqual(afterSync, custom)
  })
})
