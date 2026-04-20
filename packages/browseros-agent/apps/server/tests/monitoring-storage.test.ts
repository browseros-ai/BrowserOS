import { afterEach, describe, expect, it } from 'bun:test'
import { appendFile, mkdir, rm } from 'node:fs/promises'
import {
  getLazyMonitoringRunDir,
  getLazyMonitoringRunsDir,
} from '../src/lib/browseros-dir'
import {
  InvalidMonitoringRunIdError,
  isValidMonitoringRunId,
  MonitoringStorage,
} from '../src/monitoring/storage'

const createdRunDirs = new Set<string>()

afterEach(async () => {
  await Promise.all(
    [...createdRunDirs].map(async (runId) => {
      await rm(getLazyMonitoringRunDir(runId), { recursive: true, force: true })
    }),
  )
  createdRunDirs.clear()
})

describe('MonitoringStorage run id validation', () => {
  it('accepts UUID monitoring run ids', () => {
    expect(isValidMonitoringRunId('123e4567-e89b-12d3-a456-426614174000')).toBe(
      true,
    )
  })

  it('rejects path traversal run ids', async () => {
    expect(isValidMonitoringRunId('../../secret')).toBe(false)

    const storage = new MonitoringStorage()
    await expect(storage.readContext('../../secret')).rejects.toBeInstanceOf(
      InvalidMonitoringRunIdError,
    )
  })

  it('preserves valid JSONL records when one line is malformed', async () => {
    const runId = '123e4567-e89b-12d3-a456-426614174001'
    createdRunDirs.add(runId)

    const storage = new MonitoringStorage()
    await storage.writeContext({
      monitoringSessionId: runId,
      agentId: 'test-agent',
      sessionKey: 'session-1',
      originalPrompt: 'Inspect browser state safely',
      chatHistory: [{ role: 'user', content: 'Inspect browser state safely' }],
      startedAt: new Date().toISOString(),
      source: 'debug',
    })

    await appendFile(
      `${getLazyMonitoringRunDir(runId)}/tool-calls.jsonl`,
      [
        JSON.stringify({
          monitoringSessionId: runId,
          agentId: 'test-agent',
          toolCallId: 'tool-1',
          toolName: 'list_windows',
          source: 'browser-tool',
          args: {},
          startedAt: '2026-04-20T15:22:49.817Z',
          finishedAt: '2026-04-20T15:22:49.818Z',
          durationMs: 1,
        }),
        '{"broken":',
        JSON.stringify({
          monitoringSessionId: runId,
          agentId: 'test-agent',
          toolCallId: 'tool-2',
          toolName: 'take_snapshot',
          source: 'browser-tool',
          args: {},
          startedAt: '2026-04-20T15:22:50.817Z',
          finishedAt: '2026-04-20T15:22:50.818Z',
          durationMs: 1,
        }),
        '',
      ].join('\n'),
    )

    const toolCalls = await storage.readToolCalls(runId)
    expect(toolCalls).toHaveLength(2)
    expect(toolCalls.map((record) => record.toolCallId)).toEqual([
      'tool-1',
      'tool-2',
    ])
  })

  it('skips non-uuid directories when listing run ids', async () => {
    const validRunId = '123e4567-e89b-12d3-a456-426614174002'
    createdRunDirs.add(validRunId)

    await mkdir(getLazyMonitoringRunsDir(), { recursive: true })
    await mkdir(getLazyMonitoringRunDir(validRunId), { recursive: true })
    await mkdir(`${getLazyMonitoringRunsDir()}/not-a-uuid`, {
      recursive: true,
    })

    const storage = new MonitoringStorage()
    const runIds = await storage.listRunIds()

    expect(runIds).toContain(validRunId)
    expect(runIds).not.toContain('not-a-uuid')

    await rm(`${getLazyMonitoringRunsDir()}/not-a-uuid`, {
      recursive: true,
      force: true,
    })
  })
})
