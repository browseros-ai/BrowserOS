/** Neo: Full-corpus exclusions must not be confused with a filtered run's observed verdicts. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { sha256 } from '../../js/prepare.mjs'
import { renderCuration } from './curation.mjs'

test('renders deletion/skip reasons, selected run counts and unknown verdicts', () => {
  const manifest = {
    entries: [
      {
        file: 'a',
        id: 'A-1',
        title: 'deleted',
        action: 'delete',
        reason: 'invalid-here',
      },
      {
        file: 'a',
        id: 'A-2',
        title: 'skipped',
        action: 'skip',
        reason: 'needs Playwright internals: X',
      },
      {
        file: 'b',
        id: 'B-1',
        title: 'not selected',
        action: 'skip',
        reason: 'needs Playwright internals: Y',
      },
    ],
  }
  const rows = [
    {
      suite: 'a',
      caseId: 'A-2',
      title: 'skipped',
      outcome: 'skip',
      verdict: 'skip',
    },
    {
      suite: 'a',
      caseId: 'A-3',
      title: 'run',
      outcome: 'pass',
      verdict: 'future pass class',
    },
  ]
  const text = renderCuration(
    {
      metadata: {
        curation: { enabled: true, upstreamTotal: 10, manifestSha256: 'hash' },
      },
    },
    rows,
    manifest,
  ).join('\n')
  for (const expected of [
    'Upstream total | 10',
    'Deleted: invalid-here | 1',
    'Configured skips | 2',
    'Selected | 2',
    'Skipped: needs Playwright internals: X | 1',
    'Run (selected, excluding skips) | 1',
    'Verdict: future pass class | 1',
  ])
    assert.ok(text.includes(expected), expected)
  assert.ok(!text.includes('| Skipped: needs Playwright internals: Y'))
})

test('old runs and deliberately uncurated runs are distinguished', () => {
  assert.deepEqual(renderCuration({ metadata: {} }, []), [])
  assert.match(
    renderCuration({ metadata: { curation: { enabled: false } } }, []).join(
      '\n',
    ),
    /Disabled/,
  )
})

test('the report uses the recorded manifest and rejects a changed snapshot', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'curation-report-'))
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  const bytes = JSON.stringify({ entries: [] })
  const file = path.join(directory, 'curation.json')
  fs.writeFileSync(file, bytes)
  const run = {
    directory,
    metadata: {
      curation: {
        enabled: true,
        upstreamTotal: 3,
        manifestSha256: sha256(bytes),
      },
    },
  }
  assert.match(renderCuration(run, []).join('\n'), /Upstream total \| 3/)
  fs.writeFileSync(file, `${bytes}\n`)
  assert.throws(() => renderCuration(run, []), /hash differs/)
})

test('uncurated skips report their upstream annotation reason', () => {
  const run = {
    metadata: { curation: { enabled: false, upstreamTotal: 1 } },
    results: {
      suites: [
        {
          title: 'page/a.spec.ts',
          file: 'page/a.spec.ts',
          specs: [
            {
              title: 'A-001 skipped',
              tests: [
                {
                  annotations: [
                    {
                      type: 'fixme',
                      description: 'upstream platform limitation',
                    },
                  ],
                },
              ],
            },
          ],
        },
      ],
    },
  }
  const text = renderCuration(run, [
    {
      suite: 'tests/page/a.spec.ts',
      caseId: 'A-001',
      title: 'A-001 skipped',
      outcome: 'skip',
      verdict: 'skip',
    },
  ]).join('\n')
  assert.ok(text.includes('Skipped: upstream platform limitation | 1'))
  assert.ok(text.includes('Run (selected, excluding skips) | 0'))
})
