// The probe's reads through the built asset, on a linkedom page: each read kind's value and
// upstream's thrown messages, and the seal's verdict on the element a value came from.
// linkedom has no layout and does not reflect `type` or `checked` as properties, so boxes,
// `editable` and `checked` are measured by reads_live.rs on a real page instead.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <main id="main"><h1 title="Greeting">Hello <b>there</b></h1></main>
  <label id="pw-label">Password <input id="pw" type="password" value="hunter2"></label>
  <input id="name" value="Ada">
  <svg id="logo"><title>Logo</title></svg>
  <iframe id="pay" src="https://pay.example/frame"></iframe>
  <button id="off" disabled>Off</button>
</body></html>`

let page: ProbePage | undefined
afterEach(() => page?.close())

function booted(): ProbePage {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'read-test' })
  return page
}

const read = (probe: ProbePage, selector: string, kind: object) =>
  probe.call({
    op: 'read',
    target: { chunk: parseSelector(selector) },
    strictness: 'strict',
    frameVisible: true,
    read: kind,
  })

it('tests that the text reads answer with the element and no seal verdict', () => {
  const probe = booted()
  expect(read(probe, 'h1', { kind: 'innerHtml' })).toEqual({
    kind: 'read',
    value: 'Hello <b>there</b>',
    sensitive: null,
    sweep: ['hunter2'],
  })
  expect(read(probe, 'h1', { kind: 'innerText' })).toEqual({
    kind: 'read',
    value: 'Hello there',
    sensitive: null,
    sweep: ['hunter2'],
  })
  expect(read(probe, 'h1', { kind: 'attribute', name: 'title' })).toEqual({
    kind: 'read',
    value: 'Greeting',
    sensitive: null,
    sweep: ['hunter2'],
  })
  expect(read(probe, 'h1', { kind: 'attribute', name: 'lang' })).toEqual({
    kind: 'read',
    value: null,
    sensitive: null,
    sweep: ['hunter2'],
  })
  expect(read(probe, '#name', { kind: 'inputValue' })).toEqual({
    kind: 'read',
    value: 'Ada',
    sensitive: null,
    sweep: ['hunter2'],
  })
})

it('tests that reads with page text carry the sweep and the others carry none', () => {
  const probe = booted()
  const sweepOf = (selector: string, kind: object) =>
    (read(probe, selector, kind) as { sweep?: unknown }).sweep
  for (const kind of [
    { kind: 'textContent' },
    { kind: 'innerText' },
    { kind: 'innerHtml' },
    { kind: 'attribute', name: 'title' },
    { kind: 'located' },
  ])
    expect(sweepOf('h1', kind)).toEqual(['hunter2'])
  expect(sweepOf('#name', { kind: 'inputValue' })).toEqual(['hunter2'])
  for (const kind of [
    { kind: 'state', state: 'visible' },
    { kind: 'boundingBox' },
  ])
    expect(sweepOf('h1', kind)).toEqual([])
  expect(sweepOf('#pay', { kind: 'frameOrigin' })).toEqual([])
})

it("tests that innerText and inputValue throw upstream's messages as script errors", () => {
  const probe = booted()
  expect(read(probe, '#logo', { kind: 'innerText' })).toEqual({
    kind: 'error',
    class: 'script',
    message: 'Node is not an HTMLElement',
  })
  expect(read(probe, 'h1', { kind: 'inputValue' })).toEqual({
    kind: 'error',
    class: 'script',
    message: 'Node is not an <input>, <textarea> or <select> element',
  })
})

it('tests that a sensitive field is reported as the element the value came from', () => {
  const probe = booted()
  expect(read(probe, '#pw', { kind: 'inputValue' })).toMatchObject({
    kind: 'read',
    sensitive: 'password',
  })
  expect(read(probe, '#pw-label', { kind: 'textContent' })).toMatchObject({
    kind: 'read',
    sensitive: null,
  })
})

it("tests that state reads are upstream's elementState, frames' visibility included", () => {
  const probe = booted()
  const state = (selector: string, name: string, frameVisible = true) =>
    probe.call({
      op: 'read',
      target: { chunk: parseSelector(selector) },
      strictness: 'strict',
      frameVisible,
      read: { kind: 'state', state: name },
    })
  const value = (reply: unknown) => (reply as { value: unknown }).value
  expect(value(state('#off', 'disabled'))).toBe(true)
  expect(value(state('#off', 'enabled'))).toBe(false)
  expect(value(state('h1', 'visible'))).toBe(true)
  expect(value(state('h1', 'visible', false))).toBe(false)
  expect(value(state('h1', 'hidden', false))).toBe(true)
  expect(state('h1', 'checked')).toEqual({
    kind: 'error',
    class: 'script',
    message: 'Not a checkbox or radio button',
  })
})

it("tests that the checkable read carries no sweep and throws upstream's message", () => {
  const probe = booted()
  // linkedom has no `checked` property, so the values themselves are read by actions_live.rs.
  expect(read(probe, 'h1', { kind: 'checkable' })).toEqual({
    kind: 'error',
    class: 'script',
    message: 'Not a checkbox or radio button',
  })
})

it('tests that readAll reads every match in document order, and nothing for none', () => {
  const probe = booted()
  const every = (selector: string, kind: string) =>
    probe.call({
      op: 'readAll',
      target: { chunk: parseSelector(selector) },
      read: kind,
    })
  expect(every('h1, #pw-label', 'textContents')).toEqual({
    kind: 'values',
    count: 2,
    values: ['Hello there', 'Password '],
    sweep: ['hunter2'],
  })
  expect(every('h1, #name', 'innerTexts')).toEqual({
    kind: 'values',
    count: 2,
    values: ['Hello there', ''],
    sweep: ['hunter2'],
  })
  expect(every('#missing', 'innerTexts')).toEqual({
    kind: 'values',
    count: 0,
    values: [],
    sweep: ['hunter2'],
  })
})

it('tests that located pins the first match and counts the matches of a non-strict wait', () => {
  const probe = booted()
  const locate = (selector: string, strictness: string) =>
    probe.call({
      op: 'read',
      target: { chunk: parseSelector(selector) },
      strictness,
      frameVisible: true,
      read: { kind: 'located' },
    })
  const first = locate('input', 'firstMatch') as {
    value: { pin: unknown; visible: boolean; count: number }
  }
  expect(first).toMatchObject({
    kind: 'read',
    value: { visible: true, count: 2 },
    sensitive: 'password',
  })
  expect(typeof first.value.pin).toBe('number')
  expect(locate('h1', 'strict')).toMatchObject({ value: { count: 1 } })
  expect(locate('h1, b', 'strict')).toMatchObject({ kind: 'strict' })
  expect(locate('#missing', 'firstMatch')).toEqual({ kind: 'none' })
})

it("tests that pins carry each match's preview for its handle", () => {
  const probe = booted()
  const pins = probe.call({
    op: 'readAll',
    target: { chunk: parseSelector('input') },
    read: 'pins',
  }) as { count: number; values: { pin: unknown; preview: string }[] }
  expect(pins.count).toBe(2)
  expect(pins.values.map((match) => typeof match.pin)).toEqual([
    'number',
    'number',
  ])
  expect(pins.values[1]?.preview).toBe('<input id="name" value="Ada"/>')
})

it("tests that a root target is the handle's own node, detached once it leaves", () => {
  const probe = booted()
  const name = probe.document.getElementById('name') ?? undefined
  const readValue = () =>
    probe.call(
      {
        op: 'read',
        target: 'root',
        strictness: 'strict',
        frameVisible: true,
        read: { kind: 'inputValue' },
      },
      name,
    )
  expect(readValue()).toEqual({
    kind: 'read',
    value: 'Ada',
    sensitive: null,
    sweep: ['hunter2'],
  })
  name?.remove()
  expect(readValue()).toEqual({ kind: 'detached' })
})

it('tests that boxes and frame origins are null without a layout box', () => {
  const probe = booted()
  expect(read(probe, 'h1', { kind: 'boundingBox' })).toEqual({
    kind: 'read',
    value: null,
    sensitive: null,
    sweep: [],
  })
  expect(read(probe, '#pay', { kind: 'frameOrigin' })).toEqual({
    kind: 'read',
    value: null,
    sensitive: null,
    sweep: [],
  })
  expect(read(probe, 'h1', { kind: 'frameOrigin' })).toEqual({
    kind: 'error',
    class: 'internal',
    message: 'frameOrigin reads an <iframe> or <frame>',
  })
})
