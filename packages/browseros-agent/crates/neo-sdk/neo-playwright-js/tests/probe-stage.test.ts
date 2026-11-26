// The `stage` request on the built asset: a page stage that starts from the document, from pins
// or from handed nodes, with tree parts standing in as call-scoped candidate sets that upstream's
// own query loop evaluates. The candidate sets come from the bundle's real role engine over the
// whole document, as Rust's tree hands them over the APC tree.
import { afterEach, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import {
  type ParsedSelector,
  type ParsedSelectorPart,
  parseSelector,
} from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { SetMember, StageSet } from '../src/probe/protocol'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <div id="plans">
    <section id="free"><span>Free</span><button id="free-edit">Edit</button></section>
    <section id="pro"><span>Pro</span><button id="pro-edit">Edit</button><button id="pro-delete">Delete</button></section>
  </div>
  <button id="outside">Edit</button>
  <div id="host"></div>
  <section id="framed"><iframe id="pay" src="https://pay.example/frame"></iframe></section>
</body></html>`

let page: ProbePage | undefined
afterEach(() => {
  page?.close()
  page = undefined
})

/** A visible box for every element, shadow content included, so the role engine sees them. */
function showFixture(probe: ProbePage) {
  const view = probe.document.defaultView
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
}

function booted(): ProbePage {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'stage' })
  showFixture(page)
  const host = page.document.getElementById('host')
  if (!host) throw new Error('the fixture has a host')
  host.attachShadow({ mode: 'open' }).innerHTML =
    '<button id="shadow-edit">Edit</button>'
  return page
}

const field = (value: unknown, name: string): unknown =>
  typeof value === 'object' && value !== null
    ? Reflect.get(value, name)
    : undefined

function element(probe: ProbePage, id: string): Element {
  const found =
    probe.document.getElementById(id) ??
    probe.document.getElementById('host')?.shadowRoot?.getElementById(id)
  if (!found) throw new Error(`no #${id}`)
  return found
}

/** Every match of `chunk` from the document, pinned, as `readAll { read: pins }` answers. */
function pinsOf(probe: ProbePage, chunk: ParsedSelector | string): number[] {
  const reply = probe.call({
    op: 'readAll',
    target: {
      chunk: typeof chunk === 'string' ? parseSelector(chunk) : chunk,
    },
    read: 'pins',
  })
  const values = field(reply, 'values')
  if (!Array.isArray(values)) throw new Error(JSON.stringify(reply))
  return values.map((value) => Number(field(value, 'pin')))
}

const pinned = (probe: ProbePage, pin: number): unknown =>
  probe.call({ op: 'pinned', pin })

function elementsOf(probe: ProbePage, chunk: ParsedSelector | string) {
  return pinsOf(probe, chunk).map((pin) => pinned(probe, pin))
}

/** A candidate set: a role (or ref) chunk's whole-document matches, as Rust hydrates them. */
function candidates(
  probe: ProbePage,
  chunk: ParsedSelector | string,
): StageSet {
  return { members: pinsOf(probe, chunk).map((pin) => ({ pin })) }
}

const pinMember = (probe: ProbePage, id: string): SetMember => ({
  pin: pinsOf(probe, `#${id}`)[0] ?? 0,
})

const below = (index: number, source = 'button'): ParsedSelectorPart => ({
  name: 'internal:neo-below',
  body: String(index),
  source,
})
const inside = (index: number, source = 'e1'): ParsedSelectorPart => ({
  name: 'internal:neo-inside',
  body: String(index),
  source,
})
const nested = (name: string, parsed: ParsedSelector): ParsedSelectorPart => ({
  name,
  body: { parsed },
  source: JSON.stringify(parsed),
})
const selector = (...parts: (ParsedSelectorPart | string)[]) => ({
  parts: parts.flatMap((part) =>
    typeof part === 'string' ? parseSelector(part).parts : [part],
  ),
})

/** The elements a `staged` reply names, the document as itself; anything else throws. */
function staged(probe: ProbePage, reply: unknown): unknown[] {
  if (field(reply, 'kind') !== 'staged') throw new Error(JSON.stringify(reply))
  const matches = field(reply, 'matches')
  if (!Array.isArray(matches)) throw new Error(JSON.stringify(reply))
  return matches.map((match) =>
    match === 'document'
      ? probe.document
      : pinned(probe, Number(field(match, 'pin'))),
  )
}

