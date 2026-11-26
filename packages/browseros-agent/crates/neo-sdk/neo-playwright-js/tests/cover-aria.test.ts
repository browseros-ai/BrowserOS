// The cover's page half and the private look with the ARIA engine out of the bundle (apc-engine-
// design 8.2): every decision reads the live page through the stand-in's pure-DOM role and the
// page's own words, and what an answer names comes back pinned for Rust to name from its tree.
// linkedom has no layout, so each test lays out its elements and what is on top at a point.
import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { parseHTML } from 'linkedom'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { AriaNodeJSON, AriaSnapshotJSON } from '../src/prelude/vendored'
import type * as CoverMod from '../src/probe/cover'
import type * as CoverSurveyMod from '../src/probe/cover-survey'
import type * as FailureOutputMod from '../src/probe/failure-output'
import type * as PrivateLookMod from '../src/probe/private-look'
import type * as SnapshotInertMod from '../src/probe/snapshot-inert'
import type * as WorldMod from '../src/probe/world'
import { computeAriaRole } from '../src/stubs/roleUtils'

const PACKAGE_ROOT = resolve(import.meta.dir, '..')
const VENDOR_ROOT = resolve(
  PACKAGE_ROOT,
  '../neo-playwright/third_party/playwright',
)

type ProbeBundle = {
  cover: typeof CoverMod
  coverSurvey: typeof CoverSurveyMod
  failureOutput: typeof FailureOutputMod
  privateLook: typeof PrivateLookMod
  snapshotInert: typeof SnapshotInertMod
  createEngine: typeof WorldMod.createEngine
  Pins: typeof WorldMod.Pins
  RefMap: new () => WorldMod.World['refs']
}

const BUNDLE_ENTRY_PATH = resolve(
  PACKAGE_ROOT,
  'src/probe/__cover-aria-entry.ts',
)
const BUNDLE_ENTRY_SOURCE = [
  "export * as cover from './cover'",
  "export * as coverSurvey from './cover-survey'",
  "export * as failureOutput from './failure-output'",
  "export * as privateLook from './private-look'",
  "export * as snapshotInert from './snapshot-inert'",
  "export { createEngine, Pins } from './world'",
  "export { RefMap } from './aria-ref'",
].join('\n')

function probeBundleStubs(withAriaStubs: boolean): Record<string, string> {
  const stubs: Record<string, string> = {
    './highlight': resolve(PACKAGE_ROOT, 'src/stubs/highlight.ts'),
    './consoleApi': resolve(PACKAGE_ROOT, 'src/stubs/consoleApi.ts'),
  }
  if (!withAriaStubs) return stubs
  return {
    ...stubs,
    './roleUtils': resolve(PACKAGE_ROOT, 'src/stubs/roleUtils.ts'),
    './ariaSnapshot': resolve(PACKAGE_ROOT, 'src/stubs/ariaSnapshot.ts'),
    './roleSelectorEngine': resolve(
      PACKAGE_ROOT,
      'src/stubs/roleSelectorEngine.ts',
    ),
  }
}

function probeBundlePlugin(stubs: Record<string, string>): Bun.BunPlugin {
  return {
    name: 'cover-aria-test-bundle',
    setup(build) {
      build.onResolve({ filter: /__cover-aria-entry\.ts$/ }, () => ({
        path: BUNDLE_ENTRY_PATH,
      }))
      build.onLoad({ filter: /__cover-aria-entry\.ts$/ }, () => ({
        contents: BUNDLE_ENTRY_SOURCE,
        loader: 'ts',
      }))
      build.onResolve(
        {
          filter:
            /^\.\/(highlight|consoleApi|roleUtils|ariaSnapshot|roleSelectorEngine)$/,
        },
        (args) => (stubs[args.path] ? { path: stubs[args.path] } : undefined),
      )
      build.onResolve({ filter: /^@isomorphic\// }, (args) => ({
        path: resolve(
          VENDOR_ROOT,
          'packages/isomorphic',
          `${args.path.slice('@isomorphic/'.length)}.ts`,
        ),
      }))
    },
  }
}

