import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { sessionTabs } from '../../src/modules/db/schema/session-tabs.sql'
import {
  createRecordingStore,
  type RecordingStore,
} from '../../src/services/recordings'
import { createReplayService } from '../../src/services/replays'

const firstDocument = '33D25F3CF060E81B14070BC356FF1871'
const secondDocument = '8395FF2EF4A1D8579F1917B3B54ADECE'
const otherDocument = '9E84CDCAB8762569B5B109D125F60147'
let dir: string
let store: RecordingStore

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'replays-'))
  setAuditDbForTesting()
  store = createRecordingStore({ rootDir: dir })
})

afterEach(async () => {
  await store.resetForTesting()
  resetAuditDbForTesting()
  await rm(dir, { recursive: true, force: true })
})

function append(
  documentId: string,
  tabId: number,
  targetId: string | null,
  timestamps: number[],
  hasGap = false,
) {
  return store.appendBatch({
    documentId,
    tabId,
    targetId,
    events: timestamps.map((ts) => ({ ts, type: 3, data: { ts } })),
    batchId: `batch-${documentId}`,
    hasGap,
  })
}

describe('ReplayService', () => {
  it('joins by tab across target changes and filters exact event windows', async () => {
    await append(firstDocument, 11, 'target-before-nav', [90, 100, 150])
    await append(secondDocument, 11, 'target-after-nav', [160, 200, 201])
    await append(otherDocument, 22, null, [170])
    getAuditDb()
      .insert(sessionTabs)
      .values({
        sessionId: 'session-a',
        agentId: 'agent-a',
        tabId: 11,
        openedTargetId: 'target-before-nav',
        claimedAt: 100,
        releasedAt: 200,
      })
      .run()
    const service = createReplayService({ recordingStore: store })

    const replay = await service.readSession('session-a')
    expect(replay.map((event) => event.ts)).toEqual([100, 150, 160, 200])
    expect(replay.map((event) => event.documentId)).toEqual([
      firstDocument,
      firstDocument,
      secondDocument,
      secondDocument,
    ])
    expect(replay.every((event) => event.tabId === 11)).toBe(true)
  })

  it('groups navigation segments under one logical tab and reports gaps', async () => {
    await append(firstDocument, 11, 'target-before-nav', [100, 150])
    await append(secondDocument, 11, 'target-after-nav', [160, 200], true)
    getAuditDb()
      .insert(sessionTabs)
      .values({
        sessionId: 'session-a',
        agentId: 'agent-a',
        tabId: 11,
        openedTargetId: 'target-before-nav',
        claimedAt: 100,
        releasedAt: 200,
      })
      .run()
    const service = createReplayService({ recordingStore: store })

    expect(service.getMeta('session-a')).toMatchObject({
      exists: true,
      complete: false,
      firstEventAt: 100,
      lastEventAt: 200,
      tabs: [
        {
          tabId: 11,
          complete: false,
          segments: [
            { documentId: firstDocument, hasGap: false, legacy: false },
            { documentId: secondDocument, hasGap: true, legacy: false },
          ],
        },
      ],
    })
  })

  it('returns empty complete metadata for a session without ownership', async () => {
    const service = createReplayService({ recordingStore: store })
    expect(await service.readSession('missing')).toEqual([])
    expect(service.getMeta('missing')).toEqual({
      exists: false,
      complete: true,
      sizeBytes: 0,
      tabs: [],
    })
  })
})
