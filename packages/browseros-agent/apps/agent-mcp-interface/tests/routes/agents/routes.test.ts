/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration tests for the /agents routes. Hits the live Hono app
 * via the typed client (`hc<AppType>`) but routes everything through
 * `app.fetch` so there is no real socket bind. Each test gets its
 * own tmp `<browserosDir>` so state doesn't leak between cases.
 */

import { describe, expect, test } from 'bun:test'
import { hc } from 'hono/client'
import type {
  AgentProfileSummary,
  NewAgentValues,
  StoredAgentProfile,
} from '../../../src/routes/agents/schemas'
import app, { type AppType } from '../../../src/server'
import { withTempBrowserosDir } from '../../_helpers/temp-browseros-dir'

function client() {
  // hc only needs a base URL to construct request paths; the fetch
  // override sends every request to `app.fetch` so no port is bound.
  return hc<AppType>('http://localhost', {
    fetch: (input, init) => app.fetch(new Request(input, init)),
  })
}

function makeBody(overrides: Partial<NewAgentValues> = {}): NewAgentValues {
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
    aclRuleIds: ['wire-transfers'],
    customAclRules: [],
    ...overrides,
  }
}

describe('/agents routes', () => {
  test('full lifecycle: create → list → detail → update → regenerate → delete', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()

      // create
      const createRes = await api.agents.$post({ json: makeBody() })
      expect(createRes.status).toBe(201)
      const created = await createRes.json()
      expect(created.harness).toBe('Claude Desktop')
      expect(created.slug).toBe('cowork-finance-ops')
      expect(created.mcpUrl).toMatch(/\/mcp\/cowork-finance-ops$/)
      expect(created.cliCommand).toBe('mcp add cowork-finance-ops')

      // list
      const listRes = await api.agents.$get()
      expect(listRes.status).toBe(200)
      const list = (await listRes.json()) as AgentProfileSummary[]
      expect(list).toHaveLength(1)
      expect(list[0].id).toBe(created.id)

      // detail
      const detailRes = await api.agents[':id'].$get({
        param: { id: created.id },
      })
      expect(detailRes.status).toBe(200)
      const detail = (await detailRes.json()) as NewAgentValues
      expect(detail.name).toBe('Cowork . Finance ops')

      // update
      const patchRes = await api.agents[':id'].$patch({
        param: { id: created.id },
        json: makeBody({ name: 'Renamed' }),
      })
      expect(patchRes.status).toBe(200)
      const updated = (await patchRes.json()) as StoredAgentProfile
      expect(updated.name).toBe('Renamed')
      expect(updated.slug).toBe('renamed')

      // regenerate
      const regenRes = await api.agents[':id']['mcp-url:regenerate'].$post({
        param: { id: created.id },
      })
      expect(regenRes.status).toBe(200)
      const regen = await regenRes.json()
      expect(regen.id).toBe(created.id)
      expect(regen.mcpUrl).not.toBe(updated.mcpUrl)
      expect(regen.mcpUrl).toMatch(/\/mcp\/renamed-[a-z0-9-]+$/)

      // delete
      const delRes = await api.agents[':id'].$delete({
        param: { id: created.id },
      })
      expect(delRes.status).toBe(200)
      const del = (await delRes.json()) as {
        id: string
        harnessUninstall: { installed: boolean; message: string }
      }
      expect(del.id).toBe(created.id)
      expect(typeof del.harnessUninstall.message).toBe('string')

      // listed empty
      const emptyRes = await api.agents.$get()
      const empty = (await emptyRes.json()) as AgentProfileSummary[]
      expect(empty).toEqual([])
    })
  })

  test('404 paths for unknown ids', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const detail = await api.agents[':id'].$get({ param: { id: 'ghost' } })
      expect(detail.status).toBe(404)
      const patch = await api.agents[':id'].$patch({
        param: { id: 'ghost' },
        json: makeBody(),
      })
      expect(patch.status).toBe(404)
      const del = await api.agents[':id'].$delete({ param: { id: 'ghost' } })
      expect(del.status).toBe(404)
      const regen = await api.agents[':id']['mcp-url:regenerate'].$post({
        param: { id: 'ghost' },
      })
      expect(regen.status).toBe(404)
    })
  })

  test('400 when the create body fails zod validation', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const res = await api.agents.$post({
        // biome-ignore lint/suspicious/noExplicitAny: deliberate invalid body for the test
        json: { ...makeBody(), name: '' } as any,
      })
      expect(res.status).toBe(400)
    })
  })

  test('two creates with the same name produce slug + slug-2', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const first = await api.agents.$post({ json: makeBody({ name: 'Foo' }) })
      const second = await api.agents.$post({ json: makeBody({ name: 'Foo' }) })
      const a = await first.json()
      const b = await second.json()
      expect(a.slug).toBe('foo')
      expect(b.slug).toBe('foo-2')
    })
  })

  test('parallel updates of two distinct profiles do not corrupt each other', async () => {
    await withTempBrowserosDir(async () => {
      const api = client()
      const a = await (
        await api.agents.$post({ json: makeBody({ name: 'A' }) })
      ).json()
      const b = await (
        await api.agents.$post({ json: makeBody({ name: 'B' }) })
      ).json()
      await Promise.all([
        api.agents[':id'].$patch({
          param: { id: a.id },
          json: makeBody({ name: 'A renamed' }),
        }),
        api.agents[':id'].$patch({
          param: { id: b.id },
          json: makeBody({ name: 'B renamed' }),
        }),
      ])
      const list = (await (
        await api.agents.$get()
      ).json()) as AgentProfileSummary[]
      expect(list.map((row) => row.name).sort()).toEqual([
        'A renamed',
        'B renamed',
      ])
    })
  })
})
