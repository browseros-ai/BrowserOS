// `coverSurvey` on the built asset, the ARIA engine out of it, over a hand-laid page: linkedom
// has no layout, so each test gives its elements boxes, styles and what is on top at a point. The
// answer names each layer, its controls and its holders by pin, for Rust to word from its tree.
// Real hit tests, the top layer, shadow-root hits, the 20 ms budget and closed roots need a
// browser: the snapshot-on-top matrix (prelude_live.rs) covers the survey there.
import { afterEach, expect, it } from 'bun:test'
import type { PinnedLayer } from '../src/probe/protocol'
import { openProbePage, type ProbePage } from './support/probe-page'

type Box = { x: number; y: number; width: number; height: number }
type Fixture = {
  boxes: Record<string, Box>
  styles?: Record<string, Record<string, string>>
  /** What is on top at a point, topmost first, by id. */
  hits(at: (id: string) => boolean): string[]
}

const VIEW = { innerWidth: 1280, innerHeight: 800 }
const GLOBALS = [...Object.keys(VIEW), 'NodeList', 'MouseEvent']

let page: ProbePage | undefined
let saved: [string, PropertyDescriptor | undefined][] = []
afterEach(() => {
  page?.close()
  page = undefined
  for (const [name, descriptor] of saved)
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  saved = []
})

function laidOut(html: string, fixture: Fixture): ProbePage {
  const probe = openProbePage(html)
  page = probe
  probe.boot({ frameSeq: 0, epoch: 'cover-survey' })
  const { document } = probe
  const view = document.defaultView
  if (!view) throw new Error('the fixture has a window')
  const byId = (id: string): Element => {
    const found =
      document.getElementById(id) ??
      [...document.querySelectorAll('*')]
        .map((host) => host.shadowRoot?.getElementById(id))
        .find((inner) => inner)
    if (!found) throw new Error(`no #${id}`)
    return found
  }
  const boxes = new Map(
    Object.entries(fixture.boxes).map(([id, box]) => [byId(id), box]),
  )
  const styles = new Map(
    Object.entries(fixture.styles ?? {}).map(([id, style]) => [
      byId(id),
      style,
    ]),
  )
  const style = (element: Element) => ({
    display: 'block',
    visibility: 'visible',
    opacity: '1',
    position: 'static',
    pointerEvents: 'auto',
    overflowX: 'visible',
    overflowY: 'visible',
    cursor: 'auto',
    content: 'none',
    transform: 'none',
    borderLeftWidth: '0px',
    borderTopWidth: '0px',
    ...styles.get(element),
    getPropertyValue: () => '',
  })
  const rect = (element: Element) => {
    const { x, y, width, height } = boxes.get(element) ?? {
      x: 0,
      y: 0,
      width: 100,
      height: 40,
    }
    return {
      x,
      y,
      width,
      height,
      left: x,
      top: y,
      right: x + width,
      bottom: y + height,
    }
  }
  Reflect.set(view, 'getComputedStyle', style)
  Reflect.set(globalThis, 'getComputedStyle', style)
  const prototype = view.Element.prototype
  // The harness gives form controls a box of their own; this page's boxes replace it too.
  for (const each of [
    prototype,
    view.HTMLInputElement.prototype,
    view.HTMLTextAreaElement.prototype,
  ]) {
    Reflect.set(each, 'getBoundingClientRect', function (this: Element) {
      return rect(this)
    })
    Reflect.set(each, 'getClientRects', function (this: Element) {
      const box = rect(this)
      return box.width > 0 && box.height > 0 ? [box] : []
    })
    Reflect.set(each, 'checkVisibility', () => true)
  }
  const matches = prototype.matches
  Reflect.set(prototype, 'matches', function (this: Element, selector: string) {
    return selector === ':modal' || selector === ':popover-open'
      ? false
      : Reflect.apply(matches, this, [selector])
  })
  Object.defineProperty(view.HTMLElement.prototype, 'innerText', {
    configurable: true,
    get(this: HTMLElement) {
      return this.textContent ?? ''
    },
  })
  const hits = (x: number, y: number) =>
    fixture
      .hits((id) => {
        const box = fixture.boxes[id]
        return (
          box !== undefined &&
          x >= box.x &&
          x < box.x + box.width &&
          y >= box.y &&
          y < box.y + box.height
        )
      })
      .map(byId)
  Reflect.set(document, 'elementsFromPoint', hits)
  Reflect.set(
    document,
    'elementFromPoint',
    (x: number, y: number) => hits(x, y)[0] ?? null,
  )
  saved = GLOBALS.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ])
  for (const [name, value] of Object.entries(VIEW))
    Reflect.set(globalThis, name, value)
  for (const name of ['NodeList', 'MouseEvent'])
    Reflect.set(globalThis, name, Reflect.get(view, name))
  return probe
}