async function loadProbeBundle(options: {
  withAriaStubs: boolean
}): Promise<ProbeBundle> {
  const result = await Bun.build({
    entrypoints: [BUNDLE_ENTRY_PATH],
    target: 'browser',
    format: 'esm',
    minify: false,
    plugins: [probeBundlePlugin(probeBundleStubs(options.withAriaStubs))],
  })
  if (!result.success || !result.outputs[0])
    throw new AggregateError(
      result.logs,
      `Could not bundle probe modules (withAriaStubs=${options.withAriaStubs})`,
    )
  const source = await result.outputs[0].text()
  const url = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
  return (await import(url)) as ProbeBundle
}

const [engineBundle, stubbedBundle] = await Promise.all([
  loadProbeBundle({ withAriaStubs: false }),
  loadProbeBundle({ withAriaStubs: true }),
])

type RectLike = {
  x: number
  y: number
  width: number
  height: number
  left: number
  top: number
  right: number
  bottom: number
}

function makeRect(
  x: number,
  y: number,
  width: number,
  height: number,
): RectLike {
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

type SavedDescriptor = {
  target: object
  key: PropertyKey
  descriptor: PropertyDescriptor | undefined
}

const DOM_GLOBALS = [
  'window',
  'document',
  'Node',
  'Element',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLFormElement',
  'HTMLIFrameElement',
  'Document',
  'DocumentFragment',
  'ShadowRoot',
  'Text',
  'Attr',
  'CharacterData',
  'MutationObserver',
  'CustomEvent',
  'Event',
  'MouseEvent',
  'NodeList',
  'getComputedStyle',
  'location',
] as const

type FixtureContext = {
  document: Document
  window: ReturnType<typeof parseHTML>['window']
  byId: (id: string) => Element
  setRect: (element: Element, rect: RectLike) => void
  setStyle: (element: Element, overrides: Record<string, string>) => void
  setHits: (fn: (x: number, y: number) => Element[]) => void
}

type PatchFn = (
  target: object,
  key: PropertyKey,
  descriptor: PropertyDescriptor,
) => void

type FixtureWindow = ReturnType<typeof parseHTML>['window']

const DEFAULT_STYLE: Readonly<Record<string, string>> = {
  display: 'block',
  visibility: 'visible',
  opacity: '1',
  position: 'static',
  pointerEvents: 'auto',
  overflowX: 'visible',
  overflowY: 'visible',
  borderLeftWidth: '0px',
  borderTopWidth: '0px',
  cursor: 'auto',
  content: 'none',
  transform: 'none',
}

function createPatcher(): { patch: PatchFn; restore: () => void } {
  const saved: SavedDescriptor[] = []
  const patch: PatchFn = (target, key, descriptor) => {
    saved.push({
      target,
      key,
      descriptor: Object.getOwnPropertyDescriptor(target, key),
    })
    Object.defineProperty(target, key, { configurable: true, ...descriptor })
  }
  const restore = () => {
    for (let index = saved.length - 1; index >= 0; index--) {
      const entry = saved[index]
      if (!entry) continue
      if (entry.descriptor)
        Object.defineProperty(entry.target, entry.key, entry.descriptor)
      else Reflect.deleteProperty(entry.target, entry.key)
    }
  }
  return { patch, restore }
}

function patchValue(
  patch: PatchFn,
  target: object,
  key: PropertyKey,
  value: unknown,
): void {
  patch(target, key, { writable: true, enumerable: true, value })
}

function createStyleReader(styles: WeakMap<Element, Record<string, string>>) {
  return (element: Element) => {
    const custom = styles.get(element) ?? {}
    return {
      ...DEFAULT_STYLE,
      ...custom,
      getPropertyValue: (name: string) => custom[name] ?? '',
    }
  }
}

function installLayoutShims(
  patch: PatchFn,
  window: FixtureWindow,
  document: Document,
  rects: WeakMap<Element, RectLike>,
  styles: WeakMap<Element, Record<string, string>>,
  getHits: () => (x: number, y: number) => Element[],
): void {
  const defaultRect = makeRect(0, 0, 100, 40)
  patchValue(patch, window, 'getComputedStyle', createStyleReader(styles))
  patchValue(patch, window, 'location', {
    href: 'https://fixture.test/page',
    origin: 'https://fixture.test',
  })
  patchValue(patch, window, 'origin', 'https://fixture.test')
  patchValue(patch, document, 'elementsFromPoint', (x: number, y: number) =>
    getHits()(x, y),
  )
  patchValue(
    patch,
    document,
    'elementFromPoint',
    (x: number, y: number) => getHits()(x, y)[0] ?? null,
  )
  patchValue(patch, window.Element.prototype, 'scrollIntoView', () => {})
  patchValue(patch, window.Element.prototype, 'checkVisibility', () => true)
  patchValue(
    patch,
    window.Element.prototype,
    'getBoundingClientRect',
    function (this: Element) {
      return rects.get(this) ?? defaultRect
    },
  )
  patchValue(
    patch,
    window.Element.prototype,
    'getClientRects',
    function (this: Element) {
      const rect = rects.get(this) ?? defaultRect
      return rect.width > 0 && rect.height > 0 ? [rect] : []
    },
  )
}

function installSelectorShims(
  patch: PatchFn,
  window: FixtureWindow,
  document: Document,
): void {
  const origMatches = window.Element.prototype.matches
  patchValue(
    patch,
    window.Element.prototype,
    'matches',
    function (this: Element, selector: string) {
      if (selector === ':modal' || selector === 'dialog:modal')
        return this.localName === 'dialog' && this.hasAttribute('data-modal')
      if (selector === ':popover-open')
        return this.hasAttribute('data-popover-open')
      return Reflect.apply(origMatches, this, [selector])
    },
  )
  const origDocQsa = document.querySelectorAll
  patchValue(
    patch,
    document,
    'querySelectorAll',
    function (this: Document, selector: string) {
      if (selector === 'dialog:modal')
        return Array.from(
          Reflect.apply(origDocQsa, this, ['dialog[data-modal]']),
        )
      return Reflect.apply(origDocQsa, this, [selector])
    },
  )
}

function installFormShims(patch: PatchFn, window: FixtureWindow): void {
  const inputProto = window.HTMLInputElement.prototype
  patch(window.HTMLElement.prototype, 'innerText', {
    enumerable: true,
    get(this: HTMLElement) {
      return this.textContent ?? ''
    },
  })
  patch(inputProto, 'defaultValue', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.getAttribute('value') ?? ''
    },
    set(this: HTMLInputElement, value: string) {
      this.setAttribute('value', value)
    },
  })
  patch(window.HTMLTextAreaElement.prototype, 'defaultValue', {
    enumerable: true,
    get(this: HTMLTextAreaElement) {
      return this.textContent ?? ''
    },
    set(this: HTMLTextAreaElement, value: string) {
      this.textContent = value
    },
  })
  patch(inputProto, 'type', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return (this.getAttribute('type') ?? 'text').toLowerCase()
    },
  })
  patch(inputProto, 'checked', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('checked')
    },
  })
  patch(inputProto, 'maxLength', {
    enumerable: true,
    get(this: HTMLInputElement) {
      const parsed = Number.parseInt(this.getAttribute('maxlength') ?? '', 10)
      return Number.isFinite(parsed) && parsed >= 0 ? parsed : -1
    },
  })
  patch(inputProto, 'disabled', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('disabled')
    },
  })
  patch(inputProto, 'readOnly', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('readonly')
    },
  })
}

