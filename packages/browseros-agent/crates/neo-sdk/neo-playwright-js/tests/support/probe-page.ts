/**
 * Runs the built page asset without a browser: a linkedom document installed as this
 * process's DOM globals, and assets/injected.js evaluated the way the install expression does.
 * linkedom has no layout, so these tests cover routing, the replies' shapes and the probe's own
 * state; scripts/probe-smoke.mjs covers a real browser and a strict CSP.
 */
import { readFileSync } from 'node:fs'
import { parseHTML } from 'linkedom'
import { CALL_FUNCTION, type ProbePolicy } from '../../src/probe/protocol'

const ASSET = readFileSync(
  new URL('../../../neo-playwright/assets/injected.js', import.meta.url),
  'utf8',
)

/** The DOM names the engine and the probe read as globals. */
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
  'getComputedStyle',
] as const

export type ProbePage = {
  document: Document
  /**
   * What Rust's CALL_FUNCTION does: the probe's reply, or `uninstalled`. `nodes` follow the
   * root, as Rust's call_with_nodes hands them.
   */
  call(request: unknown, root?: Node | null, ...nodes: unknown[]): unknown
  /** `__neoProbe.boot`, as the install expression calls it. */
  boot(options: unknown): unknown
  close(): void
}

/**
 * Installs `html` as the global DOM and evaluates the asset once. `close` restores every
 * global it replaced.
 */
export function openProbePage(
  html: string,
  policy: ProbePolicy = { conformance: false, mayHaveOtherFrame: true },
): ProbePage {
  const { window, document } = parseHTML(html)
  // linkedom omits the native defaultValue accessors. Supply their markup reads only in this
  // harness so the production guard can require every native API instead of silently skipping
  // a missing privacy boundary. Browser live tests cover independent current/default values.
  const defaultValues: [object, PropertyDescriptor][] = [
    [
      window.HTMLInputElement.prototype,
      {
        configurable: true,
        enumerable: true,
        get(this: HTMLInputElement) {
          return this.getAttribute('value') ?? ''
        },
        set(this: HTMLInputElement, value: string) {
          this.setAttribute('value', value)
        },
      },
    ],
    [
      window.HTMLTextAreaElement.prototype,
      {
        configurable: true,
        enumerable: true,
        get(this: HTMLTextAreaElement) {
          return this.textContent ?? ''
        },
        set(this: HTMLTextAreaElement, value: string) {
          this.textContent = value
        },
      },
    ],
  ]
  const suppliedDefaults: object[] = []
  for (const [prototype, descriptor] of defaultValues) {
    if ('defaultValue' in prototype) continue
    Object.defineProperty(prototype, 'defaultValue', descriptor)
    suppliedDefaults.push(prototype)
  }
  // linkedom computes no styles. The privacy scope now proxies a control's normal style as
  // well as its pseudos, so those reads need objects as in a browser. Leave unrelated normal
  // reads absent, preserving the fixture's existing no-layout behavior.
  Reflect.set(
    window,
    'getComputedStyle',
    (element: Element, pseudo?: string) => {
      if (
        !pseudo &&
        element.localName !== 'input' &&
        element.localName !== 'textarea'
      )
        return undefined
      return {
        display: 'block',
        visibility: 'visible',
        content: 'none',
        getPropertyValue: () => '',
      }
    },
  )
  // computeBox treats a missing style as visible and non-inline. Controls now have styles,
  // so supply only their visibility and box reads to retain that same synthetic result.
  for (const prototype of [
    window.HTMLInputElement.prototype,
    window.HTMLTextAreaElement.prototype,
  ]) {
    Reflect.set(prototype, 'checkVisibility', () => true)
    Reflect.set(prototype, 'getBoundingClientRect', () => ({
      x: 0,
      y: 0,
      width: 10,
      height: 10,
    }))
  }
  // There are no real layout fragments; the synthetic control boxes above serve computeBox.
  Reflect.set(window.Element.prototype, 'getClientRects', () => [])
  const saved = new Map<string, unknown>()
  const replace = (name: string, value: unknown) => {
    saved.set(name, Reflect.get(globalThis, name))
    Reflect.set(globalThis, name, value)
  }
  for (const name of DOM_GLOBALS) replace(name, Reflect.get(window, name))
  replace('location', { href: 'https://fixture.test/page' })
  replace('__neoProbe', undefined)
  new Function(ASSET)()
  const callFunction = new Function(`return (${CALL_FUNCTION})`)()
  const probe = globalThis.__neoProbe
  if (!probe) throw new Error('the asset did not define __neoProbe')
  return {
    document,
    call: (request, root, ...nodes) =>
      callFunction.call(globalThis, { request, policy }, root, ...nodes),
    // Through Reflect.apply, so tests can hand boot options Rust never would.
    boot: (options) => Reflect.apply(probe.boot, probe, [options]),
    close() {
      for (const prototype of suppliedDefaults)
        Reflect.deleteProperty(prototype, 'defaultValue')
      for (const [name, value] of saved) Reflect.set(globalThis, name, value)
    },
  }
}
