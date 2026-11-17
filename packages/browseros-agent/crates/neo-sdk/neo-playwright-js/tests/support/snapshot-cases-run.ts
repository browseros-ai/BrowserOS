/**
 * Runs every snapshot text case through today's renderer, and nothing else, for the coverage
 * test (snapshot-cases-coverage.test.ts) to measure. Not named `*.test.ts`, so the package's
 * runner does not pick it up on its own.
 */
import { expect, it } from 'bun:test'
import { snapshotCaseOutputs } from '../../scripts/snapshot-cases/write'

it('renders every snapshot text case', () => {
  expect(snapshotCaseOutputs().length).toBe(2)
})