function installDomGlobals(patch: PatchFn, window: FixtureWindow): void {
  for (const name of DOM_GLOBALS)
    patchValue(patch, globalThis, name, Reflect.get(window, name))
  patchValue(patch, globalThis, 'innerWidth', 1280)
  patchValue(patch, globalThis, 'innerHeight', 800)
  patchValue(patch, globalThis, 'CSS', {
    escape: (value: string) => value.replace(/([^\w-])/g, '\\$1'),
  })
}

function withFixtureDom<T>(html: string, fn: (ctx: FixtureContext) => T): T {
  const { window, document } = parseHTML(html)
  const { patch, restore } = createPatcher()
  const rects = new WeakMap<Element, RectLike>()
  const styles = new WeakMap<Element, Record<string, string>>()
  let hitResolver: (x: number, y: number) => Element[] = () => []

  installLayoutShims(patch, window, document, rects, styles, () => hitResolver)
  installSelectorShims(patch, window, document)
  installFormShims(patch, window)
  installDomGlobals(patch, window)

  const byId = (id: string): Element => {
    const found = document.getElementById(id)
    if (!found) throw new Error(`missing #${id}`)
    return found
  }

  try {
    return fn({
      document,
      window,
      byId,
      setRect: (element, rect) => rects.set(element, rect),
      setStyle: (element, overrides) => styles.set(element, overrides),
      setHits: (resolver) => {
        hitResolver = resolver
      },
    })
  } finally {
    restore()
  }
}

