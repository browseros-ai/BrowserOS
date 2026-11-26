// `privateMarks` on the built asset: every element of the document that carries the private
// entry's mark, pinned, for Rust's mirror of the marks (design 8.1). The walk is the DOM's: the
// document's elements in order, then each shadow root's, rendered or not.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <input id="first" type="text">
  <div id="host"></div>
  <input id="plain" type="text">
  <textarea id="last"></textarea>
</body></html>`

let page: ProbePage | undefined
afterEach(() => {
  page?.close()
  page = undefined
})

function booted(): ProbePage {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'private-marks' })
  return page
}

const field = (value: unknown, name: string): unknown =>
  typeof value === 'object' && value !== null
    ? Reflect.get(value, name)
    : undefined

/** What an earlier probe of this world left on a box: the registered key, unenumerable. */
function markDirectly(element: Element) {
  Object.defineProperty(element, Symbol.for('neo.privatelyTyped'), {
    value: true,
  })
}

function marked(probe: ProbePage): unknown[] {
  const reply = probe.call({ op: 'privateMarks' })
  expect(field(reply, 'kind')).toBe('marks')
  const pins = field(reply, 'pins')
  if (!Array.isArray(pins)) throw new Error(JSON.stringify(reply))
  return pins.map((pin) => probe.call({ op: 'pinned', pin }))
}

it('tests that a document nothing marked answers no pins', () => {
  expect(booted().call({ op: 'privateMarks' })).toEqual({
    kind: 'marks',
    pins: [],
  })
})

it("tests that the entry's own mark, an earlier probe's and one in an open shadow root are listed, the document's first", async () => {
  const probe = booted()
  const { document } = probe
  const byId = (id: string) => {
    const found = document.getElementById(id)
    if (!found) throw new Error(`no #${id}`)
    return found
  }
  const inner = document.createElement('input')
  byId('host').attachShadow({ mode: 'open' }).append(inner)
  markDirectly(inner)
  markDirectly(byId('last'))

  const resolved = probe.call({
    op: 'resolve',
    target: { chunk: parseSelector('#first') },
    strictness: 'strict',
    want: 'element',
  })
  const pin = Number(field(resolved, 'pin'))
  await probe.call({
    op: 'private',
    pin,
    step: { kind: 'focus', text: 'typed', spaced: false },
  })
  probe.call({ op: 'private', pin, step: { kind: 'forget' } })

  const listed = marked(probe)
  expect(listed).toHaveLength(3)
  expect(listed[0]).toBe(byId('first'))
  expect(listed[1]).toBe(byId('last'))
  expect(listed[2]).toBe(inner)
})

it('tests that every mark is pinned as one entry, so the batch is evicted together', () => {
  const probe = booted()
  for (const id of ['first', 'plain', 'last'])
    markDirectly(probe.document.getElementById(id) as Element)
  const reply = probe.call({ op: 'privateMarks' })
  const pins = field(reply, 'pins') as number[]
  const live = () =>
    pins.filter((pin) => probe.call({ op: 'pinned', pin }) !== null).length
  const pinOnce = () =>
    probe.call({
      op: 'resolve',
      target: { chunk: parseSelector('#plain') },
      strictness: 'strict',
      want: 'element',
    })
  for (let i = 0; i < 511; i++) pinOnce()
  expect(live()).toBe(3)
  pinOnce()
  expect(live()).toBe(0)
})

it("tests that a marked box in a slot's fallback content is listed while the slot shows assigned nodes", () => {
  const probe = booted()
  const { document } = probe
  const host = document.getElementById('host')
  if (!host) throw new Error('no #host')
  const assigned = document.createElement('span')
  host.append(assigned)
  const slot = document.createElement('slot')
  const fallback = document.createElement('input')
  slot.append(fallback)
  host.attachShadow({ mode: 'open' }).append(slot)
  // linkedom assigns no slots: give this one the host's child, as a browser would, so the
  // fallback is unrendered and a walk of the rendered tree would pass it by.
  Reflect.set(slot, 'assignedNodes', () => [assigned])
  markDirectly(fallback)
  expect(marked(probe)).toEqual([fallback])
})
