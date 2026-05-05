import { describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rename, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  contentTypeForPath,
  R2Publisher,
} from '../../src/publishing/r2-publisher'

class FakeR2Client {
  readonly puts: Record<string, unknown>[] = []
  readonly existing = new Set<string>()

  async send(command: { input: Record<string, unknown> }): Promise<unknown> {
    const key = command.input.Key as string
    if ('Body' in command.input) {
      this.puts.push(command.input)
      return {}
    }
    if (this.existing.has(key)) return {}
    throw new Error('not found')
  }
}

async function writeRunFixture(
  root: string,
  configName = 'browseros-agent-weekly',
  timestamp = '2026-04-29-1200',
  options: { queryId?: string } = {},
): Promise<{ runDir: string; runId: string }> {
  const runDir = join(root, configName, timestamp)
  const taskDir = join(runDir, 'task-1')
  await mkdir(join(taskDir, 'screenshots'), { recursive: true })
  await writeFile(
    join(taskDir, 'metadata.json'),
    JSON.stringify({
      query_id: options.queryId ?? 'task-1',
      dataset: 'webbench',
      query: 'Find pricing',
      start_url: 'https://example.test',
      termination_reason: 'completed',
      total_duration_ms: 1200,
      total_steps: 4,
      screenshot_count: 1,
      agent_config: { type: 'single', model: 'kimi' },
      grader_results: {
        performance_grader: { score: 1, pass: true, reasoning: 'ok' },
      },
    }),
  )
  await writeFile(
    join(taskDir, 'messages.jsonl'),
    [
      '{"type":"user"}',
      '{"type":"tool-input-available","toolName":"click"}',
      '{"type":"tool-input-available","toolName":"take_snapshot"}',
      '{"type":"tool-output-error","toolName":"click"}',
    ].join('\n'),
  )
  await writeFile(join(taskDir, 'grades.json'), '{"ok":true}')
  await writeFile(join(taskDir, 'screenshots', '1.png'), 'png')
  await writeFile(
    join(runDir, 'summary.json'),
    JSON.stringify({ passRate: 1, avgDurationMs: 1200 }),
  )
  await writeFile(join(runDir, 'report.html'), '<html>report</html>')
  return { runDir, runId: `${configName}-${timestamp}` }
}

