import type { CaseResult } from './model'

export type Partition = 'ordinary' | 'density' | 'terminal-select'

/** Runtime evidence distinguishes actual expanded execution from shared source-body reuse. */
export interface PartitionReceipt {
  partition: Partition
  startedAt: number
  startupMs: number
  casesMs: number
  caseCleanupMs: number
  cleanupMs: number
  finishedAt: number
  /** Parent finalization follows launcher cleanup; finishedAt remains the child's completion. */
  finalizedAt?: number
  expectedIds: readonly string[]
  results: readonly CaseResult[]
  complete: boolean
  failure?: string
}

export function coverageError(
  expectedIds: readonly string[],
  results: readonly CaseResult[],
): string | undefined {
  const expected = new Set(expectedIds)
  const actual = new Set(results.map((result) => result.id))
  const duplicates = results.filter(
    (result, index) =>
      results.findIndex((item) => item.id === result.id) !== index,
  )
  const missing = [...expected].filter((id) => !actual.has(id))
  const unexpected = [...actual].filter((id) => !expected.has(id))
  if (
    expected.size !== expectedIds.length ||
    duplicates.length ||
    missing.length ||
    unexpected.length
  ) {
    return `Coverage mismatch: missing=${missing.join(',')} duplicate=${duplicates.map((result) => result.id).join(',')} unexpected=${unexpected.join(',')}`
  }
  return undefined
}

function escapeXml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&apos;')
}

/** Xfails remain visible as named skipped assertions; their strict matcher already ran. */
export function junit(receipt: PartitionReceipt): string {
  const globalFailures = receipt.failure ? 1 : 0
  const failures =
    receipt.results.filter((result) => result.status === 'fail').length +
    globalFailures
  const skipped = receipt.results.filter(
    (result) => result.status === 'xfail',
  ).length
  const cases = receipt.results.map((result) => {
    const opening = `<testcase classname="${escapeXml(result.file)}" name="${escapeXml(`${result.id} ${result.name}`)}" time="${((result.durationMs + result.cleanupMs) / 1000).toFixed(3)}">`
    if (result.status === 'fail')
      return `${opening}<failure message="${escapeXml(result.failure)}"/></testcase>`
    if (result.status === 'xfail')
      return `${opening}<skipped type="xfail" message="${escapeXml(result.reason)}"/></testcase>`
    return `${opening}</testcase>`
  })
  if (receipt.failure) {
    cases.push(
      `<testcase classname="sdk-desktop-fixtures" name="suite lifecycle and coverage"><failure message="${escapeXml(receipt.failure)}"/></testcase>`,
    )
  }
  return `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites><testsuite name="sdk-desktop-fixtures-${receipt.partition}" tests="${receipt.results.length + globalFailures}" failures="${failures}" skipped="${skipped}" time="${((receipt.startupMs + receipt.casesMs + receipt.cleanupMs) / 1000).toFixed(3)}">\n${cases.join('\n')}\n</testsuite></testsuites>\n`
}
