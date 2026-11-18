/**
 * Negative tests for explicit Neo design decisions. Scope removes these cases only from the
 * positive score; CI still judges their original assertions from the run's raw report and
 * copied manifest. Legacy exclusions without a decision remain reporting-only.
 */
import { flattenResults, outcome } from './report/classify.mjs'
import { excludedBy, validateScope } from './scope.mjs'

const selectorKey = (file, id) => JSON.stringify([file, id])

/**
 * A decided case must actually fail, not pass (even once), skip or disappear. A filtered run
 * judges only its selection; a complete run must account for every decision selector so
 * deleting/skipping a negative test cannot silently satisfy the policy.
 */
export function judgeDecisions(report, scope, { complete }) {
  validateScope(scope)
  const missing = new Map()
  for (const entry of scope.entries) {
    if (!entry.decision) continue
    for (const id of entry.ids ?? [null])
      missing.set(selectorKey(entry.file, id), {
        id,
        file: entry.file,
        title: null,
        decision: entry.decision,
        outcome: 'missing',
      })
  }
  const violations = []
  let total = 0
  let failed = 0
  for (const row of flattenResults(report)) {
    const entry = excludedBy(scope, row)
    if (!entry?.decision) continue
    missing.delete(selectorKey(row.suite, entry.ids ? row.caseId : null))
    total++
    // A successful attempt is a design violation even if a retry or expected-failure
    // annotation makes Playwright classify the overall test differently.
    const result = row.test.results?.some(
      (attempt) => attempt.status === 'passed',
    )
      ? 'pass'
      : outcome(row)
    if (result === 'fail') failed++
    else
      violations.push({
        id: row.caseId,
        file: row.suite,
        title: row.title,
        decision: entry.decision,
        outcome: result,
      })
  }
  if (complete) {
    total += missing.size
    violations.push(...missing.values())
  }
  return { total, failed, violations }
}

/** These errors also invalidate list-update proofs: --write-expected cannot waive a decision. */
export function decisionErrors(decisions) {
  return decisions.violations.map((test) => {
    const name = `${test.id ?? '-'} ${test.file}${test.title ? ` › ${test.title}` : ''}`
    return test.outcome === 'pass'
      ? `${name} passed although decision "${test.decision}" says it cannot: the design changed, or this entry is wrong`
      : `${name} did not fail (${test.outcome}) although decision "${test.decision}" requires it: a negative test must run and fail`
  })
}
