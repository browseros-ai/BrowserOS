/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { existsSync, mkdtempSync, promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, getDbHandle, initializeDb } from '../../../src/lib/db'
import { WorkspaceStore } from '../../../src/lib/workspace/workspace-store'

describe('WorkspaceStore', () => {
  const tempDirs: string[] = []
  afterEach(async () => {
    try {
      getDbHandle().sqlite.exec('PRAGMA wal_checkpoint(TRUNCATE)')
    } catch {
      // The database may not have been initialized if setup failed.
    }
    closeDb()
    await Promise.all(
      tempDirs.splice(0).map((dir) =>
        fs.rm(dir, {
          recursive: true,
          force: true,
          maxRetries: 5,
          retryDelay: 100,
        }),
      ),
    )
  })

  it('manages databases, flexible records, sources, assets, and research recaps', async () => {
    const root = mkdtempSync(join(tmpdir(), 'browseros-workspace-test-'))
    tempDirs.push(root)
    const handle = initializeDb({ dbPath: ':memory:' })
    const assetsDir = join(root, 'workspace-assets')
    const store = new WorkspaceStore({ db: handle.db, assetsDir })

    const collection = await store.createCollection({
      name: 'Construction Research',
    })
    const database = await store.createDatabase({
      name: 'Material Prices',
      collectionId: collection.id,
    })
    const field = await store.createField({
      databaseId: database.id,
      name: 'Price',
      type: 'currency',
    })
    expect(field.key).toBe('price')

    const session = await store.createResearchSession({
      goal: 'Collect current cement prices',
      databaseId: database.id,
      collectionId: collection.id,
      plan: [
        {
          title: 'Inspect authorized sources',
          toolCategory: 'browser',
        },
      ],
    })
    const source = await store.createSource({
      url: 'https://example.com/cement',
      sessionId: session.id,
      title: 'Cement listing',
    })
    const record = await store.createRecord({
      databaseId: database.id,
      sessionId: session.id,
      sourceId: source.id,
      title: 'Example Cement',
      data: { price: 285, currency: 'PHP' },
    })
    expect(record.data).toEqual({ price: 285, currency: 'PHP' })

    const asset = await store.createAsset({
      filename: '../quotation.txt',
      mimeType: 'text/plain',
      data: new TextEncoder().encode('quotation'),
      recordId: record.id,
      sourceId: source.id,
      sessionId: session.id,
    })
    const storedAsset = await store.readAsset(asset.id)
    expect(storedAsset?.asset.contentHash).toHaveLength(64)
    expect(await fs.readFile(join(assetsDir, asset.storageKey), 'utf8')).toBe(
      'quotation',
    )

    await store.updatePlanStep(session.plan[0].id, { status: 'completed' })
    await store.addResearchEvent({
      sessionId: session.id,
      kind: 'activity',
      title: 'Source inspected',
    })
    const recap = await store.buildSessionRecap(session.id)
    expect(recap).toMatchObject({
      goal: 'Collect current cement prices',
      plan: { total: 1, completed: 1, blocked: 0, remaining: 0 },
      activityCount: 1,
      nextActions: [],
    })

    expect(await store.getResearchSession(session.id)).toMatchObject({
      recap,
      plan: [{ status: 'completed' }],
      events: [{ title: 'Source inspected' }],
    })

    const suggestion = await store.buildSessionSuggestion(session.id)
    expect(suggestion).toMatchObject({
      basedOn: 'Collect current cement prices',
    })
    expect(suggestion?.message).toContain('Review the saved results')

    expect(await store.deleteRecord(record.id)).toBe(true)
    expect(existsSync(join(assetsDir, asset.storageKey))).toBe(false)
    expect(await store.deleteDatabase(database.id)).toBe(true)
  })
})
