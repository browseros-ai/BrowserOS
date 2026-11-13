// The line a snapshot's text opens with when clickable marking found seeds of closed shadow roots
// that it could not hand a probe (Rust's `closedRoots`, design section 18): at the top of the v1
// text in every form and of `mode: 'full'`, under the notice when there is one, so the size cap
// never cuts it; never in Playwright's own YAML; and with no `closedRoots`, every byte as before.
import { expect, it } from 'bun:test'
import { renderFullSnapshot, renderSnapshot } from '../src/prelude/snapshot'
import { renderSnapshotV1, withDefaults } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'

const LINE = '# partial: closed shadow roots not all read (1000 of 1001)'

const buttons = (count: number): SnapshotResult['json'] =>
  Array.from({ length: count }, (_, n) => ({
    role: 'button',
    name: `Card ${n}`,
    ref: `e${n + 2}`,
  }))

/** A main section of `count` buttons and some text, its roots all read or not. */
function page(count: number, cut: boolean): SnapshotResult {
  return {
    json: [{ role: 'main', ref: 'e1', children: ['Cards', ...buttons(count)] }],
    partial: [],
    ...(cut ? { closedRoots: { sent: 1000, found: 1001 } } : {}),
  }
}

it('tests that every v1 form and mode full open with the line, and the same text follows it', () => {
  for (const options of [{}, { text: true }, { text: false }] as const) {
    const whole = renderSnapshotV1(page(5, false), withDefaults(options)).text
    const cut = renderSnapshotV1(page(5, true), withDefaults(options)).text
    expect(cut).toBe(`${LINE}\n${whole}`)
  }
  const whole = renderFullSnapshot(page(5, false), 8000)
  expect(renderFullSnapshot(page(5, true), 8000)).toBe(`${LINE}\n${whole}`)
})

it('tests that the line sits under the text notice and that no size cap removes it', () => {
  // About 4,000 tokens of text: over the 3,000 the default keeps text for, and over a cap of 1,000.
  const large = page(500, true)
  const v1 = renderSnapshotV1(large, withDefaults({})).text.split('\n')
  expect(v1[0]).toStartWith('# text left out: about ')
  expect(v1[1]).toBe(LINE)
  const capped = renderSnapshotV1(large, withDefaults({ maxTokens: 1000 }))
  expect(capped.text.split('\n').slice(0, 2)[1]).toBe(LINE)
  expect(capped.text).toContain('\n# cut: ')
  const full = renderFullSnapshot(large, 1000).split('\n')
  expect(full[0]).toBe(LINE)
  expect(full.some((line) => line.startsWith('# cut: '))).toBe(true)
  // No limit hides it, as none hides the cut line; the size the cut line gives leaves it out.
  for (const maxTokens of [1, 10]) {
    const tiny = renderSnapshotV1(large, withDefaults({ maxTokens })).text
    expect(tiny.split('\n')).toContain(LINE)
    expect(renderFullSnapshot(large, maxTokens).split('\n')[0]).toBe(LINE)
    const without = renderSnapshotV1(
      page(500, false),
      withDefaults({ maxTokens }),
    )
    expect(tiny.replace(`${LINE}\n`, '')).toBe(without.text)
  }
})

it("tests that Playwright's own YAML never prints the line", () => {
  expect(renderSnapshot(page(5, true))).toBe(renderSnapshot(page(5, false)))
})
