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

it('pins the omission explanation outside the cap without taking space from live content', () => {
  const result: SnapshotResult = {
    json: [background, { role: 'button', ref: 'e9', name: 'Live content' }],
    partial: [],
  }
  const body = '- button "Live content" [ref=e9]'
  const maxTokens = Math.ceil(body.length / 4)
  expect(renderSnapshotV1(result, withDefaults({ maxTokens })).text).toBe(
    `${notice}\n${body}`,
  )
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
  expect(
    await runtime.run(
      `return await page.ariaSnapshot({ mode: 'full', maxTokens: ${Math.ceil(original.length / 4)} })`,
    ),
  ).toBe(
    `# inert: 3 lines of this page cannot be clicked or typed into now. page.ariaSnapshot() lists what can.\n${original}`,
  )
})

it('pins inertness before the text notice without moving the partial warning', () => {
  const result: SnapshotResult = {
    json: [
      background,
      { role: 'paragraph', text: 'Words '.repeat(2200) },
      { role: 'button', ref: 'e9', name: 'Live' },
    ],
    partial: [],
    closedRoots: { sent: 1, found: 2 },
  }
  const lines = renderSnapshotV1(result, withDefaults({})).text.split('\n')
  expect(lines[0]).toBe(notice)
  expect(lines[1]).toStartWith('# text left out:')
  expect(lines[2]).toBe('# partial: closed shadow roots not all read (1 of 2)')
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

it('counts omitted dialog prose in its forced-text form even with text false', () => {
  const result: SnapshotResult = {
    json: [
      {
        role: 'dialog',
        ref: 'e1',
        children: [
          {
            role: 'paragraph',
            ref: 'e2',
            text: 'Inert explanation',
            inert: true,
          },
          { role: 'button', name: 'Continue', ref: 'e3' },
        ],
      },
    ],
    partial: [],
  }
  expect(renderSnapshotV1(result, withDefaults({ text: false })).text).toBe(
    '# not listed: 1 lines (0 with refs) of this page are inert: nothing in them can be clicked or typed into now. Pass { inert: true } to list them.\n- dialog [ref=e1]:\n  - button "Continue" [ref=e3]',
  )
})

it('does not join omitted text across retained output while adjacent omissions still join', () => {
  for (const separated of [false, true]) {
    const result: SnapshotResult = {
      json: [
        { role: 'paragraph', ref: 'e1', text: 'First', inert: true },
        ...(separated
          ? [{ role: 'button' as const, ref: 'e2', name: 'Live' }]
          : []),
        { role: 'paragraph', ref: 'e3', text: 'Last', inert: true },
      ],
      partial: [],
    }
    expect(renderSnapshotV1(result, withDefaults({})).text).toStartWith(
      `# not listed: ${separated ? 2 : 1} lines (0 with refs)`,
    )
  }
})

it('a heading never folds an omitted descendant back into its own text', () => {
  const result: SnapshotResult = {
    json: [
      {
        role: 'heading',
        level: 2,
        ref: 'e1',
        children: [
          'Heading',
          {
            role: 'paragraph',
            ref: 'e2',
            text: 'Inert explanation',
            inert: true,
          },
        ],
      },
    ],
    partial: [],
  }
  const text = renderSnapshotV1(result, withDefaults({})).text
  expect(text).toStartWith('# not listed:')
  expect(text).toContain('- heading [level=2]: Heading')
  expect(text).not.toContain('Inert explanation')
  expect(renderSnapshotV1(result, withDefaults({ inert: true })).text).toBe(
    '- heading [level=2]: Heading Inert explanation',
  )
})

it('an emptied live container does not repeat its name as replacement text', () => {
  const result: SnapshotResult = {
    json: [
      {
        role: 'group',
        ref: 'e1',
        name: 'Container name',
        children: [
          { role: 'button', ref: 'e2', name: 'Inert content', inert: true },
        ],
      },
    ],
    partial: [],
  }
  const text = renderSnapshotV1(result, withDefaults({})).text
  expect(text).toStartWith('# not listed: 1 lines (1 with refs)')
  expect(text).not.toContain('Container name')
  expect(text).not.toContain('Inert content')
})
