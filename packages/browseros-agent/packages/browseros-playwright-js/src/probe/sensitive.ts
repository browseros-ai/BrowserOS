/**
 * The probe's side of sdk-privacy's seal. Core's seal.js runs in this world before the engine
 * and replaces every getter a field value can be read through, so the engine reads `••••` for
 * password, card and one-time-code fields. Its `__clawSeal` object keeps the native getters
 * for jobs that never hand a value to the agent; the `sweep` and `focused` requests use it.
 */
import type { Focused, SensitiveKind, Sweep } from './protocol'
import { frameSource, type World } from './world'

/** `__clawSeal`, as crates/browseros-core/src/sensitive_fields/seal.js defines it. */
export type ClawSeal = {
  version: number
  kind(element: Element): SensitiveKind | null
  valueEquals(element: Element, expected: string): boolean
  rawLength(element: Element): number
  sensitiveValues(): string[]
  watch(root: Node): void
}

const SEAL_METHODS = [
  'kind',
  'valueEquals',
  'rawLength',
  'sensitiveValues',
  'watch',
] as const

/** The seal installed in `scope`, or undefined when there is none (an unsealed world). */
export function sealIn(scope: object): ClawSeal | undefined {
  const seal: unknown = Reflect.get(scope, '__clawSeal')
  return isClawSeal(seal) ? seal : undefined
}

function isClawSeal(value: unknown): value is ClawSeal {
  if (typeof value !== 'object' || value === null) return false
  return (
    typeof Reflect.get(value, 'version') === 'number' &&
    SEAL_METHODS.every((name) => typeof Reflect.get(value, name) === 'function')
  )
}

/** Raw values of this document's sensitive fields. Rust masks them out of page echoes. */
export function sweep(world: World): Sweep {
  return { kind: 'sweep', values: world.seal.sensitiveValues() }
}

/** Open shadow roots nest; this bounds the walk on a pathological page. */
const MAX_SHADOW_DEPTH = 32

/**
 * What keyboard input would reach in this document. A focused `<iframe>` means the keys go
 * into the child frame, so it is pinned for Rust to hop into and ask there.
 */
export function focused(world: World): Focused {
  const element = deepActiveElement(world.injected.document)
  if (!element) return { kind: 'focused', sensitive: false, iframe: null }
  return {
    kind: 'focused',
    sensitive: world.seal.kind(element) !== null,
    iframe: frameSource(element) === undefined ? null : world.pins.add(element),
  }
}

/**
 * The focused element, followed through open shadow roots. A closed root shows only its
 * host here; the seal follows the closed roots Rust handed to `watch`, but does not expose
 * that walk.
 */
function deepActiveElement(document: Document): Element | null {
  let active = document.activeElement
  for (let depth = 0; depth < MAX_SHADOW_DEPTH; depth++) {
    const inner = active?.shadowRoot?.activeElement
    if (!inner) break
    active = inner
  }
  return active
}
