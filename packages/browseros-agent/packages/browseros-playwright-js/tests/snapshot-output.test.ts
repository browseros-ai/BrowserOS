import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { AriaSnapshotJSON } from '../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import { renderAriaSnapshotAsYaml } from '../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'

type PartialFrame = { ref: string; frame: string; reason: string }

const data = (name: string): unknown =>
  JSON.parse(
    readFileSync(
      new URL(
        `../../../crates/browseros-playwright/tests/data/${name}`,
        import.meta.url,
      ),
      'utf8',
    ),
  )

/** Recorded from playwright-core 1.64.1 by scripts/capture-snapshot-fixture.mjs. */
const recorded = data('snapshot-frames.json') as {
  stitched: AriaSnapshotJSON
  yaml: string
}
/** What the crate's snapshot op returns for that page (written by its Rust tests). */
const output = data('snapshot-output.json') as {
  json: AriaSnapshotJSON
  partial: PartialFrame[]
}

/**
 * Upstream's text with Neo's documented changes made as text edits, apart from the Rust code
 * that makes them on the tree: a generic named by its own text prints that text, a generic's
 * name over other content goes, the code swept in the top frame is masked where its child
 * prints it, and a partial frame's iframe keeps its line but loses its content.
 */
function withNeoRules(upstream: string): string {
  const lines: string[] = []
  let partialIndent: number | undefined
  for (const line of upstream.split('\n')) {
    const indent = line.length - line.trimStart().length
    if (partialIndent !== undefined && indent > partialIndent) continue
    partialIndent = undefined
    if (
      output.partial.some(({ ref }) => line.trim() === `- iframe [ref=${ref}]:`)
    ) {
      lines.push(line.slice(0, -1))
      partialIndent = indent
      continue
    }
    lines.push(
      line
        .replace(
          /^(\s*)- generic "([^"]*)" (\[ref=[^\]]+\])$/,
          '$1- generic $3: $2',
        )
        .replace(
          /^(\s*)- generic "[^"]*" (\[ref=[^\]]+\]):$/,
          '$1- generic $2:',
        )
        .replaceAll('482913', '[private]'),
    )
  }
  return lines.join('\n')
}

it('tests that the vendored renderer prints upstream’s stitched tree as Playwright printed it', () => {
  expect(renderAriaSnapshotAsYaml(recorded.stitched)).toBe(recorded.yaml)
})

it('tests that the crate’s snapshot output renders as Playwright’s text with Neo’s rules', () => {
  expect(output.partial.map(({ ref }) => ref)).toEqual(['e11'])
  expect(renderAriaSnapshotAsYaml(output.json)).toBe(
    withNeoRules(recorded.yaml),
  )
})
