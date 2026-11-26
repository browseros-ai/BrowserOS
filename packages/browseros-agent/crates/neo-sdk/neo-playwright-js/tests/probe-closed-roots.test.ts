// Closed shadow roots on the built asset: `openClosedRoots` handed seeds (the nodes after the
// call's root, from an APC capture's contract field 31) opens their roots to the world's engines
// for the rest of the document's life. linkedom
// shares one set of DOM prototypes with the test, so the page world's own `null` is covered on a
// real browser (tests/prelude_live.rs), with the hit test, frames and nesting.
import { afterEach, expect, it } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { openProbePage, type ProbePage } from './support/probe-page'

let page: ProbePage | undefined
afterEach(() => page?.close())

const PAGE = `<html><body><main>
  <p>Outside</p>
  <div id="host"></div>
  <div id="open"></div>
  <browseros-neo-takeover id="bar"></browseros-neo-takeover>
</main></body></html>`

/** The page, booted, with a closed root, an open root and a closed BrowserOS UI root. */
function booted() {
  page = openProbePage(PAGE)
  const probe = page
  const shadow = (id: string, mode: 'open' | 'closed', html: string) => {
    const host = probe.document.getElementById(id)
    if (!host) throw new Error(`no #${id}`)
    const root = host.attachShadow({ mode })
    root.innerHTML = html
    return root
  }
  const closed = shadow('host', 'closed', '<button>Closed button</button>')
  const open = shadow('open', 'open', '<p>Open text</p>')
  const bar = shadow('bar', 'closed', '<button>Bar button</button>')
  probe.boot({ frameSeq: 0, epoch: 'a' })
  return { probe, closed, open, bar }
}

/** The first button the page's CSS engine reaches, through every root it may enter. */
const button = {
  op: 'read',
  target: { chunk: parseSelector('button') },
  strictness: 'firstMatch',
  frameVisible: true,
  read: { kind: 'textContent' },
}

/** `openClosedRoots` with `seeds` handed after the call's root, as Rust's call_with_nodes hands them. */
function open(probe: ProbePage, ...seeds: unknown[]): unknown {
  return probe.call({ op: 'openClosedRoots' }, null, ...seeds)
}

it('tests that a seed opens its closed root to every later read of the world, and nothing else does', () => {
  const { probe, closed, open: openRoot } = booted()
  expect(probe.call(button)).toEqual({ kind: 'none' })
  // Seeds that name no closed root open nothing: the document, an open root's node, no node.
  const outside = probe.document.querySelector('p')
  expect(open(probe, outside, openRoot.firstChild, { not: 'a node' })).toEqual({
    kind: 'value',
    value: null,
  })
  expect(probe.call(button)).toEqual({ kind: 'none' })
  // A seed inside the closed root opens it for every later read of this world.
  expect(open(probe, closed.firstChild)).toEqual({ kind: 'value', value: null })
  expect(probe.call(button)).toMatchObject({
    kind: 'read',
    value: 'Closed button',
  })
  expect(probe.call(button)).toMatchObject({
    kind: 'read',
    value: 'Closed button',
  })
})

it("tests that a closed root of BrowserOS's own UI stays hidden when it is handed in", () => {
  const { probe, bar } = booted()
  open(probe, bar.firstChild)
  expect(probe.call(button)).toEqual({ kind: 'none' })
})
