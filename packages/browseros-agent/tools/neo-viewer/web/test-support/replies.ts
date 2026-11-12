/** Real reply texts from Neo, exactly as the model read them, for the reply rules' tests. */

import { readFileSync } from 'node:fs'

/**
 * Run 8 of the 4e58b2527 night check: MUI's data grid page as `page.ariaSnapshot({ text: true })`
 * returned it, one line of JSON, then the delta tip Neo appends on a later snapshot of a tab.
 */
export const SNAPSHOT_WITH_TIP = readFileSync(
  new URL('./reply-snapshot-with-tip.txt', import.meta.url),
  'utf8',
)

export const TIP_LINE =
  'Tip: page.ariaSnapshot({ delta: true }) returns only what changed since your last snapshot.'

/** The returned snapshot of `SNAPSHOT_WITH_TIP`, decoded. */
export const SNAPSHOT = JSON.parse(
  SNAPSHOT_WITH_TIP.split('\n')[0] ?? '""',
) as string