function makeWorld(
  bundle: ProbeBundle,
  seq = 0,
  epoch = 'test',
): {
  world: WorldMod.World
  output: FailureOutputMod.FailureOutput
} {
  const world: WorldMod.World = {
    epoch,
    seq,
    injected: bundle.createEngine(seq),
    pins: new bundle.Pins(),
    refs: new bundle.RefMap(),
  }
  const output = new bundle.failureOutput.FailureOutput(world, {
    conformance: false,
    discoverMovedFields: false,
  })
  return { world, output }
}

describe('cover-off-aria: source boundary', () => {
  it('keeps every engine read out of the cover, the survey and the private look', () => {
    // Each of these throws or answers null in the shipped bundle (the stand-ins of stubs/).
    const forbidden = [
      'getAriaRole',
      'getElementAccessibleName',
      'ariaSnapshotJSON',
      'forElement',
      'lastSnapshotRefs',
      'lastSnapshotNodes',
      'snapshot-aria',
    ]
    for (const relPath of [
      'src/probe/cover.ts',
      'src/probe/cover-survey.ts',
      'src/probe/private-look.ts',
    ]) {
      const source = readFileSync(resolve(PACKAGE_ROOT, relPath), 'utf8')
      for (const symbol of forbidden)
        expect({ relPath, reads: source.includes(symbol), symbol }).toEqual({
          relPath,
          reads: false,
          symbol,
        })
    }
  })

  it('bundles the stand-in roleUtils and no engine provider into the committed injected.js', () => {
    const injected = readFileSync(
      resolve(PACKAGE_ROOT, '../neo-playwright/assets/injected.js'),
      'utf8',
    )
    expect(injected).toContain('neo-playwright-js/src/stubs/roleUtils.ts')
    expect(injected).not.toContain('neo-playwright-js/src/probe/cover-aria.ts')
    expect(injected).not.toContain(
      'third_party/playwright/packages/injected/src/roleUtils.ts',
    )
  })
})

describe('cover-off-aria: the stand-in role', () => {
  it('answers what the vendored engine answers for every role a cover decision reads', () => {
    const html = `<html><body>
      <main id="main">
        <header id="hdr">Top</header>
        <nav id="nav"><a id="link" href="/go">Go</a><a id="plain-a">No href</a></nav>
        <section id="sec-named" aria-label="Filters">Filters</section>
        <section id="sec-bare">Bare</section>
        <button id="btn">Save</button>
        <input id="chk" type="checkbox">
        <input id="rad" type="radio">
        <div id="sw" role="switch" aria-checked="false"></div>
        <div id="pres" role="presentation">Pres</div>
        <div id="none-focus" role="none" tabindex="0">Focusable none</div>
        <div id="plain">Plain</div>
      </main>
      <dialog id="dlg-native" open><button id="dlg-close">Close</button></dialog>
      <div id="dlg-role" role="dialog" aria-label="Cookies"></div>
      <div id="alertdlg" role="alertdialog">Alert</div>
      <div id="status" role="status">Saved</div>
      <div id="alert" role="alert">Error</div>
      <div id="log" role="log">Log</div>
      <div id="menu" role="menu">Menu</div>
      <div id="listbox" role="listbox">List</div>
      <div id="tooltip" role="tooltip">Tip</div>
      <div id="fallback-role" role="bogus alertdialog dialog">Fallback</div>
    </body></html>`
    withFixtureDom(html, ({ document }) => {
      const { world } = makeWorld(engineBundle, 0, 'parity')
      for (const element of Array.from(document.querySelectorAll('[id]'))) {
        const vendored = world.injected.utils.getAriaRole(element) ?? ''
        expect({
          id: element.id,
          role: stubbedBundle.cover.roleOf(element),
        }).toEqual({
          id: element.id,
          role: vendored,
        })
        expect(computeAriaRole(element) ?? '').toBe(vendored)
      }
    })
  })
})

