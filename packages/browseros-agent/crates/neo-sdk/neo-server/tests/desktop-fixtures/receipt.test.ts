import { expect, test } from 'bun:test'
import type { CaseResult } from './model'
import { coverageError, junit, type PartitionReceipt } from './receipt'

const passed = {
  id: 'case-001',
  file: 'cases-act.ts',
  name: 'Case <one>',
  status: 'pass',
  durationMs: 10,
  cleanupMs: 2,
  runIds: [],
} satisfies CaseResult

test('coverage rejects missing, repeated and unexpected expanded IDs', () => {
  expect(coverageError(['case-001'], [passed])).toBeUndefined()
  expect(coverageError(['case-001', 'case-002'], [passed])).toContain(
    'missing=case-002',
  )
  expect(coverageError(['case-001'], [passed, passed])).toContain(
    'duplicate=case-001',
  )
  expect(coverageError([], [passed])).toContain('unexpected=case-001')
})

test('JUnit preserves desktop origin, strict xfail reason and lifecycle failure', () => {
  const receipt: PartitionReceipt = {
    partition: 'ordinary',
    startedAt: 0,
    startupMs: 0,
    casesMs: 10,
    caseCleanupMs: 2,
    cleanupMs: 2,
    finishedAt: 0,
    expectedIds: ['case-001'],
    complete: false,
    results: [{ ...passed, status: 'xfail', reason: 'Known & narrow' }],
    failure: 'SDK cleanup failed',
  }
  const xml = junit(receipt)
  expect(xml).toContain(
    'classname="cases-act.ts" name="case-001 Case &lt;one&gt;"',
  )
  expect(xml).toContain('type="xfail" message="Known &amp; narrow"')
  expect(xml).toContain('name="suite lifecycle and coverage"')
  expect(xml).toContain('failures="1" skipped="1"')
})
