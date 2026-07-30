export interface CargoTestCounts {
  passed: number
  failed: number
  ignored: number
  total: number
}

/**
 * Sums libtest's per-binary summary lines from `cargo test` output, e.g.
 * `test result: ok. 12 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out`.
 * One appears per test binary and one per doctest run, so summing them gives the
 * whole-workspace totals. cargo emits no JUnit natively; this lets the workflow
 * report real counts while still running doctests (which cargo-nextest skips).
 */
export function parseCargoTestCounts(output: string): CargoTestCounts {
  let passed = 0
  let failed = 0
  let ignored = 0

  const re =
    /test result:\s+\w+\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored/g
  for (const match of output.matchAll(re)) {
    passed += Number(match[1])
    failed += Number(match[2])
    ignored += Number(match[3])
  }

  return { passed, failed, ignored, total: passed + failed + ignored }
}

/**
 * Reconciles parsed counts with cargo's exit code so the JUnit never reports a
 * failed run as passing. A non-zero exit means the suite failed even when
 * libtest printed no failing test (a compile or setup error, possibly in a later
 * crate after earlier tests passed), so force at least one failure. Otherwise the
 * summary, which treats zero tests and zero failures as a passing gate, would
 * render a red job as green.
 */
export function reconcileCargoCounts(
  counts: CargoTestCounts,
  exitCode: number,
): CargoTestCounts {
  if (exitCode === 0) return counts
  const failed = Math.max(counts.failed, 1)
  return { ...counts, failed, total: Math.max(counts.total, failed) }
}

/** Builds a JUnit document the PR test summary can parse from cargo counts. */
export function buildCargoJunitXml(
  suite: string,
  counts: CargoTestCounts,
): string {
  const { total, failed, ignored } = counts
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    `<testsuites tests="${total}" failures="${failed}">`,
    `  <testsuite name="${suite}" tests="${total}" failures="${failed}" errors="0" skipped="${ignored}">`,
    '  </testsuite>',
    '</testsuites>',
    '',
  ].join('\n')
}
