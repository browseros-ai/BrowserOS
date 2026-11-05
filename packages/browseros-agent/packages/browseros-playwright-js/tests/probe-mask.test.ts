// Neo's mask on page text before Playwright's engine cuts or quotes it (src/probe/page-mask.ts):
// the forms it matches, the gate that answers `needsMask`, and the previews and locator hints
// of the built asset on a linkedom page, where a secret straddles each of the engine's cuts.
// site_reads_live.rs runs the same cases on a real page, with a role-name hint linkedom cannot
// compute.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { maskPageText, underMask } from '../src/probe/page-mask'
import { openProbePage, type ProbePage } from './support/probe-page'

const SECRET = 'inproc-secret-41'

/** Each element's copy of SECRET meets one of the engine's cuts. */
const PAGE = `<html><body>
  <div class="who">Signed in with ${SECRET}</div>
  <div class="who">Other</div>
  <img class="who" alt="A photo taken of the ${SECRET} at home">
  <div id="long">The name this account was opened under is ${SECRET} tail</div>
  <ul><li>One</li><li>Two</li></ul>
  <iframe id="pay" src="https://pay.example/frame"></iframe>
</body></html>`

let page: ProbePage | undefined
afterEach(() => page?.close())

function booted(): ProbePage {
  page = openProbePage(PAGE)
  page.boot({ frameSeq: 0, epoch: 'mask-test' })
  return page
}

const target = (selector: string) => ({ chunk: parseSelector(selector) })
const resolveOne = (selector: string, want = 'element') => ({
  op: 'resolve',
  target: target(selector),
  strictness: 'strict',
  want,
})
const readOf = (selector: string, kind: string) => ({
  op: 'read',
  target: target(selector),
  strictness: 'firstMatch',
  frameVisible: true,
  read: { kind },
})

/** `text` masked under `mask`, and whether the call would need a mask it does not have. */
const masked = (mask: string[] | undefined, text: string) =>
  underMask(mask, () => maskPageText(text))

/** The longest start of `secret`, three characters or more, that `shown` holds. */
const leakedStart = (shown: unknown, secret = SECRET): string | undefined => {
  const text = JSON.stringify(shown)
  for (let end = secret.length; end >= 3; end--)
    if (text.includes(secret.slice(0, end))) return secret.slice(0, end)
  return undefined
}

it('tests that every form core masks is masked, leftmost and longest first', () => {
  const plain =
    'We sent 482913. Typed "pa ss\\"w0rd" or ?p=pa%20ss%22w0rd&q=pa+ss%22w0rd'
  expect(masked(['482913', 'pa ss"w0rd'], plain).value).toBe(
    'We sent [private]. Typed "[private]" or ?p=[private]&q=[private]',
  )
  const markup =
    '<p data-echo="typed&amp;&quot;secret&lt;3&gt;&nbsp;x">you typed typed&amp;"secret&lt;3&gt;&nbsp;x</p>'
  expect(masked(['typed&"secret<3> x'], markup).value).toBe(
    '<p data-echo="[private]">you typed [private]</p>',
  )
  expect(masked(['abcd', 'abcdefgh'], 'xabcdefghy abcd').value).toBe(
    'x[private]y [private]',
  )
})

it("tests that the engine's whitespace-normalized copy of a value is masked", () => {
  expect(
    masked(['line one\nline  two'], 'Signed in as line one line two').value,
  ).toBe('Signed in as [private]')
  // The value itself is six characters, its normalized form only two, which stays unmasked.
  expect(masked(['  ab  '], 'ab and  ab  ').value).toBe('ab and[private]')
})

it('tests that text reaching a cut without a mask marks the call, and outside a call is masked whole', () => {
  expect(masked(undefined, `Signed in with ${SECRET}`)).toEqual({
    value: `Signed in with ${SECRET}`,
    needsMask: true,
  })
  expect(masked([], `Signed in with ${SECRET}`)).toEqual({
    value: `Signed in with ${SECRET}`,
    needsMask: false,
  })
  expect(maskPageText(`Signed in with ${SECRET}`)).toBe('[private]')
  expect(maskPageText('')).toBe('')
})

