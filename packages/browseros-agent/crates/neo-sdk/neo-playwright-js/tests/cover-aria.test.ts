import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { parseHTML } from 'linkedom'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { AriaNodeJSON, AriaSnapshotJSON } from '../src/prelude/vendored'
import type * as CoverMod from '../src/probe/cover'
import type * as CoverAriaMod from '../src/probe/cover-aria'
import type * as CoverSurveyMod from '../src/probe/cover-survey'
import type * as FailureOutputMod from '../src/probe/failure-output'
import type * as PrivateLookMod from '../src/probe/private-look'
import type * as SnapshotAriaMod from '../src/probe/snapshot-aria'
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
  coverAria: typeof CoverAriaMod
  coverSurvey: typeof CoverSurveyMod
  failureOutput: typeof FailureOutputMod
  privateLook: typeof PrivateLookMod
  snapshotAria: typeof SnapshotAriaMod
  snapshotInert: typeof SnapshotInertMod
  createEngine: typeof WorldMod.createEngine
  Pins: typeof WorldMod.Pins
  RefMap: new () => WorldMod.World['refs']
}

async function loadProbeBundle(options: {
  withAriaStubs: boolean
}): Promise<ProbeBundle> {
  const stubs: Record<string, string> = {
    './highlight': resolve(PACKAGE_ROOT, 'src/stubs/highlight.ts'),
    './consoleApi': resolve(PACKAGE_ROOT, 'src/stubs/consoleApi.ts'),
    ...(options.withAriaStubs
      ? {
          './roleUtils': resolve(PACKAGE_ROOT, 'src/stubs/roleUtils.ts'),
          './ariaSnapshot': resolve(PACKAGE_ROOT, 'src/stubs/ariaSnapshot.ts'),
          './roleSelectorEngine': resolve(
            PACKAGE_ROOT,
            'src/stubs/roleSelectorEngine.ts',
          ),
        }
      : {}),
  }
  const entryPath = resolve(PACKAGE_ROOT, 'src/probe/__cover-aria-entry.ts')
  const entrySource = `
    export * as cover from './cover'
    export * as coverAria from './cover-aria'
    export * as coverSurvey from './cover-survey'
    export * as failureOutput from './failure-output'
    export * as privateLook from './private-look'
    export * as snapshotAria from './snapshot-aria'
    export * as snapshotInert from './snapshot-inert'
    export { createEngine, Pins } from './world'
    export { RefMap } from './aria-ref'
  `
  const result = await Bun.build({
    entrypoints: [entryPath],
    target: 'browser',
    format: 'esm',
    minify: false,
    plugins: [
      {
        name: 'cover-aria-test-bundle',
        setup(build) {
          build.onResolve({ filter: /__cover-aria-entry\.ts$/ }, () => ({
            path: entryPath,
          }))
          build.onLoad({ filter: /__cover-aria-entry\.ts$/ }, () => ({
            contents: entrySource,
            loader: 'ts',
          }))
          build.onResolve(
            {
              filter:
                /^\.\/(highlight|consoleApi|roleUtils|ariaSnapshot|roleSelectorEngine)$/,
            },
            (args) => {
              const target = stubs[args.path]
              return target ? { path: target } : undefined
            },
          )
          build.onResolve({ filter: /^@isomorphic\// }, (args) => ({
            path: resolve(
              VENDOR_ROOT,
              'packages/isomorphic',
              `${args.path.slice('@isomorphic/'.length)}.ts`,
            ),
          }))
        },
      },
    ],
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

function withFixtureDom<T>(html: string, fn: (ctx: FixtureContext) => T): T {
  const { window, document } = parseHTML(html)
  const saved: SavedDescriptor[] = []
  const patch = (
    target: object,
    key: PropertyKey,
    descriptor: PropertyDescriptor,
  ) => {
    saved.push({
      target,
      key,
      descriptor: Object.getOwnPropertyDescriptor(target, key),
    })
    Object.defineProperty(target, key, { configurable: true, ...descriptor })
  }

  const rects = new WeakMap<Element, RectLike>()
  const styles = new WeakMap<Element, Record<string, string>>()
  const defaultRect = makeRect(0, 0, 100, 40)

  const styleFor = (element: Element) => {
    const custom = styles.get(element) ?? {}
    return {
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
      ...custom,
      getPropertyValue: (name: string) => custom[name] ?? '',
    }
  }

  let hitResolver: (x: number, y: number) => Element[] = () => []
  const elementsFromPoint = (x: number, y: number) => hitResolver(x, y)
  const elementFromPoint = (x: number, y: number) =>
    hitResolver(x, y)[0] ?? null

  patch(window, 'getComputedStyle', {
    writable: true,
    enumerable: true,
    value: styleFor,
  })
  patch(window, 'location', {
    writable: true,
    enumerable: true,
    value: {
      href: 'https://fixture.test/page',
      origin: 'https://fixture.test',
    },
  })
  patch(window, 'origin', {
    writable: true,
    enumerable: true,
    value: 'https://fixture.test',
  })
  patch(document, 'elementsFromPoint', {
    writable: true,
    enumerable: true,
    value: elementsFromPoint,
  })
  patch(document, 'elementFromPoint', {
    writable: true,
    enumerable: true,
    value: elementFromPoint,
  })
  patch(window.Element.prototype, 'scrollIntoView', {
    writable: true,
    enumerable: true,
    value() {},
  })
  const origMatches = window.Element.prototype.matches
  patch(window.Element.prototype, 'matches', {
    writable: true,
    enumerable: true,
    value(this: Element, selector: string) {
      if (selector === ':modal' || selector === 'dialog:modal')
        return this.localName === 'dialog' && this.hasAttribute('data-modal')
      if (selector === ':popover-open')
        return this.hasAttribute('data-popover-open')
      return Reflect.apply(origMatches, this, [selector])
    },
  })
  const origDocQsa = document.querySelectorAll
  patch(document, 'querySelectorAll', {
    writable: true,
    enumerable: true,
    value(this: Document, selector: string) {
      if (selector === 'dialog:modal')
        return Array.from(
          Reflect.apply(origDocQsa, this, ['dialog[data-modal]']),
        )
      return Reflect.apply(origDocQsa, this, [selector])
    },
  })
  patch(window.Element.prototype, 'checkVisibility', {
    writable: true,
    enumerable: true,
    value() {
      return true
    },
  })
  patch(window.Element.prototype, 'getBoundingClientRect', {
    writable: true,
    enumerable: true,
    value(this: Element) {
      return rects.get(this) ?? defaultRect
    },
  })
  patch(window.Element.prototype, 'getClientRects', {
    writable: true,
    enumerable: true,
    value(this: Element) {
      const rect = rects.get(this) ?? defaultRect
      return rect.width > 0 && rect.height > 0 ? [rect] : []
    },
  })
  patch(window.HTMLElement.prototype, 'innerText', {
    enumerable: true,
    get(this: HTMLElement) {
      return this.textContent ?? ''
    },
  })
  patch(window.HTMLInputElement.prototype, 'defaultValue', {
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
  patch(window.HTMLInputElement.prototype, 'type', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return (this.getAttribute('type') ?? 'text').toLowerCase()
    },
  })
  patch(window.HTMLInputElement.prototype, 'checked', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('checked')
    },
  })
  patch(window.HTMLInputElement.prototype, 'maxLength', {
    enumerable: true,
    get(this: HTMLInputElement) {
      const raw = this.getAttribute('maxlength')
      if (raw === null) return -1
      const parsed = Number.parseInt(raw, 10)
      return Number.isFinite(parsed) && parsed >= 0 ? parsed : -1
    },
  })
  patch(window.HTMLInputElement.prototype, 'disabled', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('disabled')
    },
  })
  patch(window.HTMLInputElement.prototype, 'readOnly', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return this.hasAttribute('readonly')
    },
  })

  for (const name of DOM_GLOBALS) {
    patch(globalThis, name, {
      writable: true,
      enumerable: true,
      value: Reflect.get(window, name),
    })
  }
  patch(globalThis, 'innerWidth', {
    writable: true,
    enumerable: true,
    value: 1280,
  })
  patch(globalThis, 'innerHeight', {
    writable: true,
    enumerable: true,
    value: 800,
  })
  patch(globalThis, 'CSS', {
    writable: true,
    enumerable: true,
    value: { escape: (value: string) => value.replace(/([^\w-])/g, '\\$1') },
  })

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
    for (let index = saved.length - 1; index >= 0; index--) {
      const entry = saved[index]
      if (!entry) continue
      if (entry.descriptor)
        Object.defineProperty(entry.target, entry.key, entry.descriptor)
      else Reflect.deleteProperty(entry.target, entry.key)
    }
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
  it('forbids direct ARIA engine calls and snapshot internal reads outside cover-aria.ts', () => {
    const coverFiles = [
      'src/probe/cover.ts',
      'src/probe/cover-survey.ts',
      'src/probe/private-look.ts',
      'src/probe/snapshot-inert.ts',
    ]
    const forbidden = [
      'getAriaRole',
      'getElementAccessibleNameText',
      'getElementAccessibleName',
      'ariaSnapshotJSON',
      'forElement',
      'lastSnapshotRefs',
      'lastSnapshotNodes',
    ]
    for (const relPath of coverFiles) {
      const source = readFileSync(resolve(PACKAGE_ROOT, relPath), 'utf8')
      for (const symbol of forbidden) {
        expect(source).not.toContain(symbol)
      }
    }

    const providerSource = readFileSync(
      resolve(PACKAGE_ROOT, 'src/probe/cover-aria.ts'),
      'utf8',
    )
    expect(providerSource).toContain('getAriaRole')
    expect(providerSource).toContain('getElementAccessibleNameText')
    expect(providerSource).toContain('ariaSnapshotJSON')
    expect(providerSource).toContain('forElement')

    const snapshotProviderSource = readFileSync(
      resolve(PACKAGE_ROOT, 'src/probe/snapshot-aria.ts'),
      'utf8',
    )
    expect(snapshotProviderSource).toContain('lastSnapshotRefs')
    expect(snapshotProviderSource).toContain('lastSnapshotNodes')
  })

  it('keeps stubs/roleUtils unbundled in committed injected.js until switch-over', () => {
    const injected = readFileSync(
      resolve(PACKAGE_ROOT, '../neo-playwright/assets/injected.js'),
      'utf8',
    )
    expect(injected).toContain('neo-playwright-js/src/probe/cover-aria.ts')
    expect(injected).toContain('neo-playwright-js/src/probe/snapshot-aria.ts')
    expect(injected).not.toContain('neo-playwright-js/src/stubs/roleUtils.ts')
  })
})

