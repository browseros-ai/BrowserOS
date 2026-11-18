/**
 * Rust reports when the snapshot root lost its clickable-element check. Default, full and
 * delta text share this pinned explanation; it describes this read and stays out of diffs.
 */
import type { SnapshotResult } from './wire'

const PREFIX = '# refs missing: '

/** This fact is outside the token cap: a cut must not conceal why refs may be absent. */
export function unmarkedHead(result: SnapshotResult): string[] {
  return result.unmarked
    ? [
        `${PREFIX}the check for clickable elements did not finish on this page (${result.unmarked.reason}). Some clickable elements may have no ref here, and content inside closed shadow roots may be missing. Take the snapshot again; if this line is still here, use a locator.`,
      ]
    : []
}

/** A read's explanation is not page content and cannot become a delta hunk. */
export function isUnmarkedLine(line: string): boolean {
  return line.startsWith(PREFIX)
}
