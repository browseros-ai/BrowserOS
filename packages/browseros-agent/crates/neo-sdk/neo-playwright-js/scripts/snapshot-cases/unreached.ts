/**
 * The renderer's lines the snapshot text cases do not reach, each with its reason. The coverage
 * test (tests/snapshot-cases-coverage.test.ts) fails on a line no case reaches and no entry here
 * explains, and on an entry that explains nothing any more. Entries name lines by their text, so
 * an edit elsewhere in a file does not move them.
 */

/** Today's renderer, as the Rust port reads it: every file whose lines the cases must reach. */
export const RENDERER = [
  'src/prelude/snapshot.ts',
  'src/prelude/snapshot-v1.ts',
  'src/prelude/snapshot-cap.ts',
  'src/prelude/snapshot-delta.ts',
  'src/prelude/snapshot-on-top.ts',
  'src/prelude/snapshot-inert.ts',
  'src/prelude/snapshot-late.ts',
  'src/prelude/snapshot-unmarked.ts',
  'src/prelude/cover.ts',
  'scripts/snapshot-cases/snapshot-lines.ts',
  '../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts',
  '../neo-playwright/third_party/playwright/packages/isomorphic/yaml.ts',
]

/**
 * Lines `from` through `to`, or the whole file when neither is given. `from` is the first line
 * of the file whose trimmed text it is, and `to` (defaulting to `from`) the first such line at
 * or after it, so a line of common text may need a `from` that pins it.
 */
export type Unreached = {
  file: (typeof RENDERER)[number]
  from?: string
  to?: string
  reason: string
}

const VENDORED_PRINTER =
  '../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts'

export const UNREACHED: Unreached[] = [
  {
    file: 'src/prelude/snapshot-late.ts',
    reason:
      'The call’s clock, not its text: whether a whole page’s baseline operations still have time. Its one text, the no-time answer, is `renderUncompared` in snapshot-delta.ts, which the `notRead` cases reach. The port moves this clock into the operation, pinned by rows on a scripted clock (the Rust snapshot renderer design, 1.7 and 5.6).',
  },
  {
    file: VENDORED_PRINTER,
    from: 'function convertToBestGuessRegex(text: string): string {',
    to: 'return String(new RegExp(pattern));',
    reason:
      'The regex mode, which only a failed `expect` uses, inside the page with upstream’s own copy of this printer: its string to pattern step. Not ported (the Rust snapshot renderer design, 3.2).',
  },
  {
    file: VENDORED_PRINTER,
    from: 'function textContributesInfo(node: AriaNodeJSON, text: string): boolean {',
    to: 'return filtered.trim().length / text.length > 0.1;',
    reason:
      'The regex mode’s test of whether a text says more than its name. Not ported (the Rust snapshot renderer design, 3.2).',
  },
  {
    file: 'src/prelude/snapshot-delta.ts',
    from: 'while (x > 0 && y > 0) {',
    to: "reversed.push({ kind: ' ', before: offset + x, after: offset + y })",
    reason:
      'No input reaches it: `diff` matches the common head before the search, so the first lines of the middle differ, round 0’s snake is empty, and the walk back always ends at the start. A port may keep the loop; no case can tell.',
  },
  {
    file: 'scripts/snapshot-cases/snapshot-lines.ts',
    from: '} catch {',
    to: "return ''",
    reason:
      'Only a tree outside Playwright’s types makes today’s renderer throw. Rust decodes the cover’s tree before it prints the block and leaves a tree that does not decode with no block (neo-playwright action/cover.rs `block`), so the Rust block has no failure to fall back from.',
  },
]