describe('cover-off-aria: default engine provider', () => {
  it('matches stubs/roleUtils computeAriaRole across cover, control, landmark, and presentation roles', () => {
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
      const aria = engineBundle.coverAria.coverAriaFor(world)
      expect(engineBundle.coverAria.coverAriaFor(world)).toBe(aria)

      const rekeyed = makeWorld(engineBundle, 1, 'parity-2').world
      expect(engineBundle.coverAria.coverAriaFor(rekeyed)).not.toBe(aria)

      for (const element of Array.from(document.querySelectorAll('[id]'))) {
        expect(aria.roleOf(element)).toBe(computeAriaRole(element))
      }
    })
  })

  it('serves names, refs, quiet cover snapshots, survey elements, snapshot refs, and node links without clobbering the latest ai snapshot', () => {
    const html = `<html><body>
      <main id="main">
        <span id="email-lbl">Work email</span>
        <input id="email" type="text" aria-labelledby="email-lbl">
        <button id="submit">Pay now</button>
      </main>
      <div id="modal" role="dialog" aria-label="Cookie choices">
        <p>We use cookies.</p>
        <button id="accept">Accept</button>
      </div>
      <div id="veil"></div>
      <div id="whitespace-veil">   \n\t  </div>
    </body></html>`

    withFixtureDom(html, ({ document, byId }) => {
      const { world } = makeWorld(engineBundle, 2, 'engine-provider')
      const aria = engineBundle.snapshotAria.snapshotAriaFor(world)
      expect(engineBundle.snapshotAria.snapshotAriaFor(world)).toBe(aria)

      expect(aria.nameOf(byId('email'))).toBe('Work email')
      expect(aria.nameOf(byId('submit'))).toBe('Pay now')
      expect(aria.nameOf(byId('modal'))).toBe('Cookie choices')

      const minted = aria.refOf(byId('veil'))
      expect(minted).toBe('f2n1')
      expect(aria.refOf(byId('veil'))).toBe('f2n1')

      const ai = world.injected.ariaSnapshotJSON(document.body, { mode: 'ai' })
      const surveyIterable = aria.surveyElements()
      const initialElements = Array.from(surveyIterable)
      expect(initialElements.length).toBeGreaterThan(0)
      expect(initialElements).toContain(byId('submit'))
      expect(initialElements).toContain(byId('accept'))

      const secondPass = Array.from(surveyIterable)
      expect(secondPass).toEqual(initialElements)

      const snapRefs = aria.snapshotRefs()
      const refEntries = new Map<string, { element: Element }>()
      const walk = (nodes: AriaSnapshotJSON) => {
        for (const node of nodes) {
          if (node.ref) {
            const el = snapRefs.elementFor(node.ref)
            if (el) refEntries.set(node.ref, { element: el })
          }
          for (const child of node.children ?? []) {
            if (typeof child !== 'string') walk([child])
          }
        }
      }
      walk(ai.json)
      world.refs.remember(refEntries)
      expect(aria.refOf(byId('submit'))).toMatch(/^f2e\d+$/)

      const modalSnap = aria.coverSnapshot(byId('modal'))
      expect(modalSnap.hasWords).toBe(true)
      expect(modalSnap.json).toBeTruthy()
      expect(aria.coverSnapshot(byId('veil')).hasWords).toBe(false)
      expect(aria.coverSnapshot(byId('whitespace-veil')).hasWords).toBe(false)

      expect(Array.from(aria.surveyElements())).toEqual(initialElements)
      const nodes = aria.snapshotNodes(ai.json)
      expect(Array.from(nodes.values())).toContain(byId('modal'))
    })
  })
})

