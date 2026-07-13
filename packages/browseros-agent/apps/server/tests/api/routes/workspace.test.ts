/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync, promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Hono } from 'hono'
import { closeDb, initializeDb } from '../../../src/lib/db'
import { createWorkspaceRoutes } from '../../../src/api/routes/workspace'
import { WorkspaceStore } from '../../../src/lib/workspace/workspace-store'

describe('workspace routes', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })),
    )
  })

  it('creates and reads workspace records and assets over HTTP', async () => {
    const root = mkdtempSync(join(tmpdir(), 'browseros-workspace-route-test-'))
    tempDirs.push(root)
    const handle = initializeDb({ dbPath: ':memory:' })
    const app = new Hono().route(
      '/workspace',
      createWorkspaceRoutes({
        store: new WorkspaceStore({
          db: handle.db,
          assetsDir: join(root, 'assets'),
        }),
      }),
    )
    const jsonHeaders = { 'Content-Type': 'application/json' }

    const collectionResponse = await app.request('/workspace/collections', {
      method: 'POST',
      headers: jsonHeaders,
      body: JSON.stringify({ name: 'Saved sources' }),
    })
    expect(collectionResponse.status).toBe(201)
    const collection = (await collectionResponse.json()).collection

    const databaseResponse = await app.request('/workspace/databases', {
      method: 'POST',
      headers: jsonHeaders,
      body: JSON.stringify({ name: 'Listings', collectionId: collection.id }),
    })
    expect(databaseResponse.status).toBe(201)
    const database = (await databaseResponse.json()).database

    const recordResponse = await app.request(
      `/workspace/databases/${database.id}/records`,
      {
        method: 'POST',
        headers: jsonHeaders,
        body: JSON.stringify({ data: { title: 'Example listing', price: 100 } }),
      },
    )
    expect(recordResponse.status).toBe(201)
    const record = (await recordResponse.json()).record

    const assetResponse = await app.request('/workspace/assets', {
      method: 'POST',
      headers: jsonHeaders,
      body: JSON.stringify({
        filename: 'evidence.txt',
        mimeType: 'text/plain',
        dataBase64: Buffer.from('evidence').toString('base64'),
        recordId: record.id,
      }),
    })
    expect(assetResponse.status).toBe(201)
    const asset = (await assetResponse.json()).asset

    const contentResponse = await app.request(
      `/workspace/assets/${asset.id}/content`,
    )
    expect(contentResponse.status).toBe(200)
    await expect(contentResponse.text()).resolves.toBe('evidence')
  })
})