describe('cover-off-aria: the page decides from its own facts', () => {
  const COVERED = `<html><body>
    <main id="main">
      <button id="target">Pay</button>
      <label id="email-label" for="email">Email</label>
      <input id="email" type="text">
      <div id="chk-wrap">
        <input id="chk" type="checkbox" checked aria-label="Accept terms">
        <span id="chk-veil">Accept terms</span>
      </div>
      <div id="float-wrap">
        <input id="float-input" type="text" placeholder="Email">
        <span id="float-label">Email address</span>
      </div>
      <div id="plain-target">Not a point role</div>
    </main>
    <div id="backdrop"></div>
    <div id="dialog" role="dialog" aria-label="Consent">
      <p id="dialog-copy">Accept cookies</p>
      <button id="accept">Accept</button>
    </div>
    <div id="wrapper-with-sole">
      <div id="sole" role="alertdialog" aria-label="Confirm">
        <button id="sole-ok">OK</button>
      </div>
    </div>
    <div id="named-sheet" aria-label="Named sheet">
      <div id="inner" role="dialog"><button id="inner-ok">OK</button></div>
    </div>
    <div id="icon-veil"><span id="icon" title="Loading"></span></div>
    <div id="toast" role="status">Saved</div>
    <iframe id="frame" src="https://pay.example"></iframe>
  </body></html>`

  type Laid = FixtureContext & {
    world: WorldMod.World
    output: FailureOutputMod.FailureOutput
  }

  function laidOut(run: (laid: Laid) => void) {
    withFixtureDom(COVERED, (context) => {
      const { world, output } = makeWorld(stubbedBundle, 0, 'stubbed-cover')
      const { byId, setRect, setStyle } = context
      Reflect.set(byId('email-label'), 'control', byId('email'))
      setRect(byId('target'), makeRect(100, 100, 120, 40))
      setRect(byId('plain-target'), makeRect(100, 200, 120, 40))
      setRect(byId('backdrop'), makeRect(0, 0, 1280, 800))
      setRect(byId('dialog'), makeRect(200, 150, 400, 250))
      setRect(byId('accept'), makeRect(250, 250, 100, 40))
      setRect(byId('chk'), makeRect(10, 10, 20, 20))
      setRect(byId('chk-veil'), makeRect(10, 10, 20, 20))
      setRect(byId('float-input'), makeRect(300, 400, 200, 40))
      setRect(byId('float-label'), makeRect(310, 405, 100, 25))
      for (const id of ['backdrop', 'dialog', 'toast'])
        setStyle(byId(id), { position: 'fixed' })
      run({ ...context, world, output })
    })
  }

  const coverRequest = (
    selector: string,
    point: { x: number; y: number },
    cover: number,
  ) => ({
    op: 'cover' as const,
    target: { chunk: parseSelector(selector) },
    point,
    cover,
    above: false,
  })

  it('names the dialog, finds a blank backdrop and the dialog it dims, and lists controls', () => {
    laidOut(({ world, output, byId, setHits }) => {
      const { cover } = stubbedBundle
      const [dialog, backdrop, accept] = ['dialog', 'backdrop', 'accept'].map(
        byId,
      )
      expect(
        cover.whichDialog([accept, dialog], dialog, { x: 260, y: 260 }),
      ).toBe(dialog)
      expect(cover.isBlank(world, output, backdrop)).toBe(true)
      expect(cover.isBlank(world, output, dialog)).toBe(false)
      // A title is words, as a snapshot prints the icon's name: not blank.
      expect(cover.isBlank(world, output, byId('icon-veil'))).toBe(false)
      setHits((x, y) =>
        x >= 250 && x <= 350 && y >= 250 && y <= 290
          ? [accept, dialog, backdrop]
          : [dialog, backdrop],
      )
      expect(cover.listedControls(world, dialog)).toEqual([accept])
      expect(cover.dialogBehind(world, backdrop, { x: 110, y: 110 })).toBe(
        dialog,
      )
      expect(cover.shownAs(world, output, backdrop, { x: 110, y: 110 })).toBe(
        dialog,
      )
      // A nameless wrapper is shown as its one dialog; one named by its own label is not.
      expect(
        cover.shownAs(world, output, byId('wrapper-with-sole'), { x: 1, y: 1 }),
      ).toBe(byId('sole'))
      expect(
        cover.shownAs(world, output, byId('named-sheet'), { x: 1, y: 1 }),
      ).toBe(byId('named-sheet'))
    })
  })

  it('reads the kinds a labelled control and a field drawing make', () => {
    laidOut(({ byId }) => {
      const { cover } = stubbedBundle
      expect(
        cover.kindOf(
          byId('email-label'),
          [byId('email')],
          { x: 10, y: 10 },
          false,
        ),
      ).toEqual({ is: 'labels', control: byId('email') })
      // The input's own placeholder names it, so the label drawn over it is its drawing.
      expect(
        cover.kindOf(
          byId('float-input'),
          [byId('float-label')],
          { x: 320, y: 410 },
          false,
        ),
      ).toEqual({ is: 'wrapper' })
    })
  })

  it('answers the cover request with pins and the page facts, and no name or ref of its own', () => {
    laidOut(({ world, output, byId, setHits }) => {
      const { cover } = stubbedBundle
      const [dialog, backdrop, accept] = ['dialog', 'backdrop', 'accept'].map(
        byId,
      )
      setHits((x, y) => {
        if (x === 160 && y === 120) return [backdrop]
        if (x >= 250 && x <= 350 && y >= 250 && y <= 290)
          return [accept, dialog, backdrop]
        return [dialog, backdrop]
      })
      const mark = cover.coverMark(byId('target'), { x: 160, y: 120 })
      const reply = cover.cover(
        world,
        coverRequest('#target', { x: 160, y: 120 }, mark?.id ?? 0),
        undefined,
        [],
        output,
      )
      if (reply.kind !== 'cover' || !reply.facts)
        throw new Error(JSON.stringify(reply))
      const { facts } = reply
      expect(Object.keys(facts)).not.toContain('ref')
      expect(Object.keys(facts)).not.toContain('name')
      expect(world.pins.live(facts.pin)).toBe(dialog)
      expect(facts.controls.map((pin) => world.pins.live(pin))).toEqual([
        accept,
      ])
      expect(facts).toMatchObject({
        behind: true,
        blank: false,
        surrounds: false,
        notice: false,
        kind: { is: 'page' },
      })
      expect(facts.text).toContain('Accept cookies')

      setHits(() => [byId('toast')])
      const toastMark = cover.coverMark(byId('target'), { x: 160, y: 120 })
      const toast = cover.cover(
        world,
        coverRequest('#target', { x: 160, y: 120 }, toastMark?.id ?? 0),
        undefined,
        [],
        output,
      )
      if (toast.kind !== 'cover' || !toast.facts)
        throw new Error(JSON.stringify(toast))
      expect(world.pins.live(toast.facts.pin)).toBe(byId('toast'))
      expect(toast.facts.notice).toBe(true)
    })
  })

  it('pins the control a labels cover is, in the same entry as the cover', () => {
    laidOut(({ world, output, byId, setHits }) => {
      const { cover } = stubbedBundle
      const email = byId('email')
      setHits(() => [email])
      const mark = cover.coverMark(byId('email-label'), { x: 10, y: 10 })
      const reply = cover.cover(
        world,
        coverRequest('#email-label', { x: 10, y: 10 }, mark?.id ?? 0),
        undefined,
        [],
        output,
      )
      if (reply.kind !== 'cover' || !reply.facts)
        throw new Error(JSON.stringify(reply))
      const { kind } = reply.facts
      if (kind.is !== 'labels') throw new Error(JSON.stringify(kind))
      expect(world.pins.live(kind.pin)).toBe(email)
    })
  })

  it("decides a press of the target's own from the stand-in role and the live state", () => {
    laidOut(({ world, output, byId, setHits }) => {
      const { cover } = stubbedBundle
      const chk = byId('chk')
      setHits(() => [byId('chk-veil')])
      const mark = cover.coverMark(chk, { x: 20, y: 20 })
      expect(
        cover.ownPress(
          world,
          output,
          chk,
          { x: 20, y: 20 },
          {
            wrapper: mark?.id,
          },
        )?.press,
      ).toMatchObject({ case: 'wrapper', checked: true })

      setHits((x, y) => {
        if (x === 160 && (y === 120 || y === 220)) return [byId('dialog')]
        if (y >= 100 && y < 140) return [byId('target')]
        return [byId('plain-target')]
      })
      expect(
        cover.ownPress(world, output, byId('target'), { x: 160, y: 120 }, {})
          ?.press.case,
      ).toBe('point')
      expect(
        cover.ownPress(
          world,
          output,
          byId('plain-target'),
          { x: 160, y: 220 },
          {},
        ),
      ).toBeUndefined()
    })
  })
})

