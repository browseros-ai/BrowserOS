/** Snapshot reply fixtures in Neo's markdown section format. */

import { readFileSync } from 'node:fs'

/** The snapshot from run 8 of the 4e58b2527 night check, followed by Neo's delta tip. */
export const SNAPSHOT_WITH_TIP = readFileSync(
  new URL('./reply-snapshot-with-tip.txt', import.meta.url),
  'utf8',
)

export const TIP_LINE =
  'page.ariaSnapshot({ delta: true }) returns only what changed since your last snapshot.'

/** The returned snapshot of `SNAPSHOT_WITH_TIP`, decoded. */
const snapshot: unknown = JSON.parse(SNAPSHOT_WITH_TIP.split('\n')[1] ?? '""')
if (typeof snapshot !== 'string') throw new Error('Expected a snapshot string')
export const SNAPSHOT = snapshot

const NOTE_LINE =
  '[not awaited] page.goto at run:4:1 (op 9) was still running when the script returned; it was cancelled.'

/** The same returned snapshot followed by either a Tips or a Notes section. */
export const TRAILED: ReadonlyArray<
  readonly [reply: string, trailer: string, section: 'tips' | 'notes']
> = [
  [SNAPSHOT_WITH_TIP, TIP_LINE, 'tips'],
  [
    `### Result\n${JSON.stringify(SNAPSHOT)}\n### Notes\n${NOTE_LINE}`,
    NOTE_LINE,
    'notes',
  ],
]
