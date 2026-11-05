/**
 * Runs the built page asset without a browser: a linkedom document installed as this
 * process's DOM globals, a stand-in for core's seal, and assets/injected.js evaluated the way
 * the install expression does. linkedom has no layout, so these tests cover routing, the
 * replies' shapes and the probe's own state; scripts/probe-smoke.mjs covers a real browser,
 * core's real seal and a strict CSP.
 */
import { readFileSync } from 'node:fs'
import { parseHTML } from 'linkedom'
import { CALL_FUNCTION } from '../../src/probe/protocol'

const ASSET = readFileSync(
  new URL(
    '../../../../crates/browseros-playwright/assets/injected.js',
    import.meta.url,
  ),
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
  /** What Rust's CALL_FUNCTION does: the probe's reply, or `uninstalled`. */
  call(request: unknown, root?: Node): unknown
  /** `__neoProbe.boot`, as the install expression calls it. */
  boot(options: unknown): unknown
  close(): void
}

/** `kind` for `type=password` inputs only, like the seal for the fixtures used here. */
const SEAL_STAND_IN = {
  version: 1,
  kind: (element: Element) =>
    element.getAttribute('type') === 'password' ? 'password' : null,
  valueEquals: () => false,
  rawLength: () => 0,
  sensitiveValues: () => ['hunter2'],
  watch: () => undefined,
}

/**
 * Installs `html` as the global DOM, with the seal stand-in unless `sealed` is false, and
 * evaluates the asset once. `close` restores every global it replaced.
 */
export function openProbePage(
  html: string,
  { sealed = true }: { sealed?: boolean } = {},
): ProbePage {
  const { window, document } = parseHTML(html)
  // linkedom computes no styles. The engine reads a missing style as "rendered and visible"
  // (domUtils computeBox), which is all these tests ask of it.
  Reflect.set(window, 'getComputedStyle', () => undefined)
  // No layout means no boxes: every element reads as one without a layout box.
  Reflect.set(window.Element.prototype, 'getClientRects', () => [])
  const saved = new Map<string, unknown>()
  const replace = (name: string, value: unknown) => {
    saved.set(name, Reflect.get(globalThis, name))
    Reflect.set(globalThis, name, value)
  }
  for (const name of DOM_GLOBALS) replace(name, Reflect.get(window, name))
  replace('location', { href: 'https://fixture.test/page' })
  replace('__clawSeal', sealed ? SEAL_STAND_IN : undefined)
  replace('__neoProbe', undefined)
  new Function(ASSET)()
  const callFunction = new Function(`return (${CALL_FUNCTION})`)()
  const probe = globalThis.__neoProbe
  if (!probe) throw new Error('the asset did not define __neoProbe')
  return {
    document,
    call: (request, root) => callFunction.call(globalThis, request, root),
    // Through Reflect.apply, so tests can hand boot options Rust never would.
    boot: (options) => Reflect.apply(probe.boot, probe, [options]),
    close() {
      for (const [name, value] of saved) Reflect.set(globalThis, name, value)
    },
  }
}