describe('cover-off-aria: the private look', () => {
  const FIELD = `<html><body>
    <span id="hint">Verification code</span>
    <div id="field-wrap">
      <input id="code" type="text">
      <span id="floating">Verification code</span>
    </div>
    <label id="card-label" for="card">Card number</label><input id="card" type="text">
  </body></html>`

  it("compares the drawing with the name hint, else with the box's own label", () => {
    withFixtureDom(FIELD, ({ byId, setRect, setHits }) => {
      const { world } = makeWorld(stubbedBundle, 0, 'stubbed-look')
      const [wrap, input, floating] = ['field-wrap', 'code', 'floating'].map(
        byId,
      )
      setRect(input, makeRect(100, 100, 200, 40))
      setRect(floating, makeRect(110, 105, 120, 25))
      setHits(() => [floating])
      const { drawingOf, lookAt, coverWrapper } = stubbedBundle.privateLook
      expect(drawingOf(input, floating, false, 'Verification code')).toBe(wrap)
      // No hint and no label of its own: the drawn words are another's, so they cover it.
      expect(drawingOf(input, floating, false)).toBeNull()
      input.setAttribute('aria-labelledby', 'hint')
      expect(drawingOf(input, floating, false)).toBe(wrap)
      input.removeAttribute('aria-labelledby')

      const ask = {
        cut: false,
        keys: false,
        frameVisible: true,
        scroll: 'asItIs' as const,
      }
      expect(
        lookAt(world, input, { ...ask, name: 'Verification code' }).refusal,
      ).toBeUndefined()
      expect(coverWrapper(input)).toBe(wrap)
      expect(
        lookAt(world, input, { ...ask, name: 'Completely different label' })
          .refusal,
      ).toEqual({ code: 'boxCovered' })
      expect(coverWrapper(input)).toBeUndefined()
    })
  })

  it("reads the box's own label in the order the accessible name tries them", () => {
    withFixtureDom(FIELD, ({ document, byId }) => {
      const { labelOf } = stubbedBundle.privateLook
      const card = byId('card')
      Object.defineProperty(card, 'labels', {
        configurable: true,
        value: document.querySelectorAll('#card-label'),
      })
      expect(labelOf(card)).toBe('Card number')
      card.setAttribute('title', 'A title comes after the label')
      card.setAttribute('aria-label', '  Card   holder ')
      expect(labelOf(card)).toBe('Card holder')
      card.setAttribute('aria-labelledby', 'hint')
      expect(labelOf(card)).toBe('Verification code')
      const code = byId('code')
      expect(labelOf(code)).toBe('')
      code.setAttribute('placeholder', 'Code')
      expect(labelOf(code)).toBe('Code')
    })
  })
})

