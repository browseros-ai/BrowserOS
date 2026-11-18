// The override behind clickable marking's page half (src/probe/pointer-targets.ts): this world's
// getComputedStyle answers `cursor: pointer` for a snapshot's targets and, as CSS inheritance
// would, for their descendants whose own real cursor is their target's, only while that one
// snapshot runs; every other read, and every read after it, is the real one.
import { expect, it } from 'bun:test'

type Style = { cursor: string; display: string }
type Fake = { parentElement: Fake | null; parentNode: Fake | null }

const real = new Map<Fake, Style>()
const element = (cursor: string, parent: Fake | null = null): Fake => {
  const made: Fake = { parentElement: parent, parentNode: parent }
  real.set(made, { cursor, display: 'block' })
  return made
}
const page = element('auto')
const target = element('auto', page)
const child = element('auto', target)
const grandchild = element('auto', child)
const field = element('text', target)
const inField = element('text', field)
const sibling = element('auto', page)
// A descendant that set a cursor of its own, the same as the page's: not the target's.
const ownDefault = element('default', target)
// A page under `body { cursor: default }`: a label (or any target) whose real cursor is
// `default`, around a named icon, beside a field and outside text.
const body = element('default')
const label = element('default', body)
const icon = element('default', label)
const labelField = element('text', label)
const outside = element('default', body)
const own = (of: Fake) => real.get(of)
Reflect.set(globalThis, 'getComputedStyle', own)
/** The world's `getComputedStyle` as it stands now. */
const current = (): unknown => Reflect.get(globalThis, 'getComputedStyle')
const { asPointerTargets } = await import('../src/probe/pointer-targets')
const style = (of: Fake, pseudo?: string): Style =>
  Reflect.apply(Reflect.get(globalThis, 'getComputedStyle'), globalThis, [
    of,
    pseudo,
  ])
const cursors = () =>
  [target, child, grandchild, field, inField, sibling, page].map(
    (of) => style(of).cursor,
  )
const asTargets = <T>(run: () => T) =>
  asPointerTargets([target as unknown as Element], run)
const labelCursors = () =>
  [label, icon, labelField, outside, body].map((of) => style(of).cursor)

it('tests that a target and its auto-cursor descendants read as a pointer, only during that snapshot', () => {
  expect(cursors()).toEqual([
    'auto',
    'auto',
    'auto',
    'text',
    'text',
    'auto',
    'auto',
  ])
  const during = asTargets(() => ({
    cursors: cursors(),
    pseudo: style(target, '::before').cursor,
    display: style(child).display,
    ownDefault: style(ownDefault).cursor,
  }))
  expect(during).toEqual({
    // The target, its descendants that inherit, but not the field with its own cursor or
    // what sits inside it, nor anything outside the target.
    cursors: ['pointer', 'pointer', 'pointer', 'text', 'text', 'auto', 'auto'],
    pseudo: 'auto',
    display: 'block',
    ownDefault: 'default',
  })
  expect(
    current() === own,
    "outside the call the world's own function is back",
  ).toBe(true)
  expect(cursors(), 'a read after the snapshot is the real one').toEqual([
    'auto',
    'auto',
    'auto',
    'text',
    'text',
    'auto',
    'auto',
  ])
})

it('tests that a snapshot that throws still leaves every read real', () => {
  expect(() =>
    asTargets(() => {
      throw new Error('engine failed')
    }),
  ).toThrow('engine failed')
  expect(cursors()).toEqual([
    'auto',
    'auto',
    'auto',
    'text',
    'text',
    'auto',
    'auto',
  ])
  expect(current() === own).toBe(true)
  expect(
    asPointerTargets([], current) === own,
    'a snapshot without targets never touches it',
  ).toBe(true)
})

it('tests that inside a target whose real cursor is not auto, what inherits it reads as a pointer', () => {
  expect(labelCursors()).toEqual([
    'default',
    'default',
    'text',
    'default',
    'default',
  ])
  const during = asPointerTargets([label as unknown as Element], labelCursors)
  // The label and the icon that inherits its `default`, not the field with its own cursor, nor
  // the text outside the label that reads the same `default` from the body.
  expect(during).toEqual(['pointer', 'pointer', 'text', 'default', 'default'])
  expect(labelCursors()).toEqual([
    'default',
    'default',
    'text',
    'default',
    'default',
  ])
})
