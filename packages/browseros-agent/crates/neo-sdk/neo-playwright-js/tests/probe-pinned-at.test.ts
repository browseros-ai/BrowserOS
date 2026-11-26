// The `pinnedAt` request on the built asset: where a pinned element sits, which Rust reads by
// deep serialization to find the element's tree node. An `<option>` a select lists answers with
// that select, its index in `select.options` and the list's length, the address of the option's
// slot in Rust's tree; anything else answers with the element alone.
import { afterEach, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <select id="size">
    <optgroup label="Small"><option id="xs">XS</option><option id="s">S</option></optgroup>
    <option id="m" selected>M</option>
    <option id="l">L</option>
  </select>
  <select id="empty"></select>
  <datalist id="colors"><option id="red">Red</option></datalist>
  <div id="loose"><option id="stray">Stray</option></div>
  <button id="go">Go</button>
</body></html>`

let page: ProbePage | undefined
afterEach(() => {
  page?.close()
  page = undefined
})

function booted(): ProbePage {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'pinned-at' })
  return page
}

function element(probe: ProbePage, id: string): Element {
  const found = probe.document.getElementById(id)
  if (!found) throw new Error(`no #${id}`)
  return found
}

/** `#id` pinned, as a stage or a resolve pins what it matched. */
function pinOf(probe: ProbePage, id: string): number {
  const reply = probe.call({
    op: 'readAll',
    target: { chunk: parseSelector(`#${id}`) },
    read: 'pins',
  })
  const values: unknown =
    typeof reply === 'object' && reply !== null
      ? Reflect.get(reply, 'values')
      : undefined
  const [first]: unknown[] = Array.isArray(values) ? values : []
  const pin: unknown =
    typeof first === 'object' && first !== null
      ? Reflect.get(first, 'pin')
      : undefined
  if (typeof pin !== 'number')
    throw new Error(`#${id} pinned nothing: ${JSON.stringify(reply)}`)
  return pin
}

const placeOf = (probe: ProbePage, pin: number): unknown =>
  probe.call({ op: 'pinnedAt', pin })

it('tests that an option answers with its select, its index in select.options and the length', () => {
  const probe = booted()
  const size = element(probe, 'size')
  // select.options lists an optgroup's options where they stand: XS, S, M, L.
  for (const [id, index] of [
    ['xs', 0],
    ['s', 1],
    ['m', 2],
    ['l', 3],
  ] as const) {
    const place = placeOf(probe, pinOf(probe, id))
    expect(Array.isArray(place) && place.length).toBe(4)
    const [option, select, ...rest] = Array.isArray(place) ? place : []
    expect(option).toBe(element(probe, id))
    expect(select).toBe(size)
    expect(rest).toEqual([index, 4])
  }
})

it('tests that the length is the list now, so a list that changed since the capture shows', () => {
  const probe = booted()
  const pin = pinOf(probe, 'l')
  element(probe, 'xs').remove()
  const place = placeOf(probe, pin)
  expect(Array.isArray(place) ? place.slice(2) : place).toEqual([2, 3])
})

it('tests that an element no select lists answers with itself alone', () => {
  const probe = booted()
  // A datalist's option, an option outside any select, a select itself, any other element.
  for (const id of ['red', 'stray', 'size', 'empty', 'go']) {
    const place = placeOf(probe, pinOf(probe, id))
    expect(Array.isArray(place) && place.length).toBe(1)
    expect(Array.isArray(place) ? place[0] : place).toBe(element(probe, id))
  }
})

it('tests that a pin out of the document, gone or evicted has no place', () => {
  const probe = booted()
  const removed = pinOf(probe, 'm')
  const moved = pinOf(probe, 'go')
  element(probe, 'm').remove()
  expect(placeOf(probe, removed)).toBeNull()
  // Connected, but in another document, as one the page moved into a same-origin iframe: not
  // the element the capture saw (design 6.2). linkedom does not adopt a node into another
  // document, so the move is the two facts the probe reads changing.
  const other = parseHTML('<html><body></body></html>').document
  Object.defineProperties(element(probe, 'go'), {
    ownerDocument: { get: () => other },
    isConnected: { get: () => true },
  })
  expect(placeOf(probe, moved)).toBeNull()
  expect(placeOf(probe, 999_999)).toBeNull()
})

it('tests that a world without a booted probe answers pinnedAt with the marker object', () => {
  page = openProbePage(PAGE)
  // By handle, like `pinned`: Rust reads an object, never JSON text, by deep serialization.
  expect(page.rawCall({ op: 'pinnedAt', pin: 1 })).toEqual({
    kind: 'uninstalled',
  })
})