describe('cover-off-aria: inert marking keeps its seam', () => {
  it('runs markInert through an injected snapshotRefs', () => {
    const html = `<html><body>
      <main id="bg" inert>
        <button id="bg-btn">Behind</button>
      </main>
      <div id="live">
        <button id="live-btn">Active</button>
      </div>
    </body></html>`

    withFixtureDom(html, ({ byId }) => {
      const { world } = makeWorld(stubbedBundle, 0, 'stubbed-inert')
      const byRef = new Map<string, Element>([
        ['e1', byId('bg')],
        ['e2', byId('bg-btn')],
        ['e3', byId('live')],
        ['e4', byId('live-btn')],
      ])
      const bgNode: AriaNodeJSON = {
        role: 'main',
        ref: 'e1',
        children: [{ role: 'button', name: 'Behind', ref: 'e2' }],
      }
      const liveNode: AriaNodeJSON = {
        role: 'generic',
        ref: 'e3',
        children: [{ role: 'button', name: 'Active', ref: 'e4' }],
      }
      const json: AriaSnapshotJSON = [bgNode, liveNode]
      stubbedBundle.snapshotInert.markInert(world, json, {
        snapshotRefs: () => ({
          elementFor: (ref) => byRef.get(ref),
          elements: () => byRef.values(),
        }),
      })
      expect(bgNode.inert).toBe(true)
      expect(liveNode.inert).toBeUndefined()
    })
  })
})
