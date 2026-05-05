import { describe, expect, it } from 'bun:test'
import type { R2RunManifest } from '../../src/publishing/r2-manifest'
import { buildViewerManifest } from '../../src/viewer/viewer-manifest'

describe('buildViewerManifest', () => {
  it('indexes task artifacts for the R2 viewer', () => {
    const manifest = buildViewerManifest({
      runId: 'run-1',
      suiteId: 'agisdk-daily-10',
      variantId: 'kimi',
      uploadedAt: '2026-04-29T06:00:00.000Z',
      reportPath: 'report.html',
      summary: { total: 1, passRate: 0 },
      tasks: [
        {
          queryId: 'agisdk-dashdish-4',
          query: 'Schedule a delivery order',
          startUrl: 'https://evals-dashdish.vercel.app',
          status: 'completed',
          durationMs: 353_000,
          screenshotCount: 42,
          metrics: {
            durationMs: 353_000,
            steps: 47,
            screenshots: 42,
            toolCalls: 19,
            toolErrors: 2,
          },
          graderResults: {
            agisdk_state_diff: {
              score: 0,
              pass: false,
              reasoning: 'Missing checkout item',
              details: { missing: ['checkout item'] },
            },
          },
        },
      ],
    })

    const publishManifest: R2RunManifest = manifest
    expect(publishManifest.schemaVersion).toBe(2)
    expect(manifest.reportPath).toBe('report.html')
    expect(manifest.tasks[0].paths.messages).toBe(
      'tasks/agisdk-dashdish-4/messages.jsonl',
    )
    expect(manifest.tasks[0].paths.screenshots).toBe(
      'tasks/agisdk-dashdish-4/screenshots',
    )
    expect(manifest.tasks[0].paths.graderArtifacts).toBe(
      'tasks/agisdk-dashdish-4/grader-artifacts',
    )
    expect(manifest.metrics).toMatchObject({
      taskCount: 1,
      avgDurationMs: 353_000,
      avgSteps: 47,
      avgToolCalls: 19,
      totalToolCalls: 19,
      totalToolErrors: 2,
    })
    expect(manifest.tasks[0].metrics).toEqual({
      durationMs: 353_000,
      steps: 47,
      screenshots: 42,
      toolCalls: 19,
      toolErrors: 2,
    })
    expect(manifest.tasks[0].graderResults.agisdk_state_diff.details).toEqual({
      missing: ['checkout item'],
    })
  })

  it('builds stable paths when optional task fields are missing', () => {
    const manifest = buildViewerManifest({
      runId: 'run-2',
      uploadedAt: '2026-04-29T06:00:00.000Z',
      tasks: [
        {
          queryId: 'task-with-minimal-fields',
          query: 'Do the task',
          status: 'completed',
          durationMs: 10,
          screenshotCount: 0,
          graderResults: {},
        },
      ],
    })

    expect(manifest).toMatchObject({
      schemaVersion: 2,
      runId: 'run-2',
      uploadedAt: '2026-04-29T06:00:00.000Z',
      tasks: [
        {
          queryId: 'task-with-minimal-fields',
          startUrl: '',
          paths: {
            attempt: 'tasks/task-with-minimal-fields/attempt.json',
            metadata: 'tasks/task-with-minimal-fields/metadata.json',
            messages: 'tasks/task-with-minimal-fields/messages.jsonl',
            trace: 'tasks/task-with-minimal-fields/trace.jsonl',
            grades: 'tasks/task-with-minimal-fields/grades.json',
            screenshots: 'tasks/task-with-minimal-fields/screenshots',
            graderArtifacts: 'tasks/task-with-minimal-fields/grader-artifacts',
          },
        },
      ],
    })
  })

  it('can separate display query ids from artifact path ids', () => {
    const manifest = buildViewerManifest({
      runId: 'run-3',
      tasks: [
        {
          queryId: 'metadata-query-id',
          artifactId: 'task-dir-id',
          query: 'Do the task',
          status: 'completed',
          durationMs: 10,
          screenshotCount: 0,
          graderResults: {},
        },
      ],
    })

    expect(manifest.tasks[0]).toMatchObject({
      queryId: 'metadata-query-id',
      paths: {
        metadata: 'tasks/task-dir-id/metadata.json',
        screenshots: 'tasks/task-dir-id/screenshots',
      },
    })
    expect('artifactId' in manifest.tasks[0]).toBe(false)
  })
})
