// Closed shadow roots on the built asset: a snapshot handed seeds (the first `closedRoots` of its
// nodes) opens their roots to the world's engine for the rest of the document's life. linkedom
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

const SNAPSHOT = { op: 'snapshot', root: 'document', mode: 'ai' }
/** A locator's snapshot of the button, which CSS reaches through every root the engine enters. */
const atButton = {
  op: 'snapshot',
  root: { chunk: parseSelector('button') },
  mode: 'ai',
}

/** Every string of a snapshot reply's tree. */
function texts(reply: unknown): string {
  return JSON.stringify(Reflect.get(Object(reply), 'json'))
}

it('tests that a seed opens its closed root to every later read of the world, and nothing else does', () => {
  const { probe, closed, open } = booted()
  const button = closed.firstChild
  expect(texts(probe.call(SNAPSHOT))).not.toContain('Closed button')
  expect(probe.call(atButton)).toEqual({ kind: 'none' })
  // A count of zero, or none, hands no seed: the node is a clickable target, not a seed.
  expect(
    texts(probe.call({ ...SNAPSHOT, closedRoots: 0 }, null, button)),
  ).not.toContain('Closed button')
  // Seeds that name no closed root open nothing: the document, an open root's node, no node.
  const outside = probe.document.querySelector('p')
  expect(
    texts(
      probe.call(
        { ...SNAPSHOT, closedRoots: 3 },
        null,
        outside,
        open.firstChild,
        { not: 'a node' },
      ),
    ),
  ).not.toContain('Closed button')

  const fed = probe.call({ ...SNAPSHOT, closedRoots: 1 }, null, button)
  expect(texts(fed)).toContain('Closed button')
  expect(texts(fed)).toContain('Open text')
  expect(texts(probe.call(SNAPSHOT))).toContain('Closed button')
  // A locator reaches inside it too, once the root was handed in.
  expect(probe.call(atButton)).toMatchObject({ kind: 'snapshot' })
})

it("tests that a closed root of BrowserOS's own UI stays hidden when it is handed in", () => {
  const { probe, bar } = booted()
  const fed = probe.call({ ...SNAPSHOT, closedRoots: 1 }, null, bar.firstChild)
  expect(texts(fed)).not.toContain('Bar button')
  expect(texts(probe.call(SNAPSHOT))).not.toContain('Bar button')
})
