/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { readJson } from '../../../src/lib/storage'
import type { NewAgentValues } from '../../../src/routes/agents/schemas'
import { storedAgentProfileSchema } from '../../../src/routes/agents/schemas'
import * as agents from '../../../src/routes/agents/service'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

function makeInput(overrides: Partial<NewAgentValues> = {}): NewAgentValues {
  return {
    name: 'Cowork . Finance ops',
    harness: 'Claude Desktop',
    loginMode: 'profile',
    selectedSites: [],
    approvals: {
      submit: 'Ask',
      payment: 'Block',
      delete: 'Ask',
      upload: 'Ask',
      navigate: 'Ask',
      input: 'Auto',
    },
    aclRuleIds: ['wire-transfers', 'payment-methods'],
    customAclRules: [],
    ...overrides,
  }
}

describe('agents service', () => {
  test('create persists a stored profile that round-trips through the schema', async () => {
    await withTempBrowserosDir(async (dir) => {
      const created = await agents.create(makeInput())
      const file = join(dir, 'claw-server/agents', `${created.id}.json`)
      expect(existsSync(file)).toBe(true)
      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.id).toBe(created.id)
      expect(stored.slug).toBe(created.slug)
      expect(stored.status).toBe('configured')
      expect(stored.createdAt).toBeTruthy()
      expect(stored.updatedAt).toBe(stored.createdAt)
    })
  })

  test('create derives slug from the name; second create with same name appends -2', async () => {
    await withTempBrowserosDir(async () => {
      const first = await agents.create(makeInput({ name: 'Foo' }))
      const second = await agents.create(makeInput({ name: 'Foo' }))
      expect(first.slug).toBe('foo')
      expect(second.slug).toBe('foo-2')
    })
  })

  test('list returns the directory projection with derived fields', async () => {
    await withTempBrowserosDir(async () => {
      await agents.create(
        makeInput({
          name: 'Selective Agent',
          loginMode: 'selective',
          selectedSites: ['concur.com', 'stripe.com', 'ramp.com'],
          aclRuleIds: ['a', 'b', 'c'],
          approvals: { submit: 'Block', payment: 'Block', input: 'Auto' },
        }),
      )
      const rows = await agents.list()
      expect(rows).toHaveLength(1)
      const row = rows[0]
      expect(row.loginScopeLabel).toBe('Selective (3)')
      expect(row.loginCount).toBe(3)
      expect(row.aclRuleCount).toBe(3)
      expect(row.blockedActionCount).toBe(2)
      expect(row.alwaysAllowCount).toBe(0)
      expect(row.lastRunAt).toBe('Never run')
      expect(row.status).toBe('configured')
      expect(row.mcpUrl).toMatch(/\/mcp\/selective-agent$/)
    })
  })

  test('list sorts by updatedAt descending', async () => {
    await withTempBrowserosDir(async () => {
      const a = await agents.create(makeInput({ name: 'Alpha' }))
      // Force a small delay so the second create has a strictly later
      // updatedAt; ISO strings sort lexicographically the same way.
      await new Promise((resolve) => setTimeout(resolve, 10))
      const b = await agents.create(makeInput({ name: 'Beta' }))
      const rows = await agents.list()
      expect(rows.map((row) => row.id)).toEqual([b.id, a.id])
    })
  })

  test('getDetail returns the wizard-shape values', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(
        makeInput({
          name: 'Detail Test',
          loginMode: 'selective',
          selectedSites: ['concur.com'],
        }),
      )
      const detail = await agents.getDetail(created.id)
      expect(detail).not.toBeNull()
      if (!detail) throw new Error('unreachable')
      expect(detail.name).toBe('Detail Test')
      expect(detail.loginMode).toBe('selective')
      expect(detail.selectedSites).toEqual(['concur.com'])
      // Wizard shape carries no server-managed fields.
      expect('id' in detail).toBe(false)
      expect('slug' in detail).toBe(false)
    })
  })

  test('getDetail returns null for unknown ids', async () => {
    await withTempBrowserosDir(async () => {
      expect(await agents.getDetail('ghost')).toBeNull()
    })
  })

  test('update rewrites the file; updatedAt advances, createdAt is preserved', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput())
      await new Promise((resolve) => setTimeout(resolve, 10))
      const updated = await agents.update(
        created.id,
        makeInput({ name: 'Renamed Profile' }),
      )
      expect(updated).not.toBeNull()
      if (!updated) throw new Error('unreachable')
      expect(updated.id).toBe(created.id)
      expect(updated.name).toBe('Renamed Profile')
      expect(updated.createdAt).toBeTruthy()
      expect(updated.updatedAt > updated.createdAt).toBe(true)
    })
  })

  test('update recomputes slug when the name changes', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput({ name: 'Cowork Finance' }))
      expect(created.slug).toBe('cowork-finance')
      const updated = await agents.update(
        created.id,
        makeInput({ name: 'Cowork Reporting' }),
      )
      expect(updated?.slug).toBe('cowork-reporting')
    })
  })

  test('update keeps slug stable when the name does not change', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput({ name: 'Stable' }))
      const updated = await agents.update(
        created.id,
        makeInput({ name: 'Stable' }),
      )
      expect(updated?.slug).toBe(created.slug)
    })
  })

  test('update returns null for unknown ids', async () => {
    await withTempBrowserosDir(async () => {
      expect(await agents.update('ghost', makeInput())).toBeNull()
    })
  })

  test('remove deletes the file and subsequent getDetail is null', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput())
      const removed = await agents.remove(created.id)
      expect(removed?.id).toBe(created.id)
      expect(removed?.harnessUninstall.installed).toBe(false)
      expect(await agents.getDetail(created.id)).toBeNull()
    })
  })

  test('remove returns null when the file does not exist', async () => {
    await withTempBrowserosDir(async () => {
      expect(await agents.remove('ghost')).toBeNull()
    })
  })

  test('regenerateMcpUrl rotates the slug and persists the new URL', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput({ name: 'Rotate' }))
      const result = await agents.regenerateMcpUrl(created.id)
      expect(result).not.toBeNull()
      if (!result) throw new Error('unreachable')
      expect(result.id).toBe(created.id)
      expect(result.mcpUrl).not.toBe(created.mcpUrl)
      expect(result.mcpUrl).toMatch(/\/mcp\/rotate-[a-z0-9-]+$/)
      const detail = await agents.getDetail(created.id)
      expect(detail).not.toBeNull()
    })
  })

  test('regenerateMcpUrl returns null for unknown ids', async () => {
    await withTempBrowserosDir(async () => {
      expect(await agents.regenerateMcpUrl('ghost')).toBeNull()
    })
  })

  test('list skips a corrupt agent file instead of rejecting the whole call', async () => {
    await withTempBrowserosDir(async (dir) => {
      const ok = await agents.create(makeInput({ name: 'Healthy' }))
      // Hand-write a garbage file under agents/. listFiles picks it
      // up; the per-file readJson rejects; loadAll logs + skips it.
      const { writeFile } = await import('node:fs/promises')
      const { join } = await import('node:path')
      await writeFile(
        join(dir, 'claw-server/agents', 'broken.json'),
        '{ this is not valid json',
        'utf8',
      )
      const rows = await agents.list()
      expect(rows.map((row) => row.id)).toEqual([ok.id])
      // The directory still serves new writes after a corrupt sibling.
      const fresh = await agents.create(makeInput({ name: 'After corruption' }))
      const after = await agents.list()
      expect(after.map((row) => row.id).sort()).toEqual(
        [ok.id, fresh.id].sort(),
      )
    })
  })

  test('traversal-shaped ids resolve as not-found across every read/write path', async () => {
    await withTempBrowserosDir(async () => {
      await agents.create(makeInput({ name: 'Real' }))
      // Build a path that LOOKS like a profile id but contains
      // characters the service must reject before the storage layer
      // sees them.
      const evilIds = [
        '../config',
        'agents/../config',
        '..',
        '../../etc/passwd',
      ]
      for (const evilId of evilIds) {
        expect(await agents.getDetail(evilId)).toBeNull()
        expect(await agents.update(evilId, makeInput())).toBeNull()
        expect(await agents.remove(evilId)).toBeNull()
        expect(await agents.regenerateMcpUrl(evilId)).toBeNull()
      }
    })
  })

  test('ten parallel creates with the same name produce distinct slugs (no TOCTOU race)', async () => {
    await withTempBrowserosDir(async () => {
      const count = 10
      const created = await Promise.all(
        Array.from({ length: count }, () =>
          agents.create(makeInput({ name: 'Race' })),
        ),
      )
      const slugs = created.map((c) => c.slug).sort()
      // Expected slugs are race, race-2, race-3, ..., race-10 (sorted
      // lexicographically -> race, race-10, race-2, race-3, ...).
      expect(new Set(slugs).size).toBe(count)
      expect(slugs).toContain('race')
      for (let i = 2; i <= count; i++) {
        expect(slugs).toContain(`race-${i}`)
      }
    })
  })

  test('findBySlug returns the profile owning the slug, null otherwise', async () => {
    await withTempBrowserosDir(async () => {
      const a = await agents.create(makeInput({ name: 'Find Me' }))
      const b = await agents.create(makeInput({ name: 'Other Agent' }))
      const found = await agents.findBySlug(a.slug)
      expect(found?.id).toBe(a.id)
      const other = await agents.findBySlug(b.slug)
      expect(other?.id).toBe(b.id)
      expect(await agents.findBySlug('does-not-exist')).toBeNull()
      // Empty directory after wipe (separate temp dir is the simplest).
    })
    await withTempBrowserosDir(async () => {
      expect(await agents.findBySlug('any-slug')).toBeNull()
    })
  })

  test('two parallel updates of different profiles do not corrupt each other', async () => {
    await withTempBrowserosDir(async () => {
      const a = await agents.create(makeInput({ name: 'Parallel A' }))
      const b = await agents.create(makeInput({ name: 'Parallel B' }))
      await Promise.all([
        agents.update(a.id, makeInput({ name: 'Parallel A renamed' })),
        agents.update(b.id, makeInput({ name: 'Parallel B renamed' })),
      ])
      const rows = await agents.list()
      expect(rows.map((row) => row.name).sort()).toEqual([
        'Parallel A renamed',
        'Parallel B renamed',
      ])
    })
  })
})
