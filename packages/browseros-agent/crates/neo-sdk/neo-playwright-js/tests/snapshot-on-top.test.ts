/** Page-survey facts cross Rust as pinned words; only their whole-view content moves. */
import { expect, it } from 'bun:test'
import { renderFullSnapshot } from '../src/prelude/snapshot'
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

const head =
  '# on top: generic [ref=e9] over the whole view\n#   its controls: button "Dismiss" [ref=e10]'
const gone = '# no longer on top: generic [ref=e9]'
const layer = { ref: 'e9', line: head, wholeView: true }
const result = (
  whole = true,
): SnapshotResult & { onTop: (typeof layer)[] } => ({
  json: [
    {
      role: 'main',
      ref: 'e1',
      children: [{ role: 'button', name: 'Behind', ref: 'e2' }],
    },
    {
      role: 'generic',
      ref: 'e9',
      onTop: whole ? 'e9' : undefined,
      children: [
        'Read this first',
        { role: 'button', name: 'Dismiss', ref: 'e10' },
      ],
    },
  ],
  partial: [],
  onTop: [
    {
      ...layer,
      ...(whole
        ? {}
        : {
            wholeView: false,
            line: head.replace('over the whole view', 'over part of the view'),
          }),
    },
  ],
})
const options = { urls: false, maxTokens: 8000, text: false }

it('pins the layer before all cap notices, down to one token', () => {
  for (const maxTokens of [1, 2, 8, 80, 8000]) {
    const text = renderSnapshotV1(result(), { ...options, maxTokens }).text
    expect(text.startsWith(`${head}\n`)).toBe(true)
    expect(text.match(/# on top:/g)?.length).toBe(1)
  }
})

it('prints whole-view content first, with its text once, even in the interactive form', () => {
  const text = renderSnapshotV1(result(), options).text
  expect(text.indexOf('Read this first')).toBeGreaterThan(text.indexOf(head))
  expect(text.indexOf('Read this first')).toBeLessThan(text.indexOf('Behind'))
  expect(text.match(/Read this first/g)?.length).toBe(1)
  expect(text.match(/button "Dismiss" \[ref=e10\]/g)?.length).toBe(2) // head and body
})

it('keeps part-view content in place and keeps full mode in document order', () => {
  const part = renderSnapshotV1(result(false), { ...options, text: true }).text
  expect(part.indexOf('Behind')).toBeLessThan(part.indexOf('Read this first'))
  const full = renderFullSnapshot(result(), 8000)
  expect(full.startsWith(`${head}\n`)).toBe(true)
  expect(full.indexOf('Behind')).toBeLessThan(full.indexOf('Read this first'))
})

it('keeps on-top words ahead of inert counts and outside delta hunks', () => {
  const current = result()
  current.json[0].inert = true
  const whole = renderSnapshotV1(current, options)
  expect(whole.text.startsWith(`${head}\n# not listed:`)).toBe(true)
  const delta = renderDelta({ text: whole.text, form: whole.form }, current, {})
  expect(delta.returned).toBe(`${NO_CHANGE}\n${head}`)
  for (const [text, status] of [
    [renderDelta(null, current, {}).returned, NO_BASELINE],
    [
      renderDelta({ text: whole.text, form: whole.form }, current, {
        urls: true,
      }).returned,
      TOO_MUCH,
    ],
    [renderUncompared(current, {}), NO_TIME],
  ]) {
    expect(text.startsWith(`${status}\n${head}\n`)).toBe(true)
    expect(text.match(/# on top:/g)?.length).toBe(1)
  }
})

it('names a removed layer once and never puts its head in a hunk', () => {
  const previous = renderSnapshotV1(result(), options)
  const current = result()
  current.onTop = []
  const delta = renderDelta(
    { text: previous.text, form: previous.form },
    current,
    {},
  )
  expect(delta.returned.split('\n')[0]).toMatch(
    /^# (?:no change |changes |whole page:)/,
  )
  expect(delta.returned.split('\n')[1]).toBe(gone)
  expect(delta.returned).not.toMatch(
    /^[+ -]# (?:on top| {3}its controls|no longer on top):/m,
  )
})
