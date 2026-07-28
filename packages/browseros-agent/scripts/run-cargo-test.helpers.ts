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