/** The same nodes, by identity: `toEqual` would take two look-alike elements as one. */
function expectSame(actual: unknown[], expected: unknown[], label = '') {
  expect({ label, length: actual.length }).toEqual({
    label,
    length: expected.length,
  })
  actual.forEach((node, index) => {
    expect(node).toBe(expected[index])
  })
}

const ids = (elements: unknown[]) =>
  elements.map((each) =>
    each instanceof Element
      ? each.id
      : each === page?.document
        ? 'document'
        : '?',
  )

it('tests that a stage from the document, from pins and from handed nodes runs every part per root', () => {
  const probe = booted()
  const edits = candidates(probe, 'internal:role=button[name="Edit"i]')
  const fromDocument = probe.call({
    op: 'stage',
    selector: selector('css=section', below(0)),
    sets: [edits],
  })
  expect(ids(staged(probe, fromDocument))).toEqual(['free-edit', 'pro-edit'])

  const fromPins = probe.call({
    op: 'stage',
    selector: selector(below(0)),
    from: [pinMember(probe, 'pro'), pinMember(probe, 'free')],
    sets: [edits],
  })
  expect(ids(staged(probe, fromPins))).toEqual(['pro-edit', 'free-edit'])

  const fromNodes = probe.call(
    {
      op: 'stage',
      selector: selector(below(0)),
      from: [{ node: 1 }, { node: 0 }],
      sets: [edits],
    },
    null,
    element(probe, 'free'),
    element(probe, 'pro'),
  )
  expect(ids(staged(probe, fromNodes))).toEqual(['pro-edit', 'free-edit'])

  const fromTheDocumentMember = probe.call({
    op: 'stage',
    selector: selector(below(0)),
    from: ['document'],
    sets: [edits],
  })
  expect(ids(staged(probe, fromTheDocumentMember))).toEqual([
    'free-edit',
    'pro-edit',
    'outside',
    'shadow-edit',
  ])
})

it('tests that every match is pinned as one entry, so the batch is evicted together', () => {
  const probe = booted()
  const reply = probe.call({
    op: 'stage',
    selector: selector(below(0)),
    sets: [candidates(probe, 'internal:role=button')],
  })
  const matches = Reflect.get(Object(reply), 'matches')
  expect(matches).toHaveLength(5)
  const live = () =>
    staged(probe, reply).filter((match) => match !== null).length
  // The pin table keeps its last 512 entries: the batch is one of them until the 512th after it.
  for (let i = 0; i < 511; i++) pinsOf(probe, '#outside')
  expect(live()).toBe(5)
  pinsOf(probe, '#outside')
  expect(live()).toBe(0)
})

it('tests that neo-below leaves the root out and neo-inside keeps it, through shadow roots', () => {
  const probe = booted()
  const set = {
    members: [
      pinMember(probe, 'pro'),
      pinMember(probe, 'pro-edit'),
      pinMember(probe, 'shadow-edit'),
    ],
  }
  const from = (id: string, part: ParsedSelectorPart) =>
    ids(
      staged(
        probe,
        probe.call({
          op: 'stage',
          selector: selector(part),
          from: [pinMember(probe, id)],
          sets: [set],
        }),
      ),
    )
  expect(from('pro', below(0))).toEqual(['pro-edit'])
  expect(from('pro', inside(0))).toEqual(['pro', 'pro-edit'])
  expect(from('host', below(0))).toEqual(['shadow-edit'])
  expect(from('free', inside(0))).toEqual([])
  const document = probe.call({
    op: 'stage',
    selector: selector(below(0)),
    sets: [set],
  })
  expect(ids(staged(probe, document))).toEqual([
    'pro',
    'pro-edit',
    'shadow-edit',
  ])
})

it("tests that nth runs over the whole set and and/or from the chunk's query root, as upstream's loop runs them", () => {
  const probe = booted()
  const edits = candidates(probe, 'internal:role=button[name="Edit"i]')
  const rows = [pinMember(probe, 'free'), pinMember(probe, 'pro')]
  const nth = probe.call({
    op: 'stage',
    selector: selector(below(0), 'nth=1'),
    from: rows,
    sets: [edits],
  })
  // One call per root would take each root's second match, of which there is none.
  expect(ids(staged(probe, nth))).toEqual(['pro-edit'])

  // `internal:or` queries its nested selector from the call's root, the document, not from
  // `from`'s members: the free row joins every Edit button in the document.
  const or = probe.call({
    op: 'stage',
    selector: selector(nested('internal:or', selector(below(0)))),
    from: [pinMember(probe, 'free')],
    sets: [edits],
  })
  expect(ids(staged(probe, or))).toEqual([
    'free',
    'free-edit',
    'pro-edit',
    'outside',
    'shadow-edit',
  ])

  const and = probe.call({
    op: 'stage',
    selector: selector(
      'css=button',
      nested('internal:and', selector(below(0))),
    ),
    from: rows,
    sets: [edits],
  })
  expect(ids(staged(probe, and))).toEqual(['free-edit', 'pro-edit'])

  // From an element handle's root, `or` reaches only what is under that root.
  const handle = probe.call(
    {
      op: 'stage',
      selector: selector('css=span', nested('internal:or', selector(below(0)))),
      sets: [edits],
    },
    element(probe, 'pro'),
  )
  expectSame(staged(probe, handle), [
    element(probe, 'pro').querySelector('span'),
    element(probe, 'pro-edit'),
  ])
})

