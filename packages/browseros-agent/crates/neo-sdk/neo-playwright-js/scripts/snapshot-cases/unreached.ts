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
  '../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts',
  '../neo-playwright/third_party/playwright/packages/isomorphic/yaml.ts',
]

/**
 * Lines `from` through `to` (each a line's trimmed text, `to` defaulting to `from`), or the
 * whole file when neither is given.
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
      'The call’s clock, not its text: whether a whole page’s baseline operations still have time. Its one text, the no-time answer, is `renderUncompared` in snapshot-delta.ts, which the `notRead` cases reach. The port moves this clock into the operation, pinned by rows on a scripted clock (design 1.7, 5.6).',
  },
  {
    file: VENDORED_PRINTER,
    from: 'const includeText = options.convertStringsToRegex ? textContributesInfo : () => true;',
    to: 'const renderString = options.convertStringsToRegex ? convertToBestGuessRegex : (str: string) => str;',
    reason:
      'The regex mode, which only a failed `expect` uses, inside the page with upstream’s own copy of this printer. Not ported (design 3.2).',
  },
]