it('tests that every reply that previews page text answers needsMask without a mask', () => {
  const probe = booted()
  const previewing = [
    resolveOne('#long'),
    resolveOne('.who'),
    resolveOne('#long', 'frame'),
    readOf('#long', 'located'),
    { op: 'readAll', target: target('li'), read: 'pins' },
  ]
  for (const request of previewing) {
    expect(probe.call(request, undefined, null)).toEqual({ kind: 'needsMask' })
    expect(probe.call(request, undefined, [SECRET])).not.toEqual({
      kind: 'needsMask',
    })
  }
})

it('tests that a reply the engine cut no page text for needs no mask', () => {
  const probe = booted()
  expect(
    probe.call(resolveOne('#pay', 'frame'), undefined, null),
  ).toMatchObject({
    kind: 'frame',
    src: 'https://pay.example/frame',
  })
  expect(
    probe.call(readOf('#long', 'textContent'), undefined, null),
  ).toMatchObject({
    kind: 'read',
    value: `The name this account was opened under is ${SECRET} tail`,
  })
  expect(
    probe.call(
      { op: 'readAll', target: target('li'), read: 'count' },
      undefined,
      null,
    ),
  ).toMatchObject({ kind: 'values', count: 2 })
})

it('tests that a preview is masked before its 50-character cut', () => {
  const probe = booted()
  expect(probe.call(resolveOne('#long'), undefined, [SECRET])).toMatchObject({
    kind: 'element',
    preview:
      '<div id="long">The name this account was opened under is [privat…</div>',
  })
  expect(probe.call(resolveOne('#long', 'frame'), undefined, [SECRET])).toEqual(
    {
      kind: 'notFrame',
      preview:
        '<div id="long">The name this account was opened under is [privat…</div>',
    },
  )
})

it("tests that a strict violation's hints are built from masked text", () => {
  const probe = booted()
  const strict = probe.call(resolveOne('.who'), undefined, [SECRET])
  expect(strict).toMatchObject({ kind: 'strict' })
  expect(leakedStart(strict)).toBeUndefined()
  const message = String(Reflect.get(Object(strict), 'message'))
  // Upstream's hints were getByText('Signed in with inproc-secret-'), the text without its
  // trailing number, and getByAltText('A photo taken of the inproc-'), the alt text cut at a word.
  expect(message).toContain(
    '1) <div class="who">Signed in with [private]</div> aka ',
  )
  expect(message).toContain(
    '3) <img class="who" alt="A photo taken of the [private] at home"/> aka ',
  )
  expect(message.match(/ aka /g)).toHaveLength(3)
})

it("tests that a field changed since Rust's sweep is masked before every cut", () => {
  // Rust swept the site a round trip before the call; the field has been typed into since.
  const fresh = 'fresh-secret-123456'
  page = openProbePage(
    `<html><body>
      <input type="password">
      <div class="who">Signed in with ${fresh}</div>
      <div class="who">Other</div>
      <div id="long">The name this account was opened under is ${fresh} tail</div>
    </body></html>`,
    { sensitiveValues: () => [fresh] },
  )
  page.boot({ frameSeq: 0, epoch: 'mask-test' })
  const previewing = [
    resolveOne('#long'),
    resolveOne('#long', 'frame'),
    resolveOne('.who'),
    readOf('#long', 'located'),
    { op: 'readAll', target: target('.who'), read: 'pins' },
  ]
  for (const stale of [[], ['stale-secret-0000']])
    for (const request of previewing) {
      const reply = page.call(request, undefined, stale)
      expect(reply).not.toEqual({ kind: 'needsMask' })
      // `sweep` is the frame's raw values for Rust's own mask, never shown.
      const { sweep: _, ...shown } = Object(reply)
      expect(leakedStart(shown, fresh)).toBeUndefined()
    }
})

it('tests that a mask that is not a list of strings is refused', () => {
  const probe = booted()
  for (const mask of ['hunter2', [1], { values: [] }])
    expect(
      probe.call(resolveOne('#long'), undefined, mask as unknown as string[]),
    ).toEqual({ kind: 'error', class: 'internal', message: 'not a probe mask' })
})