const field = (value: unknown, name: string): unknown =>
  typeof value === 'object' && value !== null
    ? Reflect.get(value, name)
    : undefined

const pinned = (probe: ProbePage, pin: number): unknown =>
  probe.call({ op: 'pinned', pin })

function survey(probe: ProbePage) {
  const reply = probe.call({ op: 'coverSurvey' })
  expect(field(reply, 'kind')).toBe('surveyed')
  return reply as {
    layers: PinnedLayer[]
    tested: number
    surveyMs: number
    phases: Record<string, unknown>
  }
}

/** The survey's layers as elements and facts, each pin read back through the probe. */
function asElements(probe: ProbePage) {
  const pinnedSurvey = survey(probe)
  const pinnedLayers = pinnedSurvey.layers.map((layer) => ({
    named: pinned(probe, layer.pin),
    controls: layer.controls.map((pin) => pinned(probe, pin)),
    facts: [
      layer.blank,
      layer.loading,
      layer.wholeView,
      layer.firstWords,
    ] as unknown[],
  }))
  return { pinnedSurvey, pinnedLayers }
}

const OVERLAY = `<html><body>
  <main id="main"><button id="behind">Pay</button><a id="more" href="/x">More</a></main>
  <div id="overlay" role="dialog" aria-label="Sign in">
    <p>Please sign in</p>
    <button id="close">Close</button><button id="later">Later</button>
  </div>
</body></html>`

const OVERLAY_LAYOUT: Fixture = {
  boxes: {
    behind: { x: 100, y: 100, width: 100, height: 40 },
    more: { x: 300, y: 100, width: 100, height: 40 },
    overlay: { x: 0, y: 0, width: 1280, height: 800 },
    close: { x: 200, y: 200, width: 80, height: 40 },
    later: { x: 300, y: 200, width: 80, height: 40 },
  },
  styles: { overlay: { position: 'fixed' } },
  hits: (at) =>
    at('close')
      ? ['close', 'overlay']
      : at('later')
        ? ['later', 'overlay']
        : ['overlay'],
}

it('tests that a whole-view layer comes back pinned, with no snapshot taken before it', () => {
  const probe = laidOut(OVERLAY, OVERLAY_LAYOUT)
  const { layers, tested, surveyMs, phases } = survey(probe)
  const element = (id: string) => probe.document.getElementById(id)
  expect(layers).toHaveLength(1)
  const [layer] = layers
  if (!layer) throw new Error('one layer')
  expect(pinned(probe, layer.pin)).toBe(element('overlay'))
  expect(layer.controls.map((pin) => pinned(probe, pin))).toEqual([
    element('close'),
    element('later'),
  ])
  expect(layer.holders.map((pin) => pinned(probe, pin))).toEqual([
    element('overlay'),
  ])
  expect(layer).toMatchObject({
    blank: false,
    loading: false,
    wholeView: true,
    firstWords: 'Please sign in CloseLater',
  })
  // The named element is also its holder: one pin per element, every pin in one entry.
  expect(layer.holders).toEqual([layer.pin])
  expect(new Set([layer.pin, ...layer.controls]).size).toBe(3)
  expect(tested).toBe(4)
  expect(surveyMs).toBe(Number(phases.hits))
  expect(phases.stopped).toBe(false)
})

const BACKDROP = `<html><body>
  <main id="main"><button id="behind">Pay</button><a id="foot" href="/f">Footer</a></main>
  <div id="backdrop"></div>
  <div id="dialog" role="dialog"><button id="close">Close</button></div>
  <div id="banner"><span>We use cookies</span><input id="agree" type="checkbox"></div>
  <iframe id="ad" src="https://ads.example/"></iframe>
  <button id="ghost">Ghost</button><button id="tiny">Tiny</button>
</body></html>`

