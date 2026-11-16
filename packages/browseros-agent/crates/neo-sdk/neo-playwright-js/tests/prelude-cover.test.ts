/**
 * A covered pointer action's error in the prelude (src/prelude/cover.ts): the cover's own
 * snapshot goes right under Rust's "On top" line, in a script's default snapshot form, cut at
 * 1,500 characters; the script keeps the rest of the cover; and a cover with nothing to render
 * leaves Rust's message as it is, its controls and "Do" lines included.
 */
import { expect, test } from 'bun:test'
import { setCoverRenderer, shapeCovered } from '../src/prelude/cover'
import { renderSnapshotV1, withDefaults } from '../src/prelude/snapshot-v1'

// As the prelude's entry does (index.ts).
setCoverRenderer(
  (json) => renderSnapshotV1({ json, partial: [] }, withDefaults({})).text,
)

const RUST = [
  'locator.click: Timeout 1500ms exceeded.',
  '  On top: dialog "Terms" [ref=e1] <div role="dialog">…</div>',
  '  It has been there since this click began.',
  '  Nothing was pressed.',
  '  Its controls:',
  '    - button "I agree" [ref=e3]',
  '  Do: answer or close it with one of its controls, then try again.',
  'Call log:',
  '  - attempting click action',
].join('\n')

function covered(snapshot: unknown): Error {
  const error = new Error(RUST)
  Reflect.set(error, 'cover', {
    ref: 'e1',
    role: 'dialog',
    name: 'Terms',
    kind: 'page',
    controls: [{ ref: 'e3', role: 'button', name: 'I agree' }],
    snapshot,
  })
  return error
}

test('tests that the snapshot goes under On top and the script keeps the rest', () => {
  const error = covered([
    {
      role: 'dialog',
      name: 'Terms',
      ref: 'e1',
      children: [
        'Read these terms.',
        { role: 'button', name: 'I agree', ref: 'e3' },
      ],
    },
  ])
  shapeCovered(error)
  const lines = error.message.split('\n')
  const onTop = lines.findIndex((line) => line.startsWith('  On top: '))
  expect(lines[onTop + 1]).toBe('    - dialog "Terms" [ref=e1]:')
  expect(error.message).toContain('Read these terms.')
  // Rust's own lines follow, in their order, before the call log.
  const after = lines.slice(onTop + 1).join('\n')
  expect(after.indexOf('  It has been there')).toBeLessThan(
    after.indexOf('  Its controls:'),
  )
  expect(after.indexOf('  Do: ')).toBeLessThan(after.indexOf('Call log:'))
  expect(Reflect.get(error, 'cover')).toEqual({
    ref: 'e1',
    role: 'dialog',
    name: 'Terms',
    kind: 'page',
    controls: [{ ref: 'e3', role: 'button', name: 'I agree' }],
  })
})

test('tests that a long snapshot is cut at 1,500 characters and the controls stay', () => {
  const words = 'Please read every word of these terms before you go on. '
  const error = covered([
    {
      role: 'dialog',
      name: 'Terms',
      ref: 'e1',
      children: Array.from({ length: 80 }, (_, index) => ({
        role: 'paragraph',
        ref: `e${index + 10}`,
        children: [words],
      })),
    },
  ])
  shapeCovered(error)
  const start = error.message.indexOf('\n', error.message.indexOf('  On top: '))
  const end = error.message.indexOf('\n  It has been there')
  const snapshot = error.message.slice(start, end)
  expect(snapshot).toContain('    - dialog "Terms" [ref=e1]')
  expect(snapshot).toContain('Please read every word')
  expect(snapshot).toContain('    … (the rest of its snapshot is cut)')
  // The kept lines hold at most 1,500 characters, plus each line's indent and the marker.
  expect(snapshot.length).toBeLessThanOrEqual(1500 + 8 * 4 + 60)
  expect(error.message).toContain('    - button "I agree" [ref=e3]')
  expect(error.message).toContain('  Do: answer or close it')
})

test('tests that nothing to render leaves the message as Rust wrote it', () => {
  for (const snapshot of [null, undefined, 'not a tree', []]) {
    const error = covered(snapshot)
    shapeCovered(error)
    expect(error.message).toBe(RUST)
    expect(Reflect.get(error, 'cover')).not.toHaveProperty('snapshot')
  }
  const plain = new Error('locator.click: Timeout 1500ms exceeded.')
  shapeCovered(plain)
  expect(plain.message).toBe('locator.click: Timeout 1500ms exceeded.')
  expect(Reflect.get(plain, 'cover')).toBeUndefined()
})
