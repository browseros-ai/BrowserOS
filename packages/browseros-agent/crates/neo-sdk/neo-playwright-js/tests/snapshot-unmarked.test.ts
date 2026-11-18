// A host must know when the snapshot could not discover script-only click targets or closed
// shadow roots. This fact is pinned above the tree and describes the read, not a page change.
import { expect, it } from 'bun:test'
import { renderFullSnapshot, renderSnapshot } from '../src/prelude/snapshot'
import {
  NO_BASELINE,
  NO_CHANGE,
  NO_TIME,
  renderDelta,
  renderUncompared,
  TOO_MUCH,
} from '../src/prelude/snapshot-delta'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'

const missing =
  '# refs missing: the check for clickable elements did not finish on this page (timeout). Some clickable elements may have no ref here, and content inside closed shadow roots may be missing. Take the snapshot again; if this line is still here, use a locator.'

const control: SnapshotResult['json'][number] = {
  role: 'button',
  name: 'Continue',
  ref: 'e1',
}

const options = { urls: false, maxTokens: 8000, text: true }
const head =
  '# on top: dialog "Read first" [ref=e9] over the whole view\n#   its controls: button "Continue" [ref=e1]'
const unchecked = (): SnapshotResult => ({
  json: [control],
  partial: [],
  unmarked: { reason: 'timeout' },
  onTop: [{ ref: 'e9', line: head, wholeView: true }],
})

it('keeps the unchecked-root explanation outside a twenty-token cap', () => {
  const result = {
    json: [control],
    partial: [],
    unmarked: { reason: 'timeout' },
  }
  expect(renderSnapshotV1(result, { urls: false, maxTokens: 20 }).text).toBe(
    `${missing}\n- button "Continue" [ref=e1]`,
  )
})

it('places the explanation after layers and before inertness in Neo and full text', () => {
  const result = unchecked()
  result.json = [
    { role: 'button', name: 'Behind', ref: 'e2', inert: true },
    control,
  ]
  for (const maxTokens of [1, 20, 8000]) {
    expect(
      renderSnapshotV1(result, { ...options, maxTokens }).text,
    ).toStartWith(`${head}\n${missing}\n# not listed:`)
    expect(renderFullSnapshot(result, maxTokens)).toStartWith(
      `${head}\n${missing}\n# inert:`,
    )
  }
  expect(renderSnapshot(result)).not.toContain('# refs missing:')
})

it('describes only the current read under every delta status, outside comparisons', () => {
  const result = unchecked()
  const checked = { ...result, unmarked: undefined }
  const baseline = renderSnapshotV1(checked, options)
  const stored = { text: baseline.text, form: baseline.form }
  expect(renderDelta(stored, result, {}).returned).toBe(
    `${NO_CHANGE}\n${head}\n${missing}`,
  )
  for (const [text, status] of [
    [renderDelta(null, result, options).returned, NO_BASELINE],
    [renderDelta(stored, result, { urls: true }).returned, TOO_MUCH],
    [renderUncompared(result, options), NO_TIME],
  ]) {
    expect(text).toStartWith(`${status}\n${head}\n${missing}\n`)
    expect(text.split(missing)).toHaveLength(2)
  }
  const uncheckedBaseline = renderSnapshotV1(result, options)
  expect(renderDelta(uncheckedBaseline, checked, {}).returned).toBe(
    `${NO_CHANGE}\n${head}`,
  )
})

it('keeps real losses of refs in changes without treating the explanation as content', () => {
  const nodes = Array.from(
    { length: 30 },
    (_, n): SnapshotResult['json'][number] => ({
      role: 'button',
      name: `Action ${n}`,
      ref: `e${n + 1}`,
    }),
  )
  const previous = renderSnapshotV1({ json: nodes, partial: [] }, options)
  const result: SnapshotResult = {
    json: nodes.map((node, n) =>
      n === 15 ? { ...node, ref: undefined } : node,
    ),
    partial: [],
    unmarked: { reason: 'page shape' },
  }
  const text = renderDelta(previous, result, {}).returned
  expect(text.split('\n')[0]).toStartWith('# changes since')
  expect(text.split('\n')[1]).toContain('(page shape).')
  expect(text).toContain('-- button "Action 15" [ref=e16]')
  expect(text.split('# refs missing:')).toHaveLength(2)
  expect(text.slice(text.indexOf('@@'))).not.toContain('# refs missing:')
})
