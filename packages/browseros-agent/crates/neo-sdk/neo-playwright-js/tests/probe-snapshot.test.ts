// The probe's `snapshot` request on the built asset: how a locator root that is not one live
// element is answered, and what a frame-level snapshot carries. linkedom has no
// layout, so ai-mode refs and the rendered iframes are covered by the crate's
// tests/snapshot_live.rs on a real browser.
import { afterEach, expect, it, spyOn } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

const PAGE = `<html><body>
  <h1>Pay</h1><p>Confirm your code</p>
  <input id="pw" type="password" value="hunter2">
  <iframe id="pay" src="https://pay.example/frame"></iframe>
</body></html>`

let page: ProbePage | undefined
afterEach(() => page?.close())

const booted = () => {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 3, epoch: 'a' })
  return page
}

const atLocator = (selector: string) => ({
  op: 'snapshot',
  root: { chunk: parseSelector(selector) },
  mode: 'ai',
})

/** A deterministic visible layout lets the real engine serialize this fixture in linkedom. */
function showFixture(probe: ProbePage) {
  const view = probe.document.defaultView
  if (!view) throw new Error('the fixture has a window')
  Reflect.set(view, 'getComputedStyle', () => ({
    display: 'block',
    visibility: 'visible',
    cursor: 'auto',
    content: 'none',
    getPropertyValue: () => '',
  }))
  for (const element of probe.document.querySelectorAll('*')) {
    Reflect.set(element, 'checkVisibility', () => true)
    Reflect.set(element, 'getBoundingClientRect', () => ({
      x: 0,
      y: 0,
      width: 10,
      height: 10,
    }))
  }
}

it('tests that a locator root matching no element, several, or a stale ref is answered before any snapshot', () => {
  const probe = booted()
  expect(probe.call(atLocator('#missing'))).toEqual({ kind: 'none' })
  expect(probe.call(atLocator('aria-ref=f3e9'))).toEqual({
    kind: 'none',
    staleRef: 'f3e9',
  })
  const strict = probe.call(atLocator('h1, p'))
  expect(strict).toMatchObject({ kind: 'strict' })
  expect(String(Reflect.get(Object(strict), 'message'))).toStartWith(
    "strict mode violation: locator('h1, p') resolved to 2 elements:",
  )
})

it('tests that a frame-level snapshot carries no iframes outside ai mode', () => {
  const probe = booted()
  expect(
    probe.call({ op: 'snapshot', root: 'document', mode: 'default' }),
  ).toMatchObject({ kind: 'snapshot', iframes: [] })
})

it('tests that a snapshot whose clickable marking throws is still taken, as upstream takes it', () => {
  const probe = booted()
  const target = probe.document.querySelector('h1')
  const inside = probe.document.createElement('span')
  target?.append(inside)
  showFixture(probe)
  // Fail marking's guard once; the engine's unmarked retry must still serialize bullets.
  const read = inside.getAttribute.bind(inside)
  const password = probe.document.querySelector('input')
  if (!(password instanceof HTMLInputElement))
    throw new Error('the fixture password exists')
  let valueDuringRetry = ''
  let markFailed = false
  inside.getAttribute = (name: string) => {
    if (name === 'aria-owns' && !markFailed) {
      markFailed = true
      valueDuringRetry = password.value
      throw new Error('unforeseen page shape')
    }
    return read(name)
  }
  const request = { op: 'snapshot', root: 'document', mode: 'ai' }
  const retried = probe.call(request, null, target)
  expect(retried).toMatchObject({
    kind: 'snapshot',
    marksSkipped: true,
  })
  expect(JSON.stringify(retried)).toContain('"text":"•••••••"')
  expect(JSON.stringify(retried)).not.toContain('hunter2')
  expect(valueDuringRetry).toBe('•••••••')
  expect(password.value).toBe('hunter2')
  const plain = probe.call(request)
  expect(plain).toMatchObject({ kind: 'snapshot' })
  expect(plain).not.toHaveProperty('marksSkipped')
  expect(JSON.stringify(plain)).toContain('"text":"•••••••"')
})

