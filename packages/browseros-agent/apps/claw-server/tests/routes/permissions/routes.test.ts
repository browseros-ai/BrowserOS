/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration test for the /permissions/catalog route. Returns the
 * baked-in approval catalog; this test pins the shape so a UI bump
 * that re-adds a category notices the backend hasn't followed.
 */

import { describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import type { ApprovalCategory } from '../../../src/lib/approval-catalog'
import app, { type AppType } from '../../../src/server'

function client() {
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

describe('/permissions/catalog route', () => {
  test('returns the six baked-in categories with the expected defaults', async () => {
    const api = client()
    const res = await api.permissions.catalog.$get()
    expect(res.status).toBe(200)
    const catalog = (await res.json()) as ApprovalCategory[]
    expect(catalog.map((c) => c.id)).toEqual([
      'submit',
      'payment',
      'delete',
      'upload',
      'navigate',
      'input',
    ])
    const byId = Object.fromEntries(catalog.map((c) => [c.id, c]))
    expect(byId.submit.defaultVerdict).toBe('Ask')
    expect(byId.payment.defaultVerdict).toBe('Block')
    expect(byId.payment.allowAuto).toBe(false)
    expect(byId.input.defaultVerdict).toBe('Auto')
    expect(byId.input.allowAuto).toBe(true)
  })
})
