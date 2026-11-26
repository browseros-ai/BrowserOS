// `describePins` on the built asset: Rust decides a strict-mode violation on its own set and asks
// the page for that message's words. Assembled as Rust's `StrictViolation::message` assembles
// them, they must be `resolve`'s strict message for the same chunk and elements, byte for byte.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { PinPreview, SetMember } from '../src/probe/protocol'
import { openProbePage, type ProbePage } from './support/probe-page'

const ITEMS = Array.from(
  { length: 12 },
  (_, index) => `<li data-testid="item-${index}">Item ${index}</li>`,
).join('')

const PAGE = `<html><body>
  <button>Go</button><button id="stop" title="Halt">Stop</button>
  <form><input id="pw" type="password" value="hunter2"><input id="pin" type="password" value="1234"></form>
  <ul>${ITEMS}</ul>
</body></html>`

let page: ProbePage | undefined
afterEach(() => {
  page?.close()
  page = undefined
})

function booted(conformance = false): ProbePage {
  page?.close()
  page = openProbePage(PAGE, { conformance, discoverMovedFields: false })
  page.boot({ frameSeq: 0, epoch: 'describe-pins' })
  const view = page.document.defaultView
  if (!view) throw new Error('the fixture has a window')
  const style = () => ({
    display: 'block',
    visibility: 'visible',
    cursor: 'auto',
    content: 'none',
    getPropertyValue: () => '',
  })
  Reflect.set(view, 'getComputedStyle', style)
  Reflect.set(globalThis, 'getComputedStyle', style)
  Reflect.set(view.Element.prototype, 'checkVisibility', () => true)
  Reflect.set(view.Element.prototype, 'getBoundingClientRect', () => ({
    x: 0,
    y: 0,
    width: 10,
    height: 10,
  }))
  return page
}

const field = (value: unknown, name: string): unknown =>
  typeof value === 'object' && value !== null
    ? Reflect.get(value, name)
    : undefined

function pinsOf(probe: ProbePage, chunk: string): number[] {
  const reply = probe.call({
    op: 'readAll',
    target: { chunk: parseSelector(chunk) },
    read: 'pins',
  })
  const values = field(reply, 'values')
  if (!Array.isArray(values)) throw new Error(JSON.stringify(reply))
  return values.map((value) => Number(field(value, 'pin')))
}

function strictMessage(probe: ProbePage, chunk: string): string {
  const reply = probe.call({
    op: 'resolve',
    target: { chunk: parseSelector(chunk) },
    strictness: 'strict',
    want: 'element',
  })
  expect(field(reply, 'kind')).toBe('strict')
  return String(field(reply, 'message'))
}

/** Rust's `StrictViolation::message`: the head, a line per item, `...` when cut, a newline. */
function assemble(locator: string, count: number, items: PinPreview[]): string {
  const lines = items.map(
    ({ preview, aka }, index) => `\n    ${index + 1}) ${preview} aka ${aka}`,
  )
  if (lines.length < count) lines.push('\n    ...')
  return `strict mode violation: ${locator} resolved to ${count} elements:${lines.join('')}\n`
}

/** At most the ten elements upstream's message lists, as Rust hydrates its previews. */
function described(
  probe: ProbePage,
  chunk: string,
  members: (pins: number[]) => SetMember[],
  ...nodes: unknown[]
) {
  const pins = pinsOf(probe, chunk)
  const reply = probe.call(
    {
      op: 'describePins',
      members: members(pins.slice(0, 10)),
      chunk: parseSelector(chunk),
    },
    null,
    ...nodes,
  )
  expect(field(reply, 'kind')).toBe('pinsDescribed')
  const items = field(reply, 'items')
  if (!Array.isArray(items)) throw new Error(JSON.stringify(reply))
  return {
    count: pins.length,
    locator: String(field(reply, 'locator')),
    items: items as PinPreview[],
  }
}

it("tests that described pins assemble into resolve's own strict message, under either policy", () => {
  for (const conformance of [false, true]) {
    const probe = booted(conformance)
    for (const chunk of [
      'internal:role=button',
      'button',
      'input',
      'li',
      'internal:role=listitem >> internal:has-text="Item"i',
    ]) {
      const { count, locator, items } = described(probe, chunk, (pins) =>
        pins.map((pin) => ({ pin })),
      )
      expect(assemble(locator, count, items)).toBe(strictMessage(probe, chunk))
    }
  }
})

it('tests that a password field previews masked, as the strict message masks it', () => {
  const probe = booted()
  const { items } = described(probe, 'input', (pins) =>
    pins.map((pin) => ({ pin })),
  )
  expect(items.map(({ preview }) => preview)).toEqual([
    '<input id="pw" type="password" value="<7 characters>"/>',
    '<input id="pin" type="password" value="<4 characters>"/>',
  ])
  expect(JSON.stringify(items)).not.toContain('hunter2')
})

it('tests that handed nodes describe as the pins of the same elements do', () => {
  const probe = booted()
  const buttons = [...probe.document.querySelectorAll('button')]
  const byNode = described(
    probe,
    'button',
    () => buttons.map((_, node) => ({ node })),
    ...buttons,
  )
  const byPin = described(probe, 'button', (pins) =>
    pins.map((pin) => ({ pin })),
  )
  expect(byNode).toEqual(byPin)
})

it('tests that a member that is gone is detached, a document member is a probe bug, and no chunk sends no locator', () => {
  const probe = booted()
  const [go = 0, stop = 0] = pinsOf(probe, 'button')
  expect(
    probe.call({ op: 'describePins', members: [{ pin: go }, { pin: stop }] }),
  ).toEqual({
    kind: 'pinsDescribed',
    items: [
      {
        preview: '<button>Go</button>',
        aka: "getByRole('button', { name: 'Go' })",
      },
      {
        preview: '<button id="stop" title="Halt">Stop</button>',
        aka: "getByRole('button', { name: 'Stop' })",
      },
    ],
  })
  probe.document.getElementById('stop')?.remove()
  expect(
    probe.call({ op: 'describePins', members: [{ pin: go }, { pin: stop }] }),
  ).toEqual({ kind: 'detached' })
  expect(
    probe.call(
      { op: 'describePins', members: [{ node: 0 }] },
      null,
      probe.document.body.cloneNode(),
    ),
  ).toEqual({ kind: 'detached' })
  expect(
    probe.call({ op: 'describePins', members: ['document'] }),
  ).toMatchObject({ kind: 'error', class: 'internal' })
  expect(
    probe.call({ op: 'describePins', members: [], chunk: parseSelector('li') }),
  ).toEqual({
    kind: 'pinsDescribed',
    locator: "locator('li')",
    items: [],
  })
})