describe('cover-off-aria: stand-in isolation (zero direct engine ARIA calls)', () => {
  function forbidDirectRefs(world: WorldMod.World): void {
    Object.defineProperty(world.refs, 'forElement', {
      configurable: true,
      value: () => {
        throw new Error('direct refs.forElement call forbidden')
      },
    })
  }

  function makeCustomAria(options: {
    names?: ReadonlyMap<Element, string>
    refs?: ReadonlyMap<Element, string>
    snapshots?: ReadonlyMap<Element, CoverAriaMod.CoverSnapshot>
    surveyElements?: () => Iterable<Element>
    snapshotRefs?: () => SnapshotAriaMod.SnapshotRefs
    snapshotNodes?: (
      json: AriaSnapshotJSON,
    ) => ReadonlyMap<AriaNodeJSON, Element>
  }): SnapshotAriaMod.SnapshotAria {
    return {
      roleOf: (element) => computeAriaRole(element),
      nameOf: (element) => options.names?.get(element) ?? '',
      refOf: (element) => options.refs?.get(element) ?? 'x0',
      coverSnapshot: (element) =>
        options.snapshots?.get(element) ?? { json: [], hasWords: false },
      surveyElements: options.surveyElements ?? (() => []),
      snapshotRefs:
        options.snapshotRefs ??
        (() => ({
          elementFor: () => undefined,
          elements: () => [],
        })),
      snapshotNodes: options.snapshotNodes ?? (() => new Map()),
    }
  }

  it('runs cover.ts helpers and the cover request purely through CoverAria with stubbed InjectedScript', () => {
    const html = `<html><body>
      <main id="main">
        <button id="target">Pay</button>
        <label id="email-label" for="email">Email</label>
        <input id="email" type="text">
        <div id="chk-wrap">
          <input id="chk" type="checkbox" checked>
          <span id="chk-veil">Accept terms</span>
        </div>
        <div id="float-wrap">
          <input id="float-input" type="text">
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
      <div id="toast" role="status">Saved</div>
      <iframe id="frame" src="https://pay.example"></iframe>
    </body></html>`

    withFixtureDom(html, ({ byId, setRect, setStyle, setHits }) => {
      const { world, output } = makeWorld(stubbedBundle, 0, 'stubbed-cover')
      forbidDirectRefs(world)

      expect(world.injected.utils.getAriaRole(byId('dialog'))).toBeNull()
      expect(() =>
        world.injected.utils.getElementAccessibleNameText(
          byId('accept'),
          false,
        ),
      ).toThrow(/Rust/)
      expect(() =>
        world.injected.ariaSnapshotJSON(byId('dialog'), { mode: 'default' }),
      ).toThrow(/Rust/)

      const target = byId('target')
      const email = byId('email')
      const emailLabel = byId('email-label')
      const chk = byId('chk')
      const chkVeil = byId('chk-veil')
      const floatInput = byId('float-input')
      const floatLabel = byId('float-label')
      const plainTarget = byId('plain-target')
      const backdrop = byId('backdrop')
      const dialog = byId('dialog')
      const accept = byId('accept')
      const wrapperWithSole = byId('wrapper-with-sole')
      const sole = byId('sole')
      const toast = byId('toast')
      const frame = byId('frame')

      Reflect.set(emailLabel, 'control', email)
      setRect(target, makeRect(100, 100, 120, 40))
      setRect(plainTarget, makeRect(100, 200, 120, 40))
      setRect(backdrop, makeRect(0, 0, 1280, 800))
      setRect(dialog, makeRect(200, 150, 400, 250))
      setRect(accept, makeRect(250, 250, 100, 40))
      setRect(chk, makeRect(10, 10, 20, 20))
      setRect(chkVeil, makeRect(10, 10, 20, 20))
      setRect(floatInput, makeRect(300, 400, 200, 40))
      setRect(floatLabel, makeRect(310, 405, 100, 25))
      setStyle(backdrop, { position: 'fixed' })
      setStyle(dialog, { position: 'fixed' })
      setStyle(toast, { position: 'fixed' })

      const aria = makeCustomAria({
        names: new Map([
          [dialog, 'Consent'],
          [accept, 'Accept'],
          [toast, 'Saved'],
          [chk, 'Accept terms'],
          [floatInput, 'Email address'],
        ]),
        refs: new Map([
          [dialog, 'r-dialog'],
          [accept, 'r-accept'],
          [email, 'r-email'],
          [toast, 'r-toast'],
        ]),
        snapshots: new Map([
          [backdrop, { json: [], hasWords: false }],
          [
            dialog,
            { json: [{ role: 'dialog', name: 'Consent' }], hasWords: true },
          ],
          [
            wrapperWithSole,
            {
              json: [{ role: 'alertdialog', name: 'Confirm' }],
              hasWords: true,
            },
          ],
          [
            toast,
            { json: [{ role: 'status', name: 'Saved' }], hasWords: true },
          ],
        ]),
      })

      expect(
        stubbedBundle.cover.whichDialog(
          world,
          [accept, dialog],
          dialog,
          { x: 260, y: 260 },
          aria,
        ),
      ).toBe(dialog)
      expect(stubbedBundle.cover.controlRole(world, frame, aria)).toBe('iframe')
      expect(stubbedBundle.cover.controlRole(world, dialog, aria)).toBe(
        'dialog',
      )
      expect(stubbedBundle.cover.nameOf(world, output, accept, aria)).toBe(
        'Accept',
      )
      expect(stubbedBundle.cover.isBlank(world, output, backdrop, aria)).toBe(
        true,
      )
      expect(stubbedBundle.cover.isBlank(world, output, dialog, aria)).toBe(
        false,
      )

      setHits((x, y) => {
        if (x >= 250 && x <= 350 && y >= 250 && y <= 290)
          return [accept, dialog, backdrop]
        return [dialog, backdrop]
      })
      expect(
        stubbedBundle.cover.dialogBehind(
          world,
          backdrop,
          { x: 110, y: 110 },
          aria,
        ),
      ).toBe(dialog)
      expect(
        stubbedBundle.cover.shownAs(
          world,
          output,
          backdrop,
          { x: 110, y: 110 },
          undefined,
          aria,
        ),
      ).toBe(dialog)
      expect(
        stubbedBundle.cover.shownAs(
          world,
          output,
          wrapperWithSole,
          { x: 110, y: 110 },
          undefined,
          aria,
        ),
      ).toBe(sole)

      const namedWrapperAria = makeCustomAria({
        names: new Map([[wrapperWithSole, 'Named sheet']]),
        snapshots: new Map([
          [
            wrapperWithSole,
            {
              json: [{ role: 'generic', name: 'Named sheet' }],
              hasWords: true,
            },
          ],
        ]),
      })
      expect(
        stubbedBundle.cover.shownAs(
          world,
          output,
          wrapperWithSole,
          { x: 110, y: 110 },
          undefined,
          namedWrapperAria,
        ),
      ).toBe(wrapperWithSole)

      expect(
        stubbedBundle.cover.controlsOf(world, output, dialog, [accept], aria),
      ).toEqual([{ role: 'button', name: 'Accept', ref: 'r-accept' }])

      expect(
        stubbedBundle.cover.kindOf(
          world,
          emailLabel,
          [email],
          { x: 10, y: 10 },
          false,
          aria,
        ),
      ).toEqual({ is: 'labels', ref: 'r-email' })
      expect(
        stubbedBundle.cover.kindOf(
          world,
          floatInput,
          [floatLabel],
          { x: 320, y: 410 },
          false,
          aria,
        ),
      ).toEqual({ is: 'wrapper' })

      setHits((x, y) => {
        if (x === 160 && y === 120) return [backdrop]
        if (x >= 250 && x <= 350 && y >= 250 && y <= 290)
          return [accept, dialog, backdrop]
        return [dialog, backdrop]
      })
      const mark = stubbedBundle.cover.coverMark(target, { x: 160, y: 120 })
      expect(mark).toBeDefined()
      const coverReply = stubbedBundle.cover.cover(
        world,
        {
          op: 'cover',
          target: { chunk: parseSelector('#target') },
          point: { x: 160, y: 120 },
          cover: mark?.id ?? 0,
          above: false,
        },
        undefined,
        [],
        output,
        aria,
      )
      expect(coverReply).toMatchObject({
        kind: 'cover',
        facts: {
          ref: 'r-dialog',
          role: 'dialog',
          name: 'Consent',
          behind: true,
          blank: false,
          notice: false,
          controls: [{ role: 'button', name: 'Accept', ref: 'r-accept' }],
        },
      })

      setHits(() => [toast])
      const toastMark = stubbedBundle.cover.coverMark(target, {
        x: 160,
        y: 120,
      })
      const toastReply = stubbedBundle.cover.cover(
        world,
        {
          op: 'cover',
          target: { chunk: parseSelector('#target') },
          point: { x: 160, y: 120 },
          cover: toastMark?.id ?? 0,
          above: false,
        },
        undefined,
        [],
        output,
        aria,
      )
      expect(toastReply).toMatchObject({
        kind: 'cover',
        facts: {
          ref: 'r-toast',
          role: 'status',
          name: 'Saved',
          notice: true,
        },
      })

      setHits(() => [chkVeil])
      const chkMark = stubbedBundle.cover.coverMark(chk, { x: 20, y: 20 })
      const wrapperPress = stubbedBundle.cover.ownPress(
        world,
        output,
        chk,
        { x: 20, y: 20 },
        { wrapper: chkMark?.id },
        aria,
      )
      expect(wrapperPress?.press).toMatchObject({
        case: 'wrapper',
        checked: true,
      })

      setHits((x, y) => {
        if (x === 160 && y === 120) return [dialog]
        if (x === 160 && y === 220) return [dialog]
        if (y >= 100 && y < 140) return [target]
        return [plainTarget]
      })
      const partialButton = stubbedBundle.cover.ownPress(
        world,
        output,
        target,
        { x: 160, y: 120 },
        {},
        aria,
      )
      expect(partialButton?.press.case).toBe('point')
      const partialDiv = stubbedBundle.cover.ownPress(
        world,
        output,
        plainTarget,
        { x: 160, y: 220 },
        {},
        aria,
      )
      expect(partialDiv).toBeUndefined()
    })
  })

  it('runs cover-survey.ts end to end through CoverAria and marks whole-view nodes', () => {
    const html = `<html><body>
      <main id="main">
        <button id="behind">Pay</button>
      </main>
      <div id="overlay" role="dialog" aria-label="Sign in">
        <p>Please sign in</p>
        <button id="close">Close</button>
      </div>
    </body></html>`

    withFixtureDom(html, ({ byId, setRect, setStyle, setHits }) => {
      const { world, output } = makeWorld(stubbedBundle, 0, 'stubbed-survey')
      forbidDirectRefs(world)

      const behind = byId('behind')
      const overlay = byId('overlay')
      const close = byId('close')
      setRect(behind, makeRect(100, 100, 100, 40))
      setRect(overlay, makeRect(0, 0, 1280, 800))
      setRect(close, makeRect(200, 200, 80, 40))
      setStyle(overlay, { position: 'fixed' })
      setHits((x, y) => {
        if (x >= 200 && x <= 280 && y >= 200 && y <= 240)
          return [close, overlay]
        return [overlay]
      })

      const behindNode: AriaNodeJSON = {
        role: 'button',
        name: 'Pay',
        ref: 'r-behind',
      }
      const overlayNode: AriaNodeJSON = {
        role: 'dialog',
        name: 'Sign in',
        ref: 'r-overlay',
        children: [{ role: 'button', name: 'Close', ref: 'r-close' }],
      }
      const json: AriaSnapshotJSON = [behindNode, overlayNode]

      const aria = makeCustomAria({
        names: new Map([
          [overlay, 'Sign in'],
          [close, 'Close'],
        ]),
        refs: new Map([
          [overlay, 'r-overlay'],
          [close, 'r-close'],
        ]),
        snapshots: new Map([
          [
            overlay,
            { json: [{ role: 'dialog', name: 'Sign in' }], hasWords: true },
          ],
        ]),
        surveyElements: () => [behind, close],
        snapshotNodes: () =>
          new Map<AriaNodeJSON, Element>([
            [behindNode, behind],
            [overlayNode, overlay],
          ]),
      })

      const result = stubbedBundle.coverSurvey.surveyCovers(
        world,
        output,
        json,
        aria,
      )
      expect(result.marksSkipped).toBe(false)
      expect(result.layers).toEqual([
        {
          role: 'dialog',
          name: 'Sign in',
          ref: 'r-overlay',
          blank: false,
          loading: false,
          wholeView: true,
          controls: [{ role: 'button', name: 'Close', ref: 'r-close' }],
          firstWords: 'Please sign in Close',
        },
      ])
      expect(overlayNode.onTop).toBe('r-overlay')
      expect(behindNode.onTop).toBeUndefined()

      const failingLinkAria: SnapshotAriaMod.SnapshotAria = {
        ...aria,
        snapshotNodes: () => {
          throw new Error('link mismatch')
        },
      }
      const json2: AriaSnapshotJSON = [
        { ...behindNode },
        { ...overlayNode, onTop: undefined },
      ]
      const skipped = stubbedBundle.coverSurvey.surveyCovers(
        world,
        output,
        json2,
        failingLinkAria,
      )
      expect(skipped.marksSkipped).toBe(true)
      expect(skipped.layers).toHaveLength(1)
    })
  })

  it('runs private-look.ts drawingOf and lookAt through CoverAria without engine name calls', () => {
    const html = `<html><body>
      <div id="field-wrap">
        <input id="code" type="text">
        <span id="floating">Verification code</span>
      </div>
    </body></html>`

    withFixtureDom(html, ({ byId, setRect, setHits }) => {
      const { world } = makeWorld(stubbedBundle, 0, 'stubbed-look')
      const wrap = byId('field-wrap')
      const input = byId('code')
      const floating = byId('floating')
      setRect(input, makeRect(100, 100, 200, 40))
      setRect(floating, makeRect(110, 105, 120, 25))
      setHits(() => [floating])

      const matchingAria = { nameOf: () => 'Verification code' }
      expect(
        stubbedBundle.privateLook.drawingOf(
          world,
          input,
          floating,
          false,
          matchingAria,
        ),
      ).toBe(wrap)

      const allowed = stubbedBundle.privateLook.lookAt(
        world,
        input,
        { cut: false, keys: false, frameVisible: true, scroll: 'asItIs' },
        matchingAria,
      )
      expect(allowed.refusal).toBeUndefined()
      expect(stubbedBundle.privateLook.coverWrapper(input)).toBe(wrap)

      const mismatchedAria = { nameOf: () => 'Completely different label' }
      const refused = stubbedBundle.privateLook.lookAt(
        world,
        input,
        { cut: false, keys: false, frameVisible: true, scroll: 'asItIs' },
        mismatchedAria,
      )
      expect(refused.refusal).toEqual({ code: 'boxCovered' })
      expect(stubbedBundle.privateLook.coverWrapper(input)).toBeUndefined()
    })
  })

  it('runs snapshot-inert.ts markInert purely through CoverAria.snapshotRefs', () => {
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
      const bg = byId('bg')
      const bgBtn = byId('bg-btn')
      const live = byId('live')
      const liveBtn = byId('live-btn')

      const byRef = new Map<string, Element>([
        ['e1', bg],
        ['e2', bgBtn],
        ['e3', live],
        ['e4', liveBtn],
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
