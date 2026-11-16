// The native-inert annotation crosses the probe/host boundary; these rows hold the public
// formatting contract independently of how the browser's DOM supplied the annotation.
import { expect, it } from 'bun:test'
import { renderSnapshot } from '../src/prelude/snapshot'
import {
  listChanges,
  NO_CHANGE,
  renderDelta,
  TOO_MUCH,
} from '../src/prelude/snapshot-delta'
import { renderSnapshotV1, withDefaults } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { loadPrelude, SEED } from './support/prelude-runtime'

const background = Object.assign(
  {
    role: 'main' as const,
    ref: 'e1',
    children: [
      { role: 'button' as const, name: 'Background', ref: 'e2' },
      { role: 'link' as const, name: 'Help', ref: 'e3' },
    ],
  },
  { inert: true },
)
const tree: SnapshotResult = {
  json: [
    background,
    {
      role: 'dialog',
      ref: 'e4',
      children: [{ role: 'button', name: 'Continue', ref: 'e5' }],
    },
  ],
  partial: [],
}
const original =
  '- main [ref=e1]:\n  - button "Background" [ref=e2]\n  - link "Help" [ref=e3]\n- dialog [ref=e4]:\n  - button "Continue" [ref=e5]'
const notice =
  '# not listed: 3 lines (3 with refs) of this page are inert: nothing in them can be clicked or typed into now. Pass { inert: true } to list them.'

it('lists only live controls and explains the exact omitted lines and refs', () => {
  expect(renderSnapshotV1(tree, withDefaults({})).text).toBe(
    `${notice}\n- dialog [ref=e4]:\n  - button "Continue" [ref=e5]`,
  )
})

it('the public inert option restores the original bytes without an omission notice', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.ariaSnapshot' ? tree : null),
  })
  expect(
    await runtime.run('return await page.ariaSnapshot({ inert: true })'),
  ).toBe(original)
  expect(await runtime.run('return await page.ariaSnapshot()')).toStartWith(
    notice,
  )
})

it('keeps the omission explanation even at a one-token cap', () => {
  for (const maxTokens of [
    ...Array.from({ length: 128 }, (_, n) => n + 1),
    8000,
  ]) {
    const text = renderSnapshotV1(tree, withDefaults({ maxTokens })).text
    expect(text.split('\n')[0]).toBe(notice)
    expect(text).not.toContain('Background')
  }
})

it('full mode keeps the original tree and adds only its inert explanation', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.ariaSnapshot' ? tree : null),
  })
  expect(
    await runtime.run("return await page.ariaSnapshot({ mode: 'full' })"),
  ).toBe(
    `# inert: 3 lines of this page cannot be clicked or typed into now. page.ariaSnapshot() lists what can.\n${original}`,
  )
})

it('does not make changed omission counts into page changes', () => {
  const content = '- button "Continue" [ref=e5]'
  expect(
    listChanges(
      `${notice}\n${content}`,
      `${notice.replace('3 lines (3', '5 lines (5')}\n${content}`,
    ),
  ).toBe(NO_CHANGE)
})

it('a tree without annotations retains its original text and no notice', () => {
  const unmarked: SnapshotResult = {
    ...tree,
    json: [{ role: 'button', name: 'Live', ref: 'e1' }],
  }
  expect(renderSnapshotV1(unmarked, withDefaults({})).text).toBe(
    '- button "Live" [ref=e1]',
  )
})

it('delta carries the inert option and returns full reads across modal opening and closing', () => {
  const included = renderSnapshotV1(tree, withDefaults({ inert: true }))
  const inherited = renderDelta(
    { text: included.text, form: included.form },
    tree,
    {},
  )
  expect(inherited.returned).toBe(NO_CHANGE)
  expect(inherited.baseline.text).toBe(original)
  const switched = renderDelta(
    { text: included.text, form: included.form },
    tree,
    { inert: false },
  )
  expect(switched.returned).toStartWith(TOO_MUCH)
  expect(switched.baseline.text).toStartWith(notice)
  const unmarked: SnapshotResult = {
    ...tree,
    json: [{ role: 'main', ref: 'e1', children: background.children }],
  }
  const before = renderSnapshotV1(unmarked, withDefaults({}))
  const opening = renderDelta(
    { text: before.text, form: before.form },
    tree,
    {},
  )
  expect(opening.returned).toStartWith(TOO_MUCH)
  expect(opening.baseline.text).toStartWith(notice)
  const closing = renderDelta(
    { text: opening.baseline.text, form: opening.baseline.form },
    unmarked,
    {},
  )
  expect(closing.returned).toStartWith(TOO_MUCH)
  expect(closing.baseline.text).toBe(before.text)
})

it('full mode always keeps the inert notice at every small cap', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.ariaSnapshot' ? tree : null),
  })
  for (const maxTokens of [1, 2, 10, 100])
    expect(
      await runtime.run(
        `return await page.ariaSnapshot({ mode: 'full', maxTokens: ${maxTokens} })`,
      ),
    ).toStartWith('# inert: 3 lines')
})

it('conformance and mode default keep upstream bytes and never request marking', async () => {
  for (const upstreamSnapshots of [true, false]) {
    const runtime = loadPrelude({
      seed: SEED,
      upstreamSnapshots,
      answer: (call) => (call.name === 'page.ariaSnapshot' ? tree : null),
    })
    const script = upstreamSnapshots
      ? "return await page.ariaSnapshot({mode:'ai', inert:true})"
      : "return await page.ariaSnapshot({mode:'default', inert:true})"
    expect(await runtime.run(script)).toBe(renderSnapshot(tree))
    const call = runtime.calls.find((call) => call.name === 'page.ariaSnapshot')
    expect(call?.input).not.toHaveProperty('clickable')
  }
})

it('omitted iframe subtrees also omit their partial diagnostics', () => {
  const frame = Object.assign(
    { role: 'iframe' as const, ref: 'e7' },
    { inert: true },
  )
  const result: SnapshotResult = {
    json: [frame],
    partial: [
      { ref: 'e7', frame: 'https://frame.example/', reason: 'timeout' },
    ],
  }
  const text = renderSnapshotV1(result, withDefaults({})).text
  expect(text).toStartWith('# not listed: 1 lines (1 with refs)')
  expect(text).not.toContain('# partial:')
  expect(
    renderSnapshotV1(result, withDefaults({ inert: true })).text,
  ).toContain('# partial:')
})
