// The probe's `snapshot` request on the built asset: how a locator root that is not one live
// element is answered, and the sweep every frame-level snapshot carries. linkedom has no
// layout, so ai-mode refs and the rendered iframes are covered by the crate's
// tests/snapshot_live.rs on a real browser.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser'
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

it('tests that a frame-level snapshot carries the sweep and no iframes outside ai mode', () => {
  const probe = booted()
  expect(
    probe.call({ op: 'snapshot', root: 'document', mode: 'default' }),
  ).toMatchObject({ kind: 'snapshot', iframes: [], sweep: ['hunter2'] })
})
