/** Neo: Explains corpus exclusions separately from verdicts for the cases actually selected. */
import fs from 'node:fs'
import path from 'node:path'
import { sha256 } from '../../js/prepare.mjs'
import { entryKey, summarize } from '../curation.mjs'
import { flattenResults } from './classify.mjs'

const cell = (value) =>
  String(value).replaceAll('|', '&#124;').replaceAll('\n', '<br>')
const table = (rows) => [
  '| Curation | Tests |',
  '| --- | ---: |',
  ...rows.map(([name, count]) => `| ${cell(name)} | ${count} |`),
]

export function renderCuration(run, rows, suppliedManifest) {
  const metadata = run.metadata.curation
  if (!metadata) return []
  let manifest = metadata.enabled ? suppliedManifest : { entries: [] }
  if (!manifest) {
    const bytes = fs.readFileSync(path.join(run.directory, 'curation.json'))
    if (sha256(bytes) !== metadata.manifestSha256)
      throw new Error('Recorded curation manifest hash differs')
    manifest = JSON.parse(bytes)
  }
  const counts = summarize(manifest.entries)
  const entries = new Map(
    manifest.entries.map((entry) => [entryKey(entry), entry]),
  )
  const skips = {}
  const verdictCounts = {}
  const annotations = new Map(
    (run.results ? flattenResults(run.results) : []).map((row) => [
      entryKey({ file: row.suite, id: row.caseId, title: row.title }),
      row.test.annotations || [],
    ]),
  )
  for (const row of rows) {
    verdictCounts[row.verdict] = (verdictCounts[row.verdict] || 0) + 1
    if (row.outcome === 'skip' || row.verdict === 'skip') {
      const key = entryKey({
        file: row.suite,
        id: row.caseId,
        title: row.title,
      })
      const entry = entries.get(key)
      const reason =
        entry?.action === 'skip'
          ? entry.reason
          : annotations
              .get(key)
              ?.filter((annotation) =>
                ['skip', 'fixme'].includes(annotation.type),
              )
              .map((annotation) => annotation.description || annotation.type)
              .join('; ') ||
            row.reason ||
            'upstream skip'
      skips[reason] = (skips[reason] || 0) + 1
    }
  }
  const skipped = Object.values(skips).reduce((a, b) => a + b, 0)
  return [
    '## Curation',
    '',
    metadata.enabled
      ? `Manifest SHA-256: ${metadata.manifestSha256}. Deletions and configured skips cover the full corpus; selected/run counts respect this invocation's filters.`
      : 'Disabled (`--no-curation`); upstream selection is unchanged.',
    '',
    ...table([
      ['Upstream total', metadata.upstreamTotal],
      ['Deleted', counts.delete.total],
      ...Object.entries(counts.delete.byReason).map(([reason, count]) => [
        `Deleted: ${reason}`,
        count,
      ]),
      ['Patched', counts.patch.total],
      ['Configured skips', counts.skip.total],
      ...Object.entries(counts.skip.byReason).map(([reason, count]) => [
        `Configured skip: ${reason}`,
        count,
      ]),
      ['Selected', rows.length],
      ['Skipped in selection', skipped],
      ...Object.entries(skips).map(([reason, count]) => [
        `Skipped: ${reason}`,
        count,
      ]),
      ['Run (selected, excluding skips)', rows.length - skipped],
      ...Object.entries(verdictCounts)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([verdict, count]) => [`Verdict: ${verdict}`, count]),
    ]),
    '',
  ]
}
