// The built probe's lifecycle and target resolution, through the requests Rust sends:
// boot and re-key, the pin table, Neo's aria-ref engine (reached through frame keys, since
// snapshots mint refs only on a real layout) and the error replies.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { PROTOCOL } from '../src/probe/protocol'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <button>Go</button><button>Stop</button>
  <input id="pw" type="password" value="hunter2">
  <div id="outside"></div>
  <section id="checkout"><iframe id="pay" src="https://pay.example/frame"></iframe></section>
</body></html>`

let page: ProbePage | undefined
afterEach(() => page?.close())

const open = () => {
  page = openProbePage(PAGE)
  return page
}

const resolveRequest = (
  selector: string,
  extra: {
    strictness?: 'strict' | 'firstMatch'
    want?: 'element' | 'frame'
  } = {},
) => ({
  op: 'resolve',
  target: { chunk: parseSelector(selector) },
  strictness: extra.strictness ?? 'strict',
  want: extra.want ?? 'element',
})

const pinOf = (reply: unknown): number => {
  const pin: unknown =
    typeof reply === 'object' && reply !== null
      ? Reflect.get(reply, 'pin')
      : undefined
  if (typeof pin !== 'number')
    throw new Error(`no pin in ${JSON.stringify(reply)}`)
  return pin
}

it('tests that a world answers uninstalled until boot', () => {
  const probe = open()
  expect(probe.call({ op: 'describe' })).toEqual({ kind: 'uninstalled' })
  probe.boot({ frameSeq: 0, epoch: 'a' })
  expect(probe.call({ op: 'describe' })).toMatchObject({ kind: 'described' })
})

it('tests that boot rejects options Rust would never send', () => {
  const probe = open()
  for (const options of [
    undefined,
    { frameSeq: -1, epoch: 'a' },
    { frameSeq: 1.5, epoch: 'a' },
    { frameSeq: 0, epoch: '' },
  ])
    expect(() => probe.boot(options)).toThrow('boot takes')
})

it('tests that describe reports the protocol, the seq and the url', () => {
  const probe = open()
  const described = {
    kind: 'described',
    protocol: PROTOCOL,
    seq: 4,
    url: 'https://fixture.test/page',
  }
  expect(probe.boot({ frameSeq: 4, epoch: 'a' })).toEqual(described)
  expect(probe.call({ op: 'describe' })).toEqual(described)
})

it('tests that a boot with the same epoch keeps the world, and another epoch re-keys it', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'first' })
  const frame = probe.call(resolveRequest('#pay', { want: 'frame' }))
  expect(frame).toMatchObject({ kind: 'frame', key: '#1' })
  const pin = pinOf(frame)

  expect(probe.boot({ frameSeq: 9, epoch: 'first' })).toEqual({
    kind: 'described',
    protocol: PROTOCOL,
    seq: 0,
    url: 'https://fixture.test/page',
  })
  expect(probe.call(resolveRequest('aria-ref=#1'))).toMatchObject({
    kind: 'element',
  })
  expect(
    probe.call({
      op: 'resolve',
      target: { pin },
      strictness: 'strict',
      want: 'element',
    }),
  ).toMatchObject({ kind: 'element' })

  expect(probe.boot({ frameSeq: 347, epoch: 'second' })).toMatchObject({
    seq: 347,
    retired: 0,
  })
  expect(probe.call(resolveRequest('aria-ref=#1'))).toEqual({
    kind: 'none',
    staleRef: '#1',
  })
  expect(
    probe.call({
      op: 'resolve',
      target: { pin },
      strictness: 'strict',
      want: 'element',
    }),
  ).toEqual({ kind: 'detached' })
  const again = probe.call(resolveRequest('#pay', { want: 'frame' }))
  expect(again).toMatchObject({ kind: 'frame', key: '#2' })
  expect(pinOf(again)).toBeGreaterThan(pin)
})

it('tests that resolve pins one element, or reports none, strict or not-a-frame', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  expect(probe.call(resolveRequest('#pw'))).toMatchObject({
    kind: 'element',
    preview: '<input id="pw" type="password" value="<7 characters>"/>',
  })
  expect(
    probe.call(resolveRequest('button', { strictness: 'firstMatch' })),
  ).toMatchObject({
    kind: 'element',
    preview: '<button>Go</button>',
  })
  const strict = probe.call(resolveRequest('button'))
  expect(strict).toMatchObject({ kind: 'strict' })
  expect(String(Reflect.get(Object(strict), 'message'))).toStartWith(
    "strict mode violation: locator('button') resolved to 2 elements:",
  )
  expect(probe.call(resolveRequest('#missing'))).toEqual({ kind: 'none' })
  expect(probe.call(resolveRequest('#outside', { want: 'frame' }))).toEqual({
    kind: 'notFrame',
    preview: '<div id="outside"></div>',
  })
  expect(probe.call(resolveRequest('#pay', { want: 'frame' }))).toMatchObject({
    kind: 'frame',
    src: 'https://pay.example/frame',
  })
})

it("tests that Neo's aria-ref engine answers inside the query root and fails a removed ref at once", () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  probe.call(resolveRequest('#pay', { want: 'frame' }))
  expect(probe.call(resolveRequest('#checkout >> aria-ref=#1'))).toMatchObject({
    kind: 'element',
  })
  expect(probe.call(resolveRequest('#outside >> aria-ref=#1'))).toEqual({
    kind: 'none',
  })
  expect(
    probe.call(resolveRequest('section >> internal:has="aria-ref=#1"')),
  ).toMatchObject({
    kind: 'element',
    preview: expect.stringContaining('<section id="checkout">'),
  })
  expect(probe.call(resolveRequest('aria-ref=e404'))).toEqual({
    kind: 'none',
    staleRef: 'e404',
  })
  probe.document.getElementById('pay')?.remove()
  expect(probe.call(resolveRequest('aria-ref=#1'))).toEqual({
    kind: 'none',
    staleRef: '#1',
  })
})

it('tests that a pin reads as detached once evicted or removed, and comes back by handle', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const first = pinOf(probe.call(resolveRequest('#pw')))
  expect(probe.call({ op: 'pinned', pin: first })).toBe(
    probe.document.getElementById('pw'),
  )
  for (let i = 0; i < 512; i++) probe.call(resolveRequest('#outside'))
  expect(
    probe.call({
      op: 'resolve',
      target: { pin: first },
      strictness: 'strict',
      want: 'element',
    }),
  ).toEqual({ kind: 'detached' })
  expect(probe.call({ op: 'pinned', pin: first })).toBeNull()

  const live = pinOf(probe.call(resolveRequest('#outside')))
  probe.document.getElementById('outside')?.remove()
  expect(
    probe.call({
      op: 'resolve',
      target: { pin: live },
      strictness: 'strict',
      want: 'element',
    }),
  ).toEqual({ kind: 'detached' })
})

it('tests that a batch of 600 pins is one entry, which outlives 511 more and keeps a removed match', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const list = probe.document.createElement('ul')
  for (let i = 0; i < 600; i++) list.append(probe.document.createElement('li'))
  probe.document.body.append(list)
  const batch = probe.call({
    op: 'readAll',
    target: { chunk: parseSelector('li') },
    read: 'pins',
  }) as { count: number; values: { pin: number }[] }
  expect(batch.count).toBe(600)
  const pins = batch.values.map((match) => match.pin)
  expect(new Set(pins).size).toBe(600)
  for (let i = 0; i < 511; i++) probe.call(resolveRequest('#outside'))
  const items = [...probe.document.querySelectorAll('li')]
  expect(pins.map((pin) => probe.call({ op: 'pinned', pin }))).toEqual(items)
  items[0]?.remove()
  expect(probe.call({ op: 'pinned', pin: pins[0] ?? 0 })).toBe(items[0])
  probe.call(resolveRequest('#outside'))
  expect(probe.call({ op: 'pinned', pin: pins[599] ?? 0 })).toBeNull()
})

it("tests that ref= gets Neo's hint and another unknown engine gets upstream's message", () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  expect(probe.call(resolveRequest('ref=e5'))).toEqual({
    kind: 'error',
    class: 'selector',
    message:
      "Unknown selector engine \"ref\". Use page.getByRef('e5') or 'aria-ref=e5'.",
  })
  expect(
    probe.call(resolveRequest('button >> internal:has="ref=e5"')),
  ).toMatchObject({
    class: 'selector',
    message: expect.stringContaining("page.getByRef('e5')"),
  })
  expect(probe.call(resolveRequest('bogus=1'))).toEqual({
    kind: 'error',
    class: 'selector',
    message: 'Unknown engine "bogus" while parsing selector bogus=1',
  })
})

it('tests that an unknown request is internal', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  expect(probe.call({ op: 'teleport' })).toEqual({
    kind: 'error',
    class: 'internal',
    message: 'not a probe call',
  })
  expect(probe.call('describe')).toMatchObject({
    kind: 'error',
    class: 'internal',
  })
})

it('rejects missing or malformed common policy before reading a private snapshot', () => {
  const page = open()
  page.boot({ frameSeq: 0, epoch: 'a' })
  const request = {
    op: 'snapshot',
    root: 'document',
    mode: 'ai',
    conformance: true,
  }
  for (const call of [
    request,
    { request },
    { request, policy: { conformance: 'true', mayHaveOtherFrame: false } },
  ]) {
    expect(globalThis.__neoProbe?.call(call)).toMatchObject({
      kind: 'error',
      class: 'internal',
    })
  }
  expect(
    globalThis.__neoProbe?.call({
      request,
      policy: { conformance: false, mayHaveOtherFrame: false },
    }),
  ).not.toEqual(
    expect.objectContaining({ class: 'internal', message: 'not a probe call' }),
  )
})

it("tests that read resolves and reads one element's textContent, strictly or first", () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const textContent = (
    selector: string,
    strictness: 'strict' | 'firstMatch' = 'strict',
  ) =>
    probe.call({
      op: 'read',
      target: { chunk: parseSelector(selector) },
      strictness,
      frameVisible: true,
      read: { kind: 'textContent' },
    })
  expect(textContent('button:nth-of-type(2)')).toEqual({
    kind: 'read',
    value: 'Stop',
  })
  expect(textContent('button', 'firstMatch')).toEqual({
    kind: 'read',
    value: 'Go',
  })
  expect(textContent('button')).toMatchObject({ kind: 'strict' })
  expect(textContent('#missing')).toEqual({ kind: 'none' })
  expect(textContent('aria-ref=e404')).toEqual({
    kind: 'none',
    staleRef: 'e404',
  })
})

it('tests that readAll counts every match without failing on a stale ref', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const count = (selector: string) =>
    probe.call({
      op: 'readAll',
      target: { chunk: parseSelector(selector) },
      read: 'count',
    })
  expect(count('button')).toEqual({
    kind: 'values',
    count: 2,
    values: [],
  })
  expect(count('#missing')).toEqual({
    kind: 'values',
    count: 0,
    values: [],
  })
  expect(count('aria-ref=e404')).toEqual({
    kind: 'values',
    count: 0,
    values: [],
  })
})

it('tests that a root element scopes the query, as an element handle call does', () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const checkout = probe.document.getElementById('checkout') ?? undefined
  expect(probe.call(resolveRequest('iframe'), checkout)).toMatchObject({
    kind: 'element',
  })
  expect(probe.call(resolveRequest('button'), checkout)).toEqual({
    kind: 'none',
  })
})

it('tests that document reads the title, or the doctype and the markup', () => {
  const markup =
    '<html><head><title>Pay</title></head><body><p>hi</p></body></html>'
  page = openProbePage(`<!doctype html>${markup}`)
  page.boot({ frameSeq: 0, epoch: 'a' })
  expect(page.call({ op: 'document', part: 'title' })).toEqual({
    kind: 'documentText',
    text: 'Pay',
  })
  expect(page.call({ op: 'document', part: 'content' })).toEqual({
    kind: 'documentText',
    text: `<!DOCTYPE html>${markup}`,
  })
})

it("tests that setContent keeps the world booted under its seq, as upstream's frame keeps its seq", () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const steps: string[] = []
  // linkedom has no document.open, write or close; these record what the probe asks of them.
  Object.assign(probe.document, {
    open: () => {
      throw new Error('InvalidStateError')
    },
  })
  expect(probe.call({ op: 'setContent', html: '<p>never</p>' })).toMatchObject({
    kind: 'error',
  })
  expect(probe.call({ op: 'describe' })).toMatchObject({
    kind: 'described',
    seq: 0,
  })
  Object.assign(probe.document, {
    open: () => steps.push('open'),
    write: (html: string) => steps.push(html),
    close: () => steps.push('close'),
  })
  expect(probe.call({ op: 'setContent', html: '<p>new</p>' })).toEqual({
    kind: 'replaced',
  })
  expect(steps).toEqual(['open', '<p>new</p>', 'close'])
  expect(probe.call({ op: 'describe' })).toMatchObject({
    kind: 'described',
    seq: 0,
  })
  const again = probe.boot({ frameSeq: 7, epoch: 'a' })
  expect(again).toMatchObject({ kind: 'described', seq: 0 })
  expect(again).not.toHaveProperty('retired')
  expect(
    probe.boot({ frameSeq: 512340, epoch: 'a restarted claw-sdk' }),
  ).toMatchObject({ seq: 512340, retired: 0 })
})

it("tests that matchUrl runs upstream's glob and the script's RegExp against the URL Rust names", () => {
  const probe = open()
  probe.boot({ frameSeq: 0, epoch: 'a' })
  const match = (pattern: unknown, url: string) =>
    probe.call({ op: 'matchUrl', pattern, url })
  expect(match({ glob: '**/done' }, 'https://a.test/x/done')).toEqual({
    kind: 'urlMatch',
    matches: true,
    url: 'https://a.test/x/done',
  })
  expect(match({ glob: '**/done' }, 'https://a.test/done/no')).toMatchObject({
    matches: false,
  })
  expect(
    match({ glob: 'https://a.test/exact' }, 'https://a.test/exact'),
  ).toMatchObject({ matches: true })
  expect(
    match(
      { regexSource: 'step-(\\d)$', regexFlags: 'i' },
      'https://a.test/STEP-2',
    ),
  ).toMatchObject({ matches: true })
  expect(
    match({ regexSource: '(?<=a)b', regexFlags: '' }, 'https://a.test/ab'),
  ).toMatchObject({ matches: true })
  expect(match({ regexSource: '(', regexFlags: '' }, 'x')).toMatchObject({
    kind: 'error',
    class: 'script',
  })
})