it('tests that a blank layer and a part-view layer pin by element, each with its own controls', () => {
  const laid = laidOut(BACKDROP, {
    boxes: {
      behind: { x: 100, y: 100, width: 100, height: 40 },
      foot: { x: 100, y: 600, width: 100, height: 40 },
      backdrop: { x: 0, y: 0, width: 1280, height: 800 },
      dialog: { x: 400, y: 150, width: 400, height: 200 },
      close: { x: 420, y: 200, width: 80, height: 40 },
      banner: { x: 0, y: 500, width: 1280, height: 300 },
      agree: { x: 120, y: 520, width: 20, height: 20 },
      ad: { x: 900, y: 100, width: 100, height: 100 },
      ghost: { x: 100, y: 300, width: 100, height: 40 },
      tiny: { x: 100, y: 400, width: 0, height: 0 },
    },
    styles: {
      backdrop: { position: 'fixed' },
      dialog: { position: 'fixed' },
      banner: { position: 'fixed' },
      ghost: { pointerEvents: 'none' },
      ad: { pointerEvents: 'none' },
    },
    hits: (at) =>
      at('close')
        ? ['close', 'dialog']
        : at('dialog')
          ? ['dialog']
          : at('agree')
            ? ['agree', 'banner']
            : at('banner')
              ? ['banner']
              : ['backdrop'],
  })
  const { pinnedLayers, pinnedSurvey } = asElements(laid)
  const element = (id: string) => laid.document.getElementById(id)
  // The blank backdrop stands under no dialog at the dialog's own centre, so it is named as
  // itself; the banner is named as itself and lists its checkbox.
  expect(pinnedLayers.map(({ named }) => named)).toEqual([
    element('backdrop'),
    element('banner'),
  ])
  expect(pinnedLayers.map(({ controls }) => controls)).toEqual([
    [],
    [element('agree')],
  ])
  expect(pinnedLayers.map(({ facts }) => facts)).toEqual([
    [true, false, false, undefined],
    [false, false, false, 'We use cookies'],
  ])
  // Neither layer covers the whole view, so neither has holders to mark under.
  expect(pinnedSurvey.layers.map(({ holders }) => holders)).toEqual([[], []])
  // Tested: Pay, Footer, Close, the checkbox and the frame (a frame takes the pointer whatever
  // its own style says); not the pointer-free Ghost nor the boxless Tiny.
  expect(pinnedSurvey.tested).toBe(5)
})

it('tests that a control in an open shadow root is in the population, in composed-tree order', () => {
  const html = `<html><body>
    <div id="host"></div>
    <div id="overlay"><p>Wait</p></div>
  </body></html>`
  const laid = laidOut(html, {
    boxes: {
      host: { x: 0, y: 0, width: 400, height: 100 },
      overlay: { x: 0, y: 0, width: 1280, height: 800 },
    },
    styles: { overlay: { position: 'fixed' } },
    hits: () => ['overlay'],
  })
  const shadowHost = laid.document.getElementById('host')
  const root = shadowHost?.attachShadow({ mode: 'open' })
  root?.append(
    Object.assign(laid.document.createElement('button'), {
      textContent: 'Inside',
    }),
  )
  const { tested, layers } = survey(laid)
  expect(tested).toBe(1)
  expect(layers.map((layer) => pinned(laid, layer.pin))).toEqual([
    laid.document.getElementById('overlay'),
  ])
})

it('tests that a page nothing covers surveys its controls and answers no layer', () => {
  const probe = laidOut(
    '<html><body><button id="a">A</button><a id="b" href="/b">B</a></body></html>',
    {
      boxes: {
        a: { x: 100, y: 100, width: 100, height: 40 },
        b: { x: 300, y: 100, width: 100, height: 40 },
      },
      hits: (at) => (at('a') ? ['a'] : at('b') ? ['b'] : []),
    },
  )
  const { layers, tested } = survey(probe)
  expect(layers).toEqual([])
  expect(tested).toBe(2)
})

it("tests that finding the population spends the survey's 20 ms budget and stops it", () => {
  // Two thousand buttons that take no pointer: each is a control the walk must read the style
  // of before it can drop it, and every style read here costs 2 ms of the survey's clock.
  const ids = Array.from({ length: 2000 }, (_, index) => `b${index}`)
  const probe = laidOut(
    `<html><body>${ids.map((id) => `<button id="${id}">${id}</button>`).join('')}</body></html>`,
    {
      boxes: {},
      styles: Object.fromEntries(
        ids.map((id) => [id, { pointerEvents: 'none' }]),
      ),
      hits: () => [],
    },
  )
  let clock = 0
  let reads = 0
  const styleOf = Reflect.get(globalThis, 'getComputedStyle') as (
    element: Element,
  ) => unknown
  const timedStyle = (element: Element) => {
    reads++
    clock += 2
    return styleOf(element)
  }
  Reflect.set(globalThis, 'getComputedStyle', timedStyle)
  Reflect.set(
    probe.document.defaultView ?? globalThis,
    'getComputedStyle',
    timedStyle,
  )
  Reflect.set(performance, 'now', () => clock)
  try {
    const { layers, tested, phases } = survey(probe)
    expect(phases.stopped).toBe(true)
    expect(tested).toBe(0)
    expect(layers).toEqual([])
    // A deadline read before each control's style reads stops the walk within the budget, not
    // after 2,000 controls' worth of reads.
    expect(reads).toBeLessThan(30)
  } finally {
    Reflect.deleteProperty(performance, 'now')
  }
})
