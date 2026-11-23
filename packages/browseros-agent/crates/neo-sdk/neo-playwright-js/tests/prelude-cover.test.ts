/**
 * A covered pointer action's error in the prelude (src/prelude/cover.ts): the cover's block,
 * which Rust prints (neo-snapshot's `cover_block`), goes right under Rust's "On top" line,
 * unchanged; the script keeps the rest of the cover without it; and a cover with no block leaves
 * Rust's message as it is, its controls and "Do" lines included.
 */
import { expect, test } from 'bun:test'
import { shapeCovered } from '../src/prelude/cover'

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

const SCRIPT_COVER = {
  ref: 'e1',
  role: 'dialog',
  name: 'Terms',
  kind: 'page',
  controls: [{ ref: 'e3', role: 'button', name: 'I agree' }],
}

function covered(block?: unknown): Error {
  const error = new Error(RUST)
  Reflect.set(
    error,
    'cover',
    block === undefined ? { ...SCRIPT_COVER } : { ...SCRIPT_COVER, block },
  )
  return error
}

/** A block as Rust sends it: lines with four spaces before each, no newline at either end. */
const BLOCK = [
  '    - dialog "Terms" [ref=e1]:',
  '      - text: Read these terms.',
  '      - button "I agree" [ref=e3]',
].join('\n')

test('tests that the block goes under On top and the script keeps the rest', () => {
  const error = covered(BLOCK)
  shapeCovered(error)
  const lines = error.message.split('\n')
  const onTop = lines.findIndex((line) => line.startsWith('  On top: '))
  expect(lines.slice(onTop + 1, onTop + 4).join('\n')).toBe(BLOCK)
  // Rust's own lines follow, in their order, before the call log.
  expect(lines.slice(onTop + 4).join('\n')).toBe(
    RUST.slice(RUST.indexOf('\n  It has been there') + 1),
  )
  expect(Reflect.get(error, 'cover')).toEqual(SCRIPT_COVER)
  expect(Reflect.get(error, 'cover')).not.toHaveProperty('block')
})

test('tests that a long block is placed whole, as Rust cut it, and the controls stay', () => {
  const block = [
    '    - dialog "Terms" [ref=e1]:',
    ...Array.from(
      { length: 24 },
      (_, index) =>
        `      - paragraph [ref=e${index + 10}]: Please read every word of these terms before you go on.`,
    ),
    '    … (the rest of its snapshot is cut)',
  ].join('\n')
  const error = covered(block)
  shapeCovered(error)
  const start = error.message.indexOf('\n', error.message.indexOf('  On top: '))
  const end = error.message.indexOf('\n  It has been there')
  expect(error.message.slice(start + 1, end)).toBe(block)
  expect(error.message).toContain('    - button "I agree" [ref=e3]')
  expect(error.message).toContain('  Do: answer or close it')
})

test('tests that a block is placed under the first On top line only, and kept at the end', () => {
  const twice = `${RUST}\n  On top: a second line`
  const error = new Error(twice)
  Reflect.set(error, 'cover', { ...SCRIPT_COVER, block: BLOCK })
  shapeCovered(error)
  expect(error.message.split('\n  On top: ').length).toBe(3)
  expect(error.message.indexOf(BLOCK)).toBeLessThan(
    error.message.indexOf('\n  It has been there'),
  )
  const last = new Error(
    'locator.click: covered\n  On top: dialog "Terms" [ref=e1]',
  )
  Reflect.set(last, 'cover', { ...SCRIPT_COVER, block: BLOCK })
  shapeCovered(last)
  expect(last.message).toBe(
    `locator.click: covered\n  On top: dialog "Terms" [ref=e1]\n${BLOCK}`,
  )
})

test('tests that no block leaves the message as Rust wrote it', () => {
  for (const block of [undefined, null, '', 42, ['    - dialog']]) {
    const error = covered(block)
    shapeCovered(error)
    expect(error.message).toBe(RUST)
    expect(Reflect.get(error, 'cover')).toEqual(SCRIPT_COVER)
  }
  const noOnTop = new Error('locator.click: Timeout 1500ms exceeded.')
  Reflect.set(noOnTop, 'cover', { ...SCRIPT_COVER, block: BLOCK })
  shapeCovered(noOnTop)
  expect(noOnTop.message).toBe('locator.click: Timeout 1500ms exceeded.')
  expect(Reflect.get(noOnTop, 'cover')).toEqual(SCRIPT_COVER)
  const plain = new Error('locator.click: Timeout 1500ms exceeded.')
  shapeCovered(plain)
  expect(plain.message).toBe('locator.click: Timeout 1500ms exceeded.')
  expect(Reflect.get(plain, 'cover')).toBeUndefined()
})
