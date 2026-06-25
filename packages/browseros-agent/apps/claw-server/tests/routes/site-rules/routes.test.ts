/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration tests for the /site-rules routes. Hits the live Hono
 * app via the typed client (`hc<AppType>`), routes through
 * `app.fetch` so no real socket bind, and isolates each test with a
 * fresh `<browserosDir>`.
 */

import { describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import type { SiteRule } from '../../../src/routes/site-rules/schemas'
import app, { type AppType } from '../../../src/server'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

describe('/site-rules routes', () => {
  test('full lifecycle: empty list → create → list → delete → empty list', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()

      const emptyRes = await api['site-rules'].$get()
      expect(emptyRes.status).toBe(200)
      expect(await emptyRes.json()).toEqual([])

      const createRes = await api['site-rules'].$post({
        json: {
          label: 'Wire transfers',
          domain: 'mercury.com',
          action: 'payments',
        },
      })
      expect(createRes.status).toBe(201)
      const created = await createRes.json()
      expect(created.label).toBe('Wire transfers')
      expect(created.id).toMatch(/^[A-Za-z0-9_-]+$/)

      const listRes = await api['site-rules'].$get()
      const list = (await listRes.json()) as SiteRule[]
      expect(list).toHaveLength(1)
      expect(list[0].id).toBe(created.id)

      const delRes = await api['site-rules'][':id'].$delete({
        param: { id: created.id },
      })
      expect(delRes.status).toBe(200)
      expect(await delRes.json()).toEqual({ id: created.id })

      const finalRes = await api['site-rules'].$get()
      expect(await finalRes.json()).toEqual([])
    })
  })

  test('400 when action enum is invalid', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const res = await api['site-rules'].$post({
        json: {
          label: 'bad',
          domain: 'x.com',
          // biome-ignore lint/suspicious/noExplicitAny: deliberate invalid enum for the test
          action: 'BOGUS' as any,
        },
      })
      expect(res.status).toBe(400)
    })
  })

  test('400 when label is empty', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const res = await api['site-rules'].$post({
        json: { label: '', domain: 'x.com', action: 'submit' },
      })
      expect(res.status).toBe(400)
    })
  })

  test('400 when domain is empty', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const res = await api['site-rules'].$post({
        json: { label: 'x', domain: '', action: 'submit' },
      })
      expect(res.status).toBe(400)
    })
  })

  test('DELETE returns 404 for unknown id', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const res = await api['site-rules'][':id'].$delete({
        param: { id: 'does-not-exist' },
      })
      expect(res.status).toBe(404)
    })
  })

  test('DELETE returns 404 for traversal-shaped ids without touching disk', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      // Seed a rule so the file exists; the bogus delete must not
      // affect it.
      await api['site-rules'].$post({
        json: { label: 'Real', domain: 'real.com', action: 'submit' },
      })
      const evilIds = ['..%2F..%2Fetc%2Fpasswd', '..', '../config']
      for (const evil of evilIds) {
        const res = await api['site-rules'][':id'].$delete({
          param: { id: evil },
        })
        expect(res.status).toBe(404)
      }
      const listRes = await api['site-rules'].$get()
      expect(await listRes.json()).toHaveLength(1)
    })
  })
})