it('tests that has, has-not and chain evaluate their candidate parts per root', () => {
  const probe = booted()
  const deletes = candidates(probe, 'internal:role=button[name="Delete"i]')
  const has = probe.call({
    op: 'stage',
    selector: selector(
      'css=section',
      nested('internal:has', selector(below(0))),
    ),
    sets: [deletes],
  })
  expect(ids(staged(probe, has))).toEqual(['pro'])
  const hasNot = probe.call({
    op: 'stage',
    selector: selector(
      'css=section',
      nested('internal:has-not', selector(below(0))),
    ),
    sets: [deletes],
  })
  expect(ids(staged(probe, hasNot))).toEqual(['free', 'framed'])
  const chain = probe.call({
    op: 'stage',
    selector: selector(nested('internal:chain', selector(below(0)))),
    from: [pinMember(probe, 'pro'), pinMember(probe, 'free')],
    sets: [candidates(probe, 'internal:role=button')],
  })
  expect(ids(staged(probe, chain))).toEqual([
    'pro-edit',
    'pro-delete',
    'free-edit',
  ])
})

it('tests that a capture counts the prepended part, so the captured element is what matches', () => {
  const probe = booted()
  const reply = probe.call({
    op: 'stage',
    selector: parseSelector('*css=section >> internal:neo-below=0'),
    from: [pinMember(probe, 'plans')],
    sets: [candidates(probe, 'internal:role=button[name="Delete"i]')],
  })
  expect(ids(staged(probe, reply))).toEqual(['pro'])
})

it('tests that an error set throws with the class Rust sent, only once a root reaches it', () => {
  const probe = booted()
  const errors: Extract<StageSet, { error: unknown }>['error'][] = [
    {
      message:
        'Unknown attribute "pressd", must be one of "checked", "description", "disabled", "expanded", "include-hidden", "level", "name", "pressed", "selected".',
      class: 'script',
    },
    {
      message: 'Unexpected end of selector while parsing selector `button[`',
      class: 'selector',
    },
  ]
  for (const error of errors) {
    const sets: StageSet[] = [{ error }]
    for (const unreached of [
      selector('css=#missing', below(0)),
      selector('css=#missing', nested('internal:has', selector(below(0)))),
      selector('css=section'),
    ])
      expect(
        field(probe.call({ op: 'stage', selector: unreached, sets }), 'kind'),
      ).toBe('staged')
    expect(
      probe.call({
        op: 'stage',
        selector: selector(
          'css=section',
          nested('internal:has', selector(below(0))),
        ),
        sets,
      }),
    ).toEqual({ kind: 'error', class: error.class, message: error.message })
  }
})

it('tests that a stale-ref set matches nothing and the reply names the first one a root reached', () => {
  const probe = booted()
  const stale: StageSet[] = [{ staleRef: 'e8' }, { staleRef: 'e9' }]
  const stage = (...parts: (ParsedSelectorPart | string)[]) =>
    probe.call({ op: 'stage', selector: selector(...parts), sets: stale })

  for (const unreached of [
    stage('css=#missing', inside(0)),
    stage('css=#missing', nested('internal:has', selector(inside(0)))),
  ])
    expect(unreached).toEqual({ kind: 'staged', matches: [] })
  const unnamed = stage('css=#outside')
  expect(field(unnamed, 'staleRef')).toBeUndefined()
  expect(ids(staged(probe, unnamed))).toEqual(['outside'])

  // The has-not keeps every section, so a reached stale ref rides along with matches.
  const kept = stage(
    'css=section',
    nested('internal:has-not', selector(inside(0))),
  )
  expect(field(kept, 'staleRef')).toBe('e8')
  expect(ids(staged(probe, kept))).toEqual(['free', 'pro', 'framed'])

  // Set 1 is reached before set 0, so its ref is the one named: reach order, not set order.
  const first = stage(
    'css=section',
    nested('internal:has-not', selector(inside(1))),
    inside(0),
  )
  expect(first).toEqual({ kind: 'staged', matches: [], staleRef: 'e9' })
})