it('never installs any field or text override under conformance', () => {
  page = openProbePage(PAGE, { conformance: true, mayHaveOtherFrame: true })
  page.boot({ frameSeq: 3, epoch: 'a' })
  const probe = page
  showFixture(probe)
  const replacements: unknown[] = []
  const define = Object.defineProperty
  const observe = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      if (
        object === HTMLInputElement.prototype ||
        object === HTMLTextAreaElement.prototype ||
        object === Text.prototype
      )
        replacements.push({ object, name })
      return define(object, name, descriptor)
    },
  )
  try {
    const result = probe.call({
      op: 'snapshot',
      root: 'document',
      mode: 'ai',
    })
    expect(result).toMatchObject({ kind: 'snapshot' })
    expect(JSON.stringify(result)).toContain('"text":"hunter2"')
    expect(replacements).toEqual([])
  } finally {
    observe.mockRestore()
  }
})

it('masks textarea child text and control attributes used as another element name', () => {
  const probe = openProbePage(`<html><body>
    <textarea role="searchbox" autocomplete="new-password">CANARY-TEXTAREA</textarea>
    <input id="private-number" type="number" autocomplete="cc-number" value="938217">
    <button aria-labelledby="private-number"></button>
    <textarea id="self" role="button" aria-labelledby="self" autocomplete="new-password">CANARY-SELF</textarea>
    <textarea id="named" autocomplete="new-password">CANARY-NAMED</textarea>
    <button aria-labelledby="named"></button>
    <textarea role="searchbox">PUBLIC-TEXTAREA</textarea>
    <input id="public-number" type="number" value="817236">
    <button aria-labelledby="public-number"></button>
  </body></html>`)
  page = probe
  probe.boot({ frameSeq: 3, epoch: 'a' })
  showFixture(probe)
  for (const mode of ['ai', 'default']) {
    const snapshot = probe.call({ op: 'snapshot', root: 'document', mode })
    expect(snapshot).toMatchObject({ kind: 'snapshot' })
    const encoded = JSON.stringify(snapshot)
    for (const canary of [
      'CANARY-TEXTAREA',
      '938217',
      'CANARY-SELF',
      'CANARY-NAMED',
    ])
      expect(encoded).not.toContain(canary)
    for (const visible of ['PUBLIC-TEXTAREA', '817236', '••••••'])
      expect(encoded).toContain(visible)
  }
})

it('refuses the snapshot rather than falling back when masking cannot be installed', () => {
  const probe = booted()
  const before = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )
  const define = Object.defineProperty
  const refuse = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      if (object === HTMLTextAreaElement.prototype && name === 'value')
        throw new TypeError('mask installation refused')
      return define(object, name, descriptor)
    },
  )
  try {
    const result = probe.call(
      { op: 'snapshot', root: 'document', mode: 'ai' },
      null,
      probe.document.querySelector('h1'),
    )
    expect(result).toMatchObject({ kind: 'error', class: 'internal' })
    expect(result).not.toHaveProperty('json')
    expect(
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value'),
    ).toEqual(before)
  } finally {
    refuse.mockRestore()
  }
})

const MARKED_PAGE = `<html><body>
  <h1>Code</h1>
  <input id="code" type="text" aria-label="Code" value="CANARY-1">
  <input id="pw" type="password" value="hunter2">
</body></html>`

/** The pin of the one element `selector` matches, as Rust's resolve hands it out. */
function pinOf(probe: ProbePage, selector: string): number {
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

it("tests that a box the entry typed into prints bullets in the page's snapshot, with and without the conformance switch (rulings R14 and R44)", async () => {
  for (const conformance of [false, true]) {
    page?.close()
    page = openProbePage(MARKED_PAGE, {
      conformance,
      mayHaveOtherFrame: true,
    })
    page.boot({ frameSeq: 3, epoch: 'a' })
    showFixture(page)
    const probe = page
    const box = pinOf(probe, '#code')
    // The entry marks the box before typing. Policy belongs to the common captured envelope.
    await probe.call({
      op: 'private',
      pin: box,
      step: { kind: 'focus', text: 'CANARY-1', spaced: false },
    })
    probe.call({ op: 'private', pin: box, step: { kind: 'forget' } })
    const printed = JSON.stringify(
      probe.call({
        op: 'snapshot',
        root: 'document',
        mode: 'ai',
      }),
    )
    expect(printed).not.toContain('CANARY-1')
    expect(printed).toContain('"text":"••••••••"')
    // The page's own declaration is masked outside conformance and prints as upstream under it.
    expect(printed).toContain(
      conformance ? '"text":"hunter2"' : '"text":"•••••••"',
    )
  }
})