describe('R2Publisher', () => {
  it('maps artifact file extensions to viewer-compatible content types', () => {
    expect(contentTypeForPath('metadata.json')).toBe('application/json')
    expect(contentTypeForPath('messages.jsonl')).toBe('application/x-ndjson')
    expect(contentTypeForPath('screenshots/1.png')).toBe('image/png')
    expect(contentTypeForPath('viewer.html')).toBe('text/html')
  })

  it('uploads run artifacts, manifest, and viewer html', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'eval-r2-'))
    const { runDir, runId } = await writeRunFixture(dir)
    const viewerPath = join(dir, 'viewer.html')
    await writeFile(viewerPath, '<html>viewer</html>')
    const client = new FakeR2Client()

    const result = await new R2Publisher({
      client,
      viewerPath,
      config: {
        accountId: 'acct',
        accessKeyId: 'key',
        secretAccessKey: 'secret',
        bucket: 'bucket',
        cdnBaseUrl: 'https://eval.example.test',
      },
      now: () => new Date('2026-04-29T12:00:00.000Z'),
    }).publishRun(runDir, runId)

    const byKey = new Map(client.puts.map((put) => [put.Key, put]))
    expect(byKey.get(`runs/${runId}/task-1/metadata.json`)?.ContentType).toBe(
      'application/json',
    )
    expect(byKey.get(`runs/${runId}/task-1/messages.jsonl`)?.ContentType).toBe(
      'application/x-ndjson',
    )
    expect(
      byKey.get(`runs/${runId}/task-1/screenshots/1.png`)?.ContentType,
    ).toBe('image/png')
    expect(
      byKey.get(`runs/${runId}/tasks/task-1/metadata.json`)?.ContentType,
    ).toBe('application/json')
    expect(
      byKey.get(`runs/${runId}/tasks/task-1/messages.jsonl`)?.ContentType,
    ).toBe('application/x-ndjson')
    expect(
      byKey.get(`runs/${runId}/tasks/task-1/screenshots/1.png`)?.ContentType,
    ).toBe('image/png')
    expect(byKey.get(`runs/${runId}/manifest.json`)?.ContentType).toBe(
      'application/json',
    )
    expect(byKey.get(`runs/${runId}/summary.json`)?.ContentType).toBe(
      'application/json',
    )
    expect(byKey.get(`runs/${runId}/report.html`)?.ContentType).toBe(
      'text/html',
    )
    expect(byKey.get('viewer.html')?.ContentType).toBe('text/html')
    expect(result.viewerUrl).toBe(
      `https://eval.example.test/viewer.html?run=${runId}`,
    )

    const manifest = JSON.parse(
      Buffer.from(
        byKey.get(`runs/${runId}/manifest.json`)?.Body as Buffer,
      ).toString('utf-8'),
    )
    expect(manifest).toMatchObject({
      schemaVersion: 2,
      runId,
      uploadedAt: '2026-04-29T12:00:00.000Z',
      agentConfig: { type: 'single', model: 'kimi' },
      dataset: 'webbench',
      reportPath: 'report.html',
      summary: { passRate: 1, avgDurationMs: 1200 },
      metrics: {
        taskCount: 1,
        avgDurationMs: 1200,
        avgSteps: 4,
        avgToolCalls: 2,
        totalToolCalls: 2,
        totalToolErrors: 1,
      },
      tasks: [
        {
          queryId: 'task-1',
          status: 'completed',
          screenshotCount: 1,
          metrics: {
            durationMs: 1200,
            steps: 4,
            screenshots: 1,
            toolCalls: 2,
            toolErrors: 1,
          },
          paths: {
            attempt: 'tasks/task-1/attempt.json',
            metadata: 'tasks/task-1/metadata.json',
            messages: 'tasks/task-1/messages.jsonl',
            trace: 'tasks/task-1/trace.jsonl',
            grades: 'tasks/task-1/grades.json',
            screenshots: 'tasks/task-1/screenshots',
            graderArtifacts: 'tasks/task-1/grader-artifacts',
          },
        },
      ],
    })
  })

  it('uses task directory ids for canonical paths when metadata query ids differ', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'eval-r2-path-id-'))
    const { runDir, runId } = await writeRunFixture(
      dir,
      'weekly',
      '2026-04-29-1200',
      { queryId: 'query-id-from-metadata' },
    )
    const viewerPath = join(dir, 'viewer.html')
    await writeFile(viewerPath, '<html>viewer</html>')
    const client = new FakeR2Client()

    await new R2Publisher({
      client,
      viewerPath,
      config: {
        accountId: 'acct',
        accessKeyId: 'key',
        secretAccessKey: 'secret',
        bucket: 'bucket',
        cdnBaseUrl: 'https://eval.example.test',
      },
      now: () => new Date('2026-04-29T12:00:00.000Z'),
    }).publishRun(runDir, runId)

    const byKey = new Map(client.puts.map((put) => [put.Key, put]))
    const manifest = JSON.parse(
      Buffer.from(
        byKey.get(`runs/${runId}/manifest.json`)?.Body as Buffer,
      ).toString('utf-8'),
    )

    expect(byKey.has(`runs/${runId}/tasks/task-1/metadata.json`)).toBe(true)
    expect(manifest.tasks[0]).toMatchObject({
      queryId: 'query-id-from-metadata',
      paths: {
        metadata: 'tasks/task-1/metadata.json',
        screenshots: 'tasks/task-1/screenshots',
      },
    })
  })

  it('encodes run ids in returned viewer urls', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'eval-r2-viewer-url-'))
    const { runDir } = await writeRunFixture(dir)
    const viewerPath = join(dir, 'viewer.html')
    await writeFile(viewerPath, '<html>viewer</html>')
    const client = new FakeR2Client()

    const result = await new R2Publisher({
      client,
      viewerPath,
      config: {
        accountId: 'acct',
        accessKeyId: 'key',
        secretAccessKey: 'secret',
        bucket: 'bucket',
        cdnBaseUrl: 'https://eval.example.test',
      },
    }).publishRun(runDir, 'run with spaces')

    expect(result.viewerUrl).toBe(
      'https://eval.example.test/viewer.html?run=run%20with%20spaces',
    )
  })

  it('publishes unuploaded runs from a config results directory', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'eval-r2-config-'))
    const first = await writeRunFixture(dir, 'weekly', '2026-04-29-1200')
    const second = await writeRunFixture(dir, 'weekly', '2026-04-30-1200')
    const viewerPath = join(dir, 'viewer.html')
    await writeFile(viewerPath, '<html>viewer</html>')
    const client = new FakeR2Client()
    client.existing.add(`runs/${first.runId}/manifest.json`)

    const result = await new R2Publisher({
      client,
      viewerPath,
      config: {
        accountId: 'acct',
        accessKeyId: 'key',
        secretAccessKey: 'secret',
        bucket: 'bucket',
        cdnBaseUrl: 'https://eval.example.test',
      },
      now: () => new Date('2026-04-29T12:00:00.000Z'),
    }).publishPath(join(dir, 'weekly'))

    expect(result.uploadedRuns.map((run) => run.runId)).toEqual([second.runId])
    expect(
      client.puts.some(
        (put) => put.Key === `runs/${first.runId}/manifest.json`,
      ),
    ).toBe(false)
    expect(
      client.puts.some(
        (put) => put.Key === `runs/${second.runId}/manifest.json`,
      ),
    ).toBe(true)

    await expect(
      readFile(join(second.runDir, 'summary.json'), 'utf-8'),
    ).resolves.toContain('passRate')
  })

  it('recognizes and publishes canonical tasks directory runs', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'eval-r2-tasks-'))
    const { runDir, runId } = await writeRunFixture(dir)
    await mkdir(join(runDir, 'tasks'), { recursive: true })
    await rename(join(runDir, 'task-1'), join(runDir, 'tasks', 'task-1'))
    const viewerPath = join(dir, 'viewer.html')
    await writeFile(viewerPath, '<html>viewer</html>')
    const client = new FakeR2Client()

    const result = await new R2Publisher({
      client,
      viewerPath,
      config: {
        accountId: 'acct',
        accessKeyId: 'key',
        secretAccessKey: 'secret',
        bucket: 'bucket',
        cdnBaseUrl: 'https://eval.example.test',
      },
    }).publishPath(runDir)

    const keys = client.puts.map((put) => put.Key)
    const byKey = new Map(client.puts.map((put) => [put.Key, put]))
    const manifest = JSON.parse(
      Buffer.from(
        byKey.get(`runs/${runId}/manifest.json`)?.Body as Buffer,
      ).toString('utf-8'),
    )

    expect(result.uploadedRuns.map((run) => run.runId)).toEqual([runId])
    expect(keys).toContain(`runs/${runId}/task-1/metadata.json`)
    expect(keys).toContain(`runs/${runId}/tasks/task-1/metadata.json`)
    expect(manifest).toMatchObject({
      schemaVersion: 2,
      tasks: [
        {
          queryId: 'task-1',
          paths: {
            metadata: 'tasks/task-1/metadata.json',
            screenshots: 'tasks/task-1/screenshots',
          },
        },
      ],
    })
  })
})
