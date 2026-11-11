/** Reporting policy for the selected corpus. Scope never changes test selection or execution. */
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { flattenResults, isGeneratedSkip, outcome } from './report/classify.mjs'

const scopePath = new URL('./scope.json', import.meta.url)
const scopeReasons = ['network', 'handles', 'drift', 'decided-deviation']
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex')

/** A selector is a corpus-relative spec file, optionally narrowed to stable corpus test IDs. */
export function validateScope(manifest) {
  if (
    manifest?.schemaVersion !== 1 ||
    !Array.isArray(manifest.entries) ||
    !scopeReasons.every(
      (reason) =>
        typeof manifest.reasons?.[reason] === 'string' &&
        manifest.reasons[reason].trim(),
    )
  )
    throw new Error('Invalid scope schema or reasons')
  const selectors = new Map()
  for (const entry of manifest.entries) {
    if (
      !entry ||
      !scopeReasons.includes(entry.reason) ||
      typeof entry.file !== 'string' ||
      !/^tests\/(page|library)\/[^/]+\.spec\.ts$/.test(entry.file) ||
      (entry.ids !== undefined &&
        (!Array.isArray(entry.ids) ||
          !entry.ids.length ||
          entry.ids.some(
            (id) => typeof id !== 'string' || !/^[A-Z][A-Z0-9]*-\d+$/.test(id),
          )))
    )
      throw new Error('Invalid scope entry')
    const prior = selectors.get(entry.file) || new Set()
    for (const id of entry.ids || ['*']) {
      if (prior.has(id) || prior.has('*') || (id === '*' && prior.size))
        throw new Error(`Overlapping scope selectors: ${entry.file} ${id}`)
      prior.add(id)
    }
    selectors.set(entry.file, prior)
  }
  return manifest
}

export function loadScope(file = scopePath) {
  return validateScope(JSON.parse(fs.readFileSync(file, 'utf8')))
}

/** Freeze policy beside results so a later PR cannot silently change a historical score. */
export function recordScope(directory) {
  const bytes = fs.readFileSync(scopePath)
  validateScope(JSON.parse(bytes))
  fs.writeFileSync(path.join(directory, 'scope.json'), bytes)
  return { manifestSha256: digest(bytes) }
}

export function scopeForRun(run) {
  if (!run.metadata.scope) return loadScope()
  const bytes = fs.readFileSync(path.join(run.directory, 'scope.json'))
  if (digest(bytes) !== run.metadata.scope.manifestSha256)
    throw new Error('Recorded scope manifest hash differs')
  return validateScope(JSON.parse(bytes))
}

/** Whether the harness made this skip (curation's internals list, expectLoop's coverage table). */
function skippedByHarness(row) {
  return [
    ...(row.test.annotations || []),
    ...(row.test.results || []).flatMap((result) => result.annotations || []),
  ].some(
    (annotation) =>
      annotation.type === 'skip' &&
      typeof annotation.description === 'string' &&
      isGeneratedSkip(annotation.description),
  )
}

/**
 * Use Playwright outcomes, including flaky/expected failures. A skipped test ran nothing, so
 * it counts against neither rate: `skipped` reports it beside the denominator, and
 * `skippedByHarness` says how many of those the harness made rather than the spec itself.
 * Incomplete cases stay in the denominator: they started and did not pass.
 */
export function scoreResults(results, manifest = loadScope()) {
  validateScope(manifest)
  const cases = flattenResults(results, [], { allowRepeated: true })
  const all = { passed: 0, total: 0, skipped: 0, skippedByHarness: 0 }
  const inScope = { passed: 0, total: 0, skipped: 0, skippedByHarness: 0 }
  const byReason = Object.fromEntries(
    scopeReasons.map((reason) => [reason, { total: 0, passed: 0 }]),
  )
  const passingExcluded = []
  const count = (score, row, result) => {
    if (result === 'skip') {
      score.skipped++
      if (skippedByHarness(row)) score.skippedByHarness++
      return
    }
    score.total++
    if (result === 'pass') score.passed++
  }
  for (const row of cases) {
    const result = outcome(row)
    const passed = result === 'pass'
    count(all, row, result)
    const entry = manifest.entries.find(
      (entry) =>
        entry.file === row.suite &&
        (!entry.ids || entry.ids.includes(row.caseId)),
    )
    if (entry) {
      byReason[entry.reason].total++
      if (passed) {
        byReason[entry.reason].passed++
        passingExcluded.push({
          file: row.suite,
          id: row.caseId,
          title: row.title,
          reason: entry.reason,
        })
      }
    } else {
      count(inScope, row, result)
    }
  }
  return { all, inScope, byReason, passingExcluded }
}

export function scoreText(score) {
  const percent = score.total
    ? `${((100 * score.passed) / score.total).toFixed(1)}%`
    : 'n/a'
  const skipped = score.skipped
    ? `; ${score.skipped} skipped, not counted (${score.skippedByHarness} by the harness)`
    : ''
  return `${score.passed}/${score.total} (${percent})${skipped}`
}

/** The live runner and offline report share the same numerator and denominator. */
export function printScores(scores) {
  console.log(`All tests: ${scoreText(scores.all)}`)
  console.log(`In-scope tests: ${scoreText(scores.inScope)}`)
  for (const [reason, count] of Object.entries(scores.byReason))
    console.log(
      `Scope ${reason}: ${count.total} excluded; ${count.passed} passing`,
    )
  console.log(`Passing tests in scope file: ${scores.passingExcluded.length}`)
  for (const row of scores.passingExcluded)
    console.log(`  ${row.reason}: ${row.file} · ${row.title}`)
}
