import { afterEach, beforeEach, describe, it, mock, spyOn } from 'bun:test'
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
import type { RemoteSkillCatalog } from '../../src/skills/types'

let testDir: string
let builtinDir: string

mock.module('../../src/lib/browseros-dir', () => ({
  getSkillsDir: () => testDir,
  getBuiltinSkillsDir: () => builtinDir,
}))

const { fetchRemoteCatalog, syncBuiltinSkills } = await import(
  '../../src/skills/remote-sync'
)

function makeCatalog(
  skills: { id: string; version: string; content: string }[],
): RemoteSkillCatalog {
  return { version: 1, skills }
}

const SKILL_V1 = `---
name: test-skill
description: A test skill
metadata:
  display-name: Test Skill
  enabled: "true"
  version: "1.0"
---

# Test Skill

Do the thing.
`

const SKILL_V2 = `---
name: test-skill
description: A test skill (updated)
metadata:
  display-name: Test Skill
  enabled: "true"
  version: "2.0"
---

# Test Skill v2

Do the thing better.
`

beforeEach(async () => {
  testDir = await mkdtemp(join(tmpdir(), 'skill-sync-'))
  builtinDir = join(testDir, 'builtin')
  await mkdir(builtinDir, { recursive: true })
})

afterEach(async () => {
  await rm(testDir, { recursive: true, force: true })
  mock.restore()
})

describe('fetchRemoteCatalog', () => {
  it('returns null on network failure', async () => {
    const spy = spyOn(globalThis, 'fetch').mockRejectedValue(
      new Error('offline'),
    )
    assert.strictEqual(await fetchRemoteCatalog(), null)
    spy.mockRestore()
  })

  it('returns null on non-ok response', async () => {
    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('Not Found', { status: 404 }),
    )
    assert.strictEqual(await fetchRemoteCatalog(), null)
    spy.mockRestore()
  })

  it('returns catalog on success', async () => {
    const catalog = makeCatalog([
      { id: 'test', version: '1.0', content: 'hello' },
    ])
    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify(catalog), { status: 200 }),
    )
    assert.deepStrictEqual(await fetchRemoteCatalog(), catalog)
    spy.mockRestore()
  })
})

describe('syncBuiltinSkills', () => {
  it('installs from remote into builtin/', async () => {
    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([{ id: 'new-skill', version: '1.0', content: SKILL_V1 }]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const content = await readFile(
      join(builtinDir, 'new-skill', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, SKILL_V1)
    spy.mockRestore()
  })

  it('updates skill when remote has newer version', async () => {
    await mkdir(join(builtinDir, 'test-skill'), { recursive: true })
    await writeFile(join(builtinDir, 'test-skill', 'SKILL.md'), SKILL_V1)

    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: 'test-skill', version: '2.0', content: SKILL_V2 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const content = await readFile(
      join(builtinDir, 'test-skill', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, SKILL_V2)
    spy.mockRestore()
  })

  it('skips when version matches', async () => {
    await mkdir(join(builtinDir, 'test-skill'), { recursive: true })
    await writeFile(join(builtinDir, 'test-skill', 'SKILL.md'), SKILL_V1)

    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: 'test-skill', version: '1.0', content: SKILL_V1 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const content = await readFile(
      join(builtinDir, 'test-skill', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, SKILL_V1)
    spy.mockRestore()
  })

  it('preserves enabled:false when updating', async () => {
    const disabledV1 = SKILL_V1.replace('enabled: "true"', 'enabled: "false"')
    await mkdir(join(builtinDir, 'test-skill'), { recursive: true })
    await writeFile(join(builtinDir, 'test-skill', 'SKILL.md'), disabledV1)

    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: 'test-skill', version: '2.0', content: SKILL_V2 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const content = await readFile(
      join(builtinDir, 'test-skill', 'SKILL.md'),
      'utf-8',
    )
    assert.ok(content.includes('v2'), 'should have v2 content')
    assert.ok(
      content.includes('enabled: "false"') ||
        content.includes("enabled: 'false'"),
      'should preserve disabled state',
    )
    spy.mockRestore()
  })

  it('falls back to bundled defaults when offline', async () => {
    const spy = spyOn(globalThis, 'fetch').mockRejectedValue(
      new Error('offline'),
    )
    await syncBuiltinSkills()
    const entries = await readdir(builtinDir)
    const skills = entries.filter((e: string) => !e.startsWith('.'))
    assert.ok(skills.length > 0, 'should have bundled defaults')
    spy.mockRestore()
  })

  it('removes builtin skill not in catalog', async () => {
    await mkdir(join(builtinDir, 'old-skill'), { recursive: true })
    await writeFile(join(builtinDir, 'old-skill', 'SKILL.md'), SKILL_V1)

    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: 'other-skill', version: '1.0', content: SKILL_V2 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const exists = await stat(join(builtinDir, 'old-skill'))
      .then(() => true)
      .catch(() => false)
    assert.strictEqual(exists, false)
    spy.mockRestore()
  })

  it('does not touch user skills in root', async () => {
    const custom = '---\nname: my-custom\ndescription: mine\n---\n# Mine\n'
    await mkdir(join(testDir, 'my-custom'), { recursive: true })
    await writeFile(join(testDir, 'my-custom', 'SKILL.md'), custom)

    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: 'test-skill', version: '1.0', content: SKILL_V1 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const content = await readFile(
      join(testDir, 'my-custom', 'SKILL.md'),
      'utf-8',
    )
    assert.strictEqual(content, custom)
    spy.mockRestore()
  })

  it('rejects path traversal in skill ids', async () => {
    const spy = spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify(
          makeCatalog([
            { id: '../../etc/evil', version: '1.0', content: SKILL_V1 },
          ]),
        ),
        { status: 200 },
      ),
    )
    await syncBuiltinSkills()
    const exists = await stat(join(builtinDir, '..', '..', 'etc', 'evil'))
      .then(() => true)
      .catch(() => false)
    assert.strictEqual(exists, false)
    spy.mockRestore()
  })
})
