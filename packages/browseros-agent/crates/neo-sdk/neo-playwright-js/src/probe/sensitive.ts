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
  attributeEquals(element: Element, name: string, expected: string): boolean
  rawLength(element: Element): number
  sensitiveValues(): string[]
  watch(root: Node): void
}

const SEAL_METHODS = [
  'kind',
  'valueEquals',
  'attributeEquals',
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

/**
 * The seal's kind of the field an edit of `element` changes: the element, or the control its
 * label names (upstream's `retarget(element, 'follow-label')`, which fill, selectOptions,
 * setInputFiles and the `editable` state go through). Keys pressed on a label land there too,
 * because Chromium hands a label's focus to its control. Rust marks the op's target sensitive
 * on this answer before it logs or sends the value, so classifying the label itself would
 * print a password typed through it into the call log and the trace.
 */
export function editedKind(
  world: World,
  element: Element,
): SensitiveKind | null {
  return world.seal.kind(
    world.injected.retarget(element, 'follow-label') ?? element,
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
 * into the child frame, so it is pinned for Rust to hop into and ask there. Any other element
 * with no open shadow root is pinned as a `host`: a closed root shows only its host here, so
 * the control that has the focus may sit inside one, and only CDP can tell (Rust asks
 * `DOM.describeNode`, which lists closed roots).
 */
export function focused(world: World): Focused {
  const element = deepActiveElement(world.injected.document)
  if (!element)
    return { kind: 'focused', sensitive: false, iframe: null, host: null }
  const sensitive = world.seal.kind(element) !== null
  const iframe =
    frameSource(element) === undefined ? null : world.pins.add(element)
  const unseen = !sensitive && iframe === null && !element.shadowRoot
  return {
    kind: 'focused',
    sensitive,
    iframe,
    host: unseen ? world.pins.add(element) : null,
  }
}

/**
 * The focused element, followed through open shadow roots. A closed root shows only its
 * host here (`focused` pins that host for Rust); the seal follows the closed roots Rust
 * handed to `watch`, but does not expose that walk, and sdk-playwright hands it none.
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
