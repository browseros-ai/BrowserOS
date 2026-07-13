/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { existsSync, mkdtempSync, promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, getDbHandle, initializeDb } from '../../../../src/lib/db'
import { registerWorkspaceMcpTools } from '../../../../src/api/services/mcp/register-workspace-mcp'
import { WorkspaceStore } from '../../../../src/lib/workspace/workspace-store'

type RegisteredHandler = (
  args: Record<string, unknown>,
) => Promise<{
  content: unknown
  isError?: boolean
}>

function createFakeServer() {
  const handlers = new Map<string, RegisteredHandler>()
  return {
    handlers,
    server: {
      registerTool(
        name: string,
        _config: unknown,
        handler: RegisteredHandler,
      ) {
        handlers.set(name, handler)
      },
    },
  }
}

function textOf(result: Awaited<ReturnType<RegisteredHandler>> | undefined) {
  if (!Array.isArray(result?.content)) return ''
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

describe('registerWorkspaceMcpTools', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    try {
      getDbHandle().sqlite.exec('PRAGMA wal_checkpoint(TRUNCATE)')
    } catch {
      // The database may not have initialized if setup failed.
    }
    closeDb()
    await Promise.all(
      tempDirs.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })),
    )
  })

  it('saves source-backed records and assets for the scoped research session', async () => {
    const root = mkdtempSync(join(tmpdir(), 'browseros-workspace-mcp-test-'))
    tempDirs.push(root)
    const handle = initializeDb({ dbPath: ':memory:' })
    const store = new WorkspaceStore({
      db: handle.db,
      assetsDir: join(root, 'assets'),
    })
    const collection = await store.createCollection({ name: 'Research' })
    const database = await store.createDatabase({
      name: 'Listings',
      collectionId: collection.id,
    })
    const session = await store.createResearchSession({
      conversationId: 'conversation-1',
      goal: 'Find visible listings',
      status: 'running',
      collectionId: collection.id,
      databaseId: database.id,
      plan: [
        { title: 'Collect evidence', toolCategory: 'browser' },
        { title: 'Save verified results', toolCategory: 'database' },
      ],
    })
    const fake = createFakeServer()
    registerWorkspaceMcpTools(fake.server as never, {
      store,
      conversationId: 'conversation-1',
    })

    expect(fake.handlers.has('workspace_create_record')).toBe(true)
    const sessionResult = await fake.handlers.get('research_get_session')?.({})
    expect(textOf(sessionResult)).toContain('Find visible listings')

    const sourceResult = await fake.handlers.get('workspace_create_source')?.({
      url: 'https://example.com/listing',
      title: 'Listing source',
      excerpt: 'Example item for PHP 100',
    })
    expect(sourceResult?.isError).toBeFalsy()
    const sourcePayload = JSON.parse(textOf(sourceResult)) as {
      source: { id: string }
    }

    const recordResult = await fake.handlers.get('workspace_create_record')?.({
      data: { item: 'Example item', price: 100 },
      sourceId: sourcePayload.source.id,
    })
    expect(recordResult?.isError).toBeFalsy()
    const recordPayload = JSON.parse(textOf(recordResult)) as {
      record: { id: string }
    }

    const assetResult = await fake.handlers.get('workspace_save_asset')?.({
      filename: 'evidence.txt',
      mimeType: 'text/plain',
      dataBase64: Buffer.from('evidence').toString('base64'),
      recordId: recordPayload.record.id,
      sourceId: sourcePayload.source.id,
    })
    expect(assetResult?.isError).toBeFalsy()
    const assetPayload = JSON.parse(textOf(assetResult)) as {
      asset: { storageKey: string }
    }
    expect(await fs.readFile(join(root, 'assets', assetPayload.asset.storageKey), 'utf8')).toBe('evidence')

    const stepResult = await fake.handlers.get('research_update_plan_step')?.({
      stepId: session.plan[0]?.id,
      status: 'completed',
    })
    expect(stepResult?.isError).toBeFalsy()
    const finishResult = await fake.handlers.get('research_update_status')?.({
      status: 'completed',
    })
    expect(finishResult?.isError).toBeFalsy()
    const savedSession = await store.getResearchSession(session.id)
    expect(savedSession?.status).toBe('completed')
    expect(savedSession?.plan[0]?.status).toBe('completed')
    expect(savedSession?.recap?.plan.completed).toBe(1)
    expect(savedSession?.recap?.plan.remaining).toBe(1)
    expect(existsSync(join(root, 'assets', assetPayload.asset.storageKey))).toBe(true)
  })

  it('rejects writes from an MCP scope without a research session', async () => {
    const handle = initializeDb({ dbPath: ':memory:' })
    const store = new WorkspaceStore({ db: handle.db })
    const fake = createFakeServer()
    registerWorkspaceMcpTools(fake.server as never, {
      store,
      conversationId: 'unknown-conversation',
    })

    const result = await fake.handlers.get('workspace_create_record')?.({
      databaseId: 'missing',
      data: { value: 'blocked' },
    })
    expect(result?.isError).toBe(true)
    expect(textOf(result)).toContain('No research session')
  })
})
