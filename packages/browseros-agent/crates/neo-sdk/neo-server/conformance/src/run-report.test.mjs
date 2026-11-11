import { expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { persistState, writeReport } from './run-report.mjs'

test('rejudge retains a missing final pair and a failure after otherwise valid pairs', () => {
  const dir = mkdtempSync(join(tmpdir(), 'conformance-report-'))
  const json = (file, value) =>
    writeFileSync(join(dir, file), JSON.stringify(value))
  try {
    json('inventory.json', {
      tests: [{ id: 'x', selected: true, class: 'runnable' }],
    })
    for (const side of ['native', 'neo']) {
      mkdirSync(join(dir, side))
      json(`${side}/first.json`, {
        id: 'x',
        attempt: 1,
        status: 'passed',
        expectedStatus: 'passed',
        assertions: { called: 1 },
        provenance: {
          side,
          browserVersion: 'browser',
          programSha256: 'same',
          playwrightCore: '1.64.1',
          neo: {
            runId: 'run',
            binarySha256: 'bin',
            gitSha: 'sha',
            opCount: 1,
            traceVerified: true,
          },
        },
      })
    }
    const state = {
      schema: 1,
      mode: 'run',
      side: 'both',
      phase: 'suite',
      status: 'completed',
      repeat: 2,
    }
    persistState(dir, state)
    const incomplete = writeReport(dir, {})
    expect(incomplete.ok).toBe(false)
    expect(incomplete.results.map((result) => result.classification)).toEqual([
      'pass',
      'harness-error',
    ])
    persistState(dir, {
      ...state,
      repeat: 1,
      status: 'interrupted',
      failure: 'interrupted by SIGTERM',
    })
    const interrupted = writeReport(dir, {})
    expect(interrupted.ok).toBe(false)
    expect(interrupted.failure).toBe('interrupted by SIGTERM')
    expect(interrupted.results[0].classification).toBe('pass')
    persistState(dir, { ...state, repeat: 1 })
    json('failure.json', { error: 'suite budget exhausted' })
    expect(writeReport(dir, {})).toMatchObject({
      ok: false,
      failure: 'Error: suite budget exhausted',
    })
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
