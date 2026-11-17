// The pointer-action requests that need no layout: verdicts and disarms without an armed
// interceptor, the route check's move count, and edits of a pinned element. Measuring, arming and hit tests need a
// real layout, so tests/actions_live.rs covers them on BrowserOS.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body><button id="go">Go</button><input id="name"></body></html>`

let page: ProbePage | undefined
afterEach(() => page?.close())

const booted = () => {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'a' })
  return page
}

const pin = (probe: ProbePage, selector: string): number => {
  const reply = probe.call({
    op: 'resolve',
    target: { chunk: parseSelector(selector) },
    strictness: 'strict',
    want: 'element',
  })
  const pinned: unknown =
    typeof reply === 'object' && reply !== null
      ? Reflect.get(reply, 'pin')
      : undefined
  if (typeof pinned !== 'number')
    throw new Error(`no pin in ${JSON.stringify(reply)}`)
  return pinned
}

it('tests that a verdict for a token nobody armed is an internal error, never done', () => {
  const probe = booted()
  expect(probe.call({ op: 'verdict', token: 7 })).toEqual({
    kind: 'error',
    class: 'internal',
    message: 'no interceptor is armed with token 7',
  })
})

it('tests that the route check counts trusted moves only, with no interceptor armed', () => {
  const probe = booted()
  expect(probe.call({ op: 'moves' })).toEqual({ kind: 'moves', moves: 0 })
  probe.document.dispatchEvent(new Event('mousemove', { bubbles: true }))
  expect(probe.call({ op: 'moves' })).toEqual({ kind: 'moves', moves: 0 })
})

it('tests that disarm with nothing armed answers done, every time', () => {
  const probe = booted()
  expect(probe.call({ op: 'disarm' })).toEqual({ kind: 'verdict', hit: 'done' })
  expect(probe.call({ op: 'disarm' })).toEqual({ kind: 'verdict', hit: 'done' })
})

it('tests that focus, blur and dispatchEvent act on the pinned element', () => {
  const probe = booted()
  const input = pin(probe, '#name')
  const field = probe.document.querySelector('#name')
  if (!field) throw new Error('the fixture has its field')
  const focused: string[] = []
  field.addEventListener('focus', () => focused.push('focus'))
  field.addEventListener('blur', () => focused.push('blur'))
  expect(
    probe.call({
      op: 'edit',
      pin: input,
      edit: { kind: 'focus', resetSelection: false },
    }),
  ).toEqual({ kind: 'value', value: null })
  expect(
    probe.call({ op: 'edit', pin: input, edit: { kind: 'blur' } }),
  ).toEqual({
    kind: 'value',
    value: null,
  })
  // Upstream's focusNode calls focus() twice (a Firefox workaround); linkedom fires both.
  expect(focused.filter((type, at) => type !== focused[at - 1])).toEqual([
    'focus',
    'blur',
  ])

  const button = pin(probe, '#go')
  const seen: string[] = []
  probe.document
    .querySelector('#go')
    ?.addEventListener('ping', (event) =>
      seen.push(`${event.type} bubbles=${event.bubbles}`),
    )
  expect(
    probe.call({
      op: 'edit',
      pin: button,
      edit: { kind: 'dispatchEvent', type: 'ping', init: {} },
    }),
  ).toEqual({ kind: 'value', value: null })
  expect(seen).toEqual(['ping bubbles=true'])
})

it('tests that an edit of a pin that is gone is detached', () => {
  const probe = booted()
  const button = pin(probe, '#go')
  probe.document.querySelector('#go')?.remove()
  expect(
    probe.call({ op: 'edit', pin: button, edit: { kind: 'blur' } }),
  ).toEqual({
    kind: 'detached',
  })
  expect(
    probe.call({
      op: 'edit',
      pin: button,
      edit: { kind: 'fill', value: 'x' },
    }),
  ).toEqual({ kind: 'detached' })
})

it("tests that fill, selectOptions and a file input refuse other elements with upstream's messages", () => {
  const probe = booted()
  const button = pin(probe, '#go')
  const refusal = (message: string) => ({
    kind: 'error',
    class: 'script',
    message,
  })
  expect(
    probe.call({
      op: 'edit',
      pin: button,
      edit: { kind: 'fill', value: 'x' },
    }),
  ).toEqual(
    refusal(
      'Element is not an <input>, <textarea> or [contenteditable] element',
    ),
  )
  expect(
    probe.call({
      op: 'edit',
      pin: button,
      edit: { kind: 'selectOptions', options: [{ value: 'a' }] },
    }),
  ).toEqual(refusal('Element is not a <select> element'))
  expect(
    probe.call({
      op: 'read',
      target: { pin: button },
      strictness: 'strict',
      frameVisible: true,
      read: { kind: 'fileInput', multiple: false, directory: false },
    }),
  ).toEqual(refusal('Node is not an HTMLInputElement'))
})