it('tests that a from member that is gone, removed or in another document is detached, and a gone candidate is dropped', () => {
  const probe = booted()
  const edits = candidates(probe, 'internal:role=button[name="Edit"i]')
  const stageFrom = (from: SetMember[], ...nodes: unknown[]) =>
    probe.call(
      { op: 'stage', selector: selector(below(0)), from, sets: [edits] },
      null,
      ...nodes,
    )

  const evicted = pinMember(probe, 'free')
  for (let i = 0; i < 512; i++) pinsOf(probe, '#outside')
  expect(stageFrom([evicted, pinMember(probe, 'pro')])).toEqual({
    kind: 'detached',
  })

  const removed = pinMember(probe, 'pro')
  const removedEdit = pinMember(probe, 'pro-edit')
  element(probe, 'pro').remove()
  expect(stageFrom([removed])).toEqual({ kind: 'detached' })
  expect(stageFrom([{ node: 0 }], element(probe, 'free').cloneNode())).toEqual({
    kind: 'detached',
  })

  // A node the page adopted into another same-origin document keeps its wrapper and is
  // connected there, but it is not the element this world's capture saw (design 6.2). linkedom
  // cannot adopt, so the element answers as an adopted one would.
  const free = element(probe, 'free')
  const adopted = pinMember(probe, 'free')
  const other = parseHTML('<html><body></body></html>').document
  const own = ['ownerDocument', 'isConnected'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(free, name)] as const,
  )
  for (const [name, value] of [
    ['ownerDocument', other],
    ['isConnected', true],
  ] as const)
    Object.defineProperty(free, name, { configurable: true, value })
  expect(stageFrom([adopted])).toEqual({ kind: 'detached' })
  expect(stageFrom([{ node: 0 }], free)).toEqual({ kind: 'detached' })
  expect(stageFrom([{ node: 3 }])).toEqual({ kind: 'detached' })
  for (const [name, descriptor] of own)
    if (descriptor) Object.defineProperty(free, name, descriptor)
    else Reflect.deleteProperty(free, name)

  // A candidate set naming an evicted pin and a removed element: both match nothing.
  const reply = probe.call({
    op: 'stage',
    selector: selector(below(0)),
    sets: [
      {
        members: [
          evicted,
          removedEdit,
          pinMember(probe, 'free-edit'),
          pinMember(probe, 'shadow-edit'),
        ],
      },
    ],
  })
  expect(ids(staged(probe, reply))).toEqual(['free-edit', 'shadow-edit'])
})

it('tests that the call-scoped engines are gone after the call, even one that threw', () => {
  const probe = booted()
  const neoChunk = selector(below(0))
  const resolveNeo = () =>
    probe.call({
      op: 'resolve',
      target: { chunk: neoChunk },
      strictness: 'firstMatch',
      want: 'element',
    })
  const unknown = {
    kind: 'error',
    class: 'selector',
    message: expect.stringContaining('Unknown engine "internal:neo-below"'),
  }
  expect(resolveNeo()).toMatchObject(unknown)
  probe.call({
    op: 'stage',
    selector: neoChunk,
    from: [pinMember(probe, 'pro')],
    sets: [candidates(probe, 'internal:role=button')],
  })
  expect(resolveNeo()).toMatchObject(unknown)
  expect(
    probe.call({
      op: 'stage',
      selector: neoChunk,
      sets: [{ error: { message: 'bad', class: 'script' } }],
    }),
  ).toMatchObject({ kind: 'error', class: 'script', message: 'bad' })
  expect(resolveNeo()).toMatchObject(unknown)
  expect(
    probe.call({
      op: 'resolve',
      target: {
        chunk: { parts: [{ name: 'internal:neo-from', body: '', source: '' }] },
      },
      strictness: 'firstMatch',
      want: 'element',
    }),
  ).toMatchObject({
    kind: 'error',
    class: 'selector',
    message: expect.stringContaining('Unknown engine "internal:neo-from"'),
  })
})

