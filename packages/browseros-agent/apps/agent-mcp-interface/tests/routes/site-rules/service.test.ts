/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { readJson } from '../../../src/lib/storage'
import { siteRulesFileSchema } from '../../../src/routes/site-rules/schemas'
import * as siteRules from '../../../src/routes/site-rules/service'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

describe('site-rules service', () => {
  test('list returns [] before any rule is added (file does not exist)', async () => {
    await withTempBrowserosDir(async (dir) => {
      expect(await siteRules.list()).toEqual([])
      expect(existsSync(join(dir, 'mcp-interface/site-rules.json'))).toBe(false)
    })
  })

  test('add creates the file and round-trips through the schema', async () => {
    await withTempBrowserosDir(async (dir) => {
      const created = await siteRules.add({
        label: 'Wire transfers',
        domain: 'mercury.com',
        action: 'payments',
      })
      expect(created.id).toMatch(/^[A-Za-z0-9_-]+$/)
      expect(created.label).toBe('Wire transfers')
      const file = join(dir, 'mcp-interface/site-rules.json')
      expect(existsSync(file)).toBe(true)
      const stored = await readJson('site-rules.json', siteRulesFileSchema)
      expect(stored).toHaveLength(1)
      expect(stored[0]).toEqual(created)
    })
  })

  test('list returns rules in insertion order', async () => {
    await withTempBrowserosDir(async () => {
      const a = await siteRules.add({
        label: 'A',
        domain: 'a.com',
        action: 'submit',
      })
      const b = await siteRules.add({
        label: 'B',
        domain: 'b.com',
        action: 'submit',
      })
      const c = await siteRules.add({
        label: 'C',
        domain: 'c.com',
        action: 'submit',
      })
      const rows = await siteRules.list()
      expect(rows.map((r) => r.id)).toEqual([a.id, b.id, c.id])
    })
  })

  test('duplicate (domain, action, label) tuples are allowed (user-managed)', async () => {
    await withTempBrowserosDir(async () => {
      const first = await siteRules.add({
        label: 'Wire transfers',
        domain: 'mercury.com',
        action: 'payments',
      })
      const second = await siteRules.add({
        label: 'Wire transfers',
        domain: 'mercury.com',
        action: 'payments',
      })
      expect(first.id).not.toBe(second.id)
      expect(await siteRules.list()).toHaveLength(2)
    })
  })

  test('remove deletes the rule, returns the id, and shortens the list', async () => {
    await withTempBrowserosDir(async () => {
      const created = await siteRules.add({
        label: 'X',
        domain: 'x.com',
        action: 'submit',
      })
      const removed = await siteRules.remove(created.id)
      expect(removed).toEqual({ id: created.id })
      expect(await siteRules.list()).toEqual([])
    })
  })

  test('remove returns null when the id is unknown', async () => {
    await withTempBrowserosDir(async () => {
      await siteRules.add({ label: 'X', domain: 'x.com', action: 'submit' })
      expect(await siteRules.remove('ghost')).toBeNull()
      expect(await siteRules.list()).toHaveLength(1)
    })
  })

  test('remove returns null when no rules file exists yet', async () => {
    await withTempBrowserosDir(async () => {
      expect(await siteRules.remove('anything')).toBeNull()
    })
  })

  test('traversal-shaped ids resolve to null without touching disk', async () => {
    await withTempBrowserosDir(async (dir) => {
      await siteRules.add({ label: 'X', domain: 'x.com', action: 'submit' })
      const evilIds = [
        '../config',
        '..',
        '../../etc/passwd',
        'site-rules/../config',
      ]
      for (const evil of evilIds) {
        expect(await siteRules.remove(evil)).toBeNull()
      }
      // The real rule is untouched.
      expect(await siteRules.list()).toHaveLength(1)
      expect(existsSync(join(dir, 'mcp-interface/site-rules.json'))).toBe(true)
    })
  })

  test('ten parallel adds all persist (no read-then-rewrite race)', async () => {
    await withTempBrowserosDir(async () => {
      const count = 10
      await Promise.all(
        Array.from({ length: count }, (_, i) =>
          siteRules.add({
            label: `Rule ${i}`,
            domain: `domain-${i}.com`,
            action: 'submit',
          }),
        ),
      )
      const rows = await siteRules.list()
      expect(rows).toHaveLength(count)
      expect(new Set(rows.map((r) => r.id)).size).toBe(count)
    })
  })

  test('findMatching honours glob patterns end-to-end', async () => {
    await withTempBrowserosDir(async () => {
      await siteRules.add({
        label: 'Wire',
        domain: 'mercury.com',
        action: 'payments',
      })
      await siteRules.add({
        label: 'Admin',
        domain: 'admin.*',
        action: 'admin',
      })
      await siteRules.add({
        label: 'Stripe',
        domain: '*.stripe.com',
        action: 'payments',
      })

      // Exact match.
      const exact = await siteRules.findMatching('mercury.com', 'payments')
      expect(exact.map((r) => r.label)).toEqual(['Wire'])

      // Subdomain wildcard hits `*.stripe.com`.
      const sub = await siteRules.findMatching('api.stripe.com', 'payments')
      expect(sub.map((r) => r.label)).toEqual(['Stripe'])

      // Trailing wildcard hits `admin.*` for matching action only.
      const adm = await siteRules.findMatching('admin.example.com', 'admin')
      expect(adm.map((r) => r.label)).toEqual(['Admin'])

      // Same domain but wrong action returns nothing.
      const wrongAction = await siteRules.findMatching('mercury.com', 'submit')
      expect(wrongAction).toEqual([])

      // No matching rule at all.
      const noMatch = await siteRules.findMatching('elsewhere.org', 'submit')
      expect(noMatch).toEqual([])
    })
  })
})