it('tests that a body naming no set, or a selector naming internal:neo-from, is a probe bug', () => {
  const probe = booted()
  for (const bad of [
    { op: 'stage', selector: selector(below(1)), sets: [{ members: [] }] },
    { op: 'stage', selector: selector(below(0)) },
    {
      op: 'stage',
      selector: selector({
        name: 'internal:neo-below',
        body: '01',
        source: '',
      }),
      sets: [{ members: [] }],
    },
    {
      op: 'stage',
      selector: selector(
        'css=section',
        nested(
          'internal:has',
          selector({ name: 'internal:neo-from', body: '', source: '' }),
        ),
      ),
      from: ['document'],
    },
  ])
    expect(probe.call(bad)).toMatchObject({ kind: 'error', class: 'internal' })
})

it('tests that nth can keep the document, which answers as the document marker', () => {
  const probe = booted()
  expect(probe.call({ op: 'stage', selector: selector('nth=0') })).toEqual({
    kind: 'staged',
    matches: ['document'],
  })
  const or = probe.call({
    op: 'stage',
    selector: selector(nested('internal:or', selector('css=#outside'))),
    from: ['document'],
  })
  expect(ids(staged(probe, or))).toEqual(['document', 'outside'])
})

it("tests that a strict stage with several matches answers upstream's own strict message for the chunk", () => {
  const probe = booted()
  const chunk = parseSelector('section >> internal:role=button[name="Edit"i]')
  const resolved = probe.call({
    op: 'resolve',
    target: { chunk },
    strictness: 'strict',
    want: 'element',
  })
  expect(field(resolved, 'kind')).toBe('strict')
  const reply = probe.call({
    op: 'stage',
    selector: selector('css=section', below(0, 'button[name="Edit"i]')),
    sets: [candidates(probe, 'internal:role=button[name="Edit"i]')],
    strict: chunk,
  })
  expect(reply).toEqual({ kind: 'strict', message: field(resolved, 'message') })
  const one = probe.call({
    op: 'stage',
    selector: selector('css=#pro', below(0)),
    sets: [candidates(probe, 'internal:role=button[name="Edit"i]')],
    strict: chunk,
  })
  expect(ids(staged(probe, one))).toEqual(['pro-edit'])
})

/**
 * The page form Rust's ChunkPlan builds: every tree leaf, at any depth, becomes a call-scoped
 * engine over its candidate set, which the real role engine (or Neo's ref engine) computes over
 * the whole document.
 */
function pageForm(
  probe: ProbePage,
  chunk: ParsedSelector,
  sets: StageSet[],
): ParsedSelector {
  return {
    ...chunk,
    parts: chunk.parts.map((part): ParsedSelectorPart => {
      const leaf =
        part.name === 'internal:role' || part.name === 'role'
          ? 'internal:neo-below'
          : part.name === 'aria-ref'
            ? 'internal:neo-inside'
            : undefined
      if (leaf) {
        sets.push(candidates(probe, { parts: [part] }))
        return {
          name: leaf,
          body: String(sets.length - 1),
          source: part.source,
        }
      }
      const body = part.body
      if (typeof body === 'object' && body !== null && 'parsed' in body)
        return {
          ...part,
          body: { ...body, parsed: pageForm(probe, body.parsed, sets) },
        }
      return part
    }),
  }
}

it('tests that the page form of mixed chunks matches what the engine itself matches', () => {
  const probe = booted()
  const frame = probe.call({
    op: 'resolve',
    target: { chunk: parseSelector('#pay') },
    strictness: 'strict',
    want: 'frame',
  })
  expect(field(frame, 'key')).toBe('#1')
  for (const text of [
    'section >> internal:role=button[name="Edit"i]',
    'internal:role=button >> nth=2',
    'section >> internal:has="internal:role=button[name=\\"Delete\\"i]" >> internal:role=button',
    'section >> internal:has-not="internal:role=button[name=\\"Delete\\"i]"',
    'internal:role=button[name="Edit"i] >> internal:or="css=span"',
    'css=button >> internal:and="internal:role=button[name=\\"Edit\\"i]"',
    'div >> internal:chain="section >> internal:role=button"',
    'section >> internal:has="aria-ref=#1"',
    '#plans >> internal:role=button >> nth=-1',
  ]) {
    const chunk = parseSelector(text)
    const sets: StageSet[] = []
    const reply = probe.call({
      op: 'stage',
      selector: pageForm(probe, chunk, sets),
      sets,
    })
    const expected = elementsOf(probe, chunk)
    expect(expected.length).toBeGreaterThan(0)
    expectSame(staged(probe, reply), expected, text)
  }
})
