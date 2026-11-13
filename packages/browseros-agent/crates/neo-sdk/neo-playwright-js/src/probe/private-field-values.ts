/**
 * Projects page-declared private fields during a synchronous tree read in this frame's
 * isolated world. Snapshots and text/name comparisons share this scope; native stored values,
 * field-value comparisons and deliberate reads remain clear outside it.
 */
import { anyPrivatelyTyped, privatelyTyped } from './private-entry'

const PRIVATE_AUTOCOMPLETE = new Set([
  'current-password',
  'new-password',
  'one-time-code',
  'cc-number',
  'cc-csc',
])

type Field = HTMLInputElement | HTMLTextAreaElement

const HTML_NAMESPACE = 'http://www.w3.org/1999/xhtml'
const FIELD_TAGS: ReadonlySet<string> = new Set(['input', 'textarea'])

/** The resolved walk root and Rust's conservative other-frame hint. */
type FieldDiscovery = {
  readonly root: Element | null
  readonly mayHaveOtherFrame: boolean
}

/** Actual wrapper prototypes for this walk only. A textarea and its Text can come from different realms. */
type FieldPrototypes = {
  readonly inputs: Set<object>
  readonly textareas: Set<object>
  readonly texts: Set<object>
}

/** One reversible own-property shadow, including an inherited property's original absence. */
type PropertyOverride = {
  target: object
  property: string
  original: PropertyDescriptor | undefined
  masked: PropertyDescriptor
}

export const PRIVATE_VALUE_ATTRIBUTES: ReadonlySet<string> = new Set([
  'value',
  'aria-valuetext',
  'aria-valuenow',
])

export function isHtmlField(element: Node): element is Field {
  return (
    element.nodeType === 1 &&
    'localName' in element &&
    typeof element.localName === 'string' &&
    FIELD_TAGS.has(element.localName) &&
    'namespaceURI' in element &&
    element.namespaceURI === HTML_NAMESPACE
  )
}

/** Declaration masking is a synchronous scope; a host-typing mark has no bypass. */
let declaredMasked: boolean | undefined

function isHtmlTextarea(node: Node | null): node is HTMLTextAreaElement {
  return (
    node?.nodeType === 1 &&
    'localName' in node &&
    node.localName === 'textarea' &&
    'namespaceURI' in node &&
    node.namespaceURI === HTML_NAMESPACE
  )
}

/** SOP document access also recognizes inherited about:blank origins without comparing URL strings. */
function hasReadableOpener(document: Document): boolean {
  try {
    const view = document.defaultView
    if (!view) return true
    const opener: unknown = view.opener
    if (opener === null || opener === undefined) return false
    if (typeof opener !== 'object' && typeof opener !== 'function') return true
    // Access is the test: SecurityError, rather than the document's contents, decides it.
    const _openerDocument: unknown = Reflect.get(opener, 'document')
    return true
  } catch (error: unknown) {
    // A denied opener is not a source realm. Other failures cannot establish its absence.
    return !(
      typeof error === 'object' &&
      error !== null &&
      'name' in error &&
      error.name === 'SecurityError'
    )
  }
}

/**
 * Whole-document discovery covers ARIA names and ownership outside a scoped walk root. Query
 * each exposed shadow root through the same getter the engine uses, without entering frames.
 * The hint avoids a document scan on ordinary single-frame snapshots. That skip can miss a
 * retained source prototype after its frame disappears and no neighbour or opener remains.
 * Local guards and an actual field root always apply.
 */
function collectFieldPrototypes({
  root,
  mayHaveOtherFrame,
}: FieldDiscovery): FieldPrototypes {
  const prototypes: FieldPrototypes = {
    inputs: new Set([HTMLInputElement.prototype]),
    textareas: new Set([HTMLTextAreaElement.prototype]),
    texts: new Set([Text.prototype]),
  }
  const collect = (element: Element) => {
    if (!isHtmlField(element)) return
    const prototype: object | null = Object.getPrototypeOf(element)
    if (!prototype)
      throw new TypeError('snapshot field prototype is unavailable')
    if (element.localName === 'input') {
      prototypes.inputs.add(prototype)
      return
    }
    prototypes.textareas.add(prototype)
    for (const child of element.childNodes) {
      if (child.nodeType !== 3) continue
      const textPrototype: object | null = Object.getPrototypeOf(child)
      if (!textPrototype)
        throw new TypeError('snapshot textarea Text prototype is unavailable')
      prototypes.texts.add(textPrototype)
    }
  }
  // A page may remove its document element while Neo still holds a marked field. Its
  // Document and local prototypes survive, so a mark-only diagnostic must still be guarded.
  const ownerDocument = root?.ownerDocument ?? document
  if (root) collect(root)
  if (!mayHaveOtherFrame && !hasReadableOpener(ownerDocument)) return prototypes
  const roots: (Document | ShadowRoot)[] = [ownerDocument]
  for (const tree of roots) {
    for (const element of tree.querySelectorAll('input,textarea'))
      collect(element)
    for (const host of tree.querySelectorAll('*')) {
      const shadow = host.shadowRoot
      if (shadow) roots.push(shadow)
    }
  }
  return prototypes
}

/** One privacy decision for tree reads and diagnostics, including fields adopted from another realm. */
export function isPrivateField(
  field: Node,
  { declared = declaredMasked ?? true }: { declared?: boolean } = {},
): field is Field {
  return (
    isHtmlField(field) &&
    (privatelyTyped(field) ||
      ((declaredMasked === true || declared) &&
        (field.getAttribute('type')?.toLowerCase() === 'password' ||
          (field.getAttribute('autocomplete') ?? '')
            .toLowerCase()
            .split(/\s+/)
            .some((token) => PRIVATE_AUTOCOMPLETE.has(token)))))
  )
}

function maskedValue(
  descriptor: PropertyDescriptor,
  valueReads: Set<Field>,
): PropertyDescriptor {
  if (typeof descriptor.get !== 'function')
    throw new TypeError('snapshot field value accessor is unavailable')
  const get = descriptor.get
  return {
    ...descriptor,
    get(this: Field) {
      // Some DOMs implement .value through textContent. Only that native getter may read the
      // stored textarea text; direct text reads by the snapshot walker still return empty.
      const alreadyReading = valueReads.has(this)
      valueReads.add(this)
      let value: unknown
      try {
        value = get.call(this)
      } finally {
        if (!alreadyReading) valueReads.delete(this)
      }
      if (typeof value !== 'string')
        throw new TypeError('snapshot field value accessor did not return text')
      return isPrivateField(this) ? '•'.repeat(value.length) : value
    },
  }
}

/**
 * Resolves the accessor or method as the DOM would, before installing an own-property shadow.
 * Native Text.nodeValue and getAttribute are inherited; their shadows must be removed later.
 */
function propertyOverride(
  target: object,
  property: string,
  mask: (descriptor: PropertyDescriptor) => PropertyDescriptor,
): PropertyOverride {
  const original = Object.getOwnPropertyDescriptor(target, property)
  if (
    original?.configurable === false ||
    (!original && !Object.isExtensible(target))
  )
    throw new TypeError(`snapshot field ${property} cannot be overridden`)
  let owner: object | null = target
  while (owner) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, property)
    if (descriptor) {
      // An inherited non-configurable property can be shadowed, but our temporary own
      // property must be configurable so cleanup can remove it.
      return {
        target,
        property,
        original,
        masked: { ...mask(descriptor), configurable: true },
      }
    }
    owner = Object.getPrototypeOf(owner)
  }
  throw new TypeError(`snapshot field ${property} is unavailable`)
}

function maskedAttributes(descriptor: PropertyDescriptor): PropertyDescriptor {
  if (typeof descriptor.value !== 'function')
    throw new TypeError('snapshot field getAttribute method is unavailable')
  const getAttribute = descriptor.value
  return {
    ...descriptor,
    value(this: Field, name: string) {
      const value: unknown = getAttribute.call(this, name)
      return typeof value === 'string' &&
        typeof name === 'string' &&
        PRIVATE_VALUE_ATTRIBUTES.has(name.toLowerCase()) &&
        isPrivateField(this)
        ? '•'.repeat(value.length)
        : value
    },
  }
}

function emptyTextareaText(
  descriptor: PropertyDescriptor,
  textareaOf: (node: Node) => HTMLTextAreaElement | null,
  valueReads: Set<Field>,
): PropertyDescriptor {
  if (typeof descriptor.get !== 'function')
    throw new TypeError('snapshot textarea text accessor is unavailable')
  const get = descriptor.get
  return {
    ...descriptor,
    get(this: Node) {
      const textarea = textareaOf(this)
      return textarea && !valueReads.has(textarea) && isPrivateField(textarea)
        ? ''
        : get.call(this)
    },
  }
}

function parentTextarea(node: Node): HTMLTextAreaElement | null {
  const parent = node.parentElement
  return isHtmlTextarea(parent) ? parent : null
}

/**
 * Chromium resolves CSS attr(value) without calling getAttribute, including alternative text
 * on a control's own content. Suppress content on private controls and their pseudo-elements
 * in this world, while retaining the browser's style for every other read.
 */
function maskedComputedStyle(
  descriptor: PropertyDescriptor,
): PropertyDescriptor {
  if (typeof descriptor.value !== 'function')
    throw new TypeError('snapshot getComputedStyle method is unavailable')
  const read = descriptor.value
  return {
    ...descriptor,
    value(this: unknown, element: Element, pseudo?: string | null) {
      const style: CSSStyleDeclaration = Reflect.apply(read, this, [
        element,
        pseudo,
      ])
      // Every element reaches this hot path. Reject other tags and namespaces before reading
      // privacy attributes, so ordinary elements retain the native style object.
      if (!isHtmlField(element)) return style
      if (!isPrivateField(element)) return style
      return new Proxy(style, {
        get(target, key) {
          if (key === 'content') return 'none'
          if (key === 'getPropertyValue')
            return (property: string) =>
              property.toLowerCase() === 'content'
                ? 'none'
                : target.getPropertyValue(property)
          const value: unknown = Reflect.get(target, key)
          return typeof value === 'function' ? value.bind(target) : value
        },
      })
    },
  }
}

/** A cleanup error must refuse the result, but cannot prevent the remaining restorations. */
function restoreOverrides(installed: PropertyOverride[]): void {
  const failures: unknown[] = []
  for (const override of installed.reverse()) {
    try {
      if (override.original) {
        Object.defineProperty(
          override.target,
          override.property,
          override.original,
        )
      } else if (!Reflect.deleteProperty(override.target, override.property)) {
        throw new TypeError(
          `snapshot field ${override.property} could not be removed`,
        )
      }
    } catch (error) {
      failures.push(error)
    }
  }
  if (failures.length)
    throw new TypeError('snapshot field overrides could not be restored', {
      cause: new AggregateError(failures),
    })
}

/**
 * Mark-only reads install no guards until this world has privately typed a box (R44/R47), or
 * until `root` is itself a marked box: one marked in another same-origin frame's copy of the
 * probe and moved here, which this world's flag never saw (R53). `root`'s own prototype, made in
 * the frame it came from, is guarded with this document's.
 */
export function withMarkedFieldsMasked<T>(
  walk: () => T,
  root: Element | null = document.documentElement,
): T {
  return anyPrivatelyTyped() || (root !== null && privatelyTyped(root))
    ? withMaskedFieldValues({ root, mayHaveOtherFrame: true }, walk, {
        declared: false,
      })
    : walk()
}

/**
 * Owns every field read the engine uses until `walk` returns or throws. The entire marking
 * retry stays inside this scope, so its fallback cannot create a clear-value tree. Validate
 * every patch before installing any; cleanup attempts all installed patches in reverse order.
 * The style guard shares this registry so it restores the function it found, including a
 * pointer-target wrapper when the two scopes are nested in either order.
 */
export function withMaskedFieldValues<T>(
  discovery: FieldDiscovery,
  walk: () => T,
  { declared = true }: { declared?: boolean } = {},
): T {
  const prototypes = collectFieldPrototypes(discovery)
  const inputs = Array.from(prototypes.inputs)
  const textareas = Array.from(prototypes.textareas)
  const texts = Array.from(prototypes.texts)
  const valueReads = new Set<Field>()
  const overrides = [
    ...inputs.map((input) =>
      propertyOverride(input, 'value', (descriptor) =>
        maskedValue(descriptor, valueReads),
      ),
    ),
    ...textareas.map((textarea) =>
      propertyOverride(textarea, 'value', (descriptor) =>
        maskedValue(descriptor, valueReads),
      ),
    ),
    ...inputs.map((input) =>
      propertyOverride(input, 'defaultValue', (descriptor) =>
        maskedValue(descriptor, valueReads),
      ),
    ),
    // Element.getAttribute is a hot path for every node. Shadow only the two control types.
    ...inputs.map((input) =>
      propertyOverride(input, 'getAttribute', maskedAttributes),
    ),
    ...textareas.map((textarea) =>
      propertyOverride(textarea, 'getAttribute', maskedAttributes),
    ),
    ...texts.flatMap((text) =>
      ['nodeValue', 'textContent', 'data', 'wholeText'].map((property) =>
        propertyOverride(text, property, (descriptor) =>
          emptyTextareaText(descriptor, parentTextarea, valueReads),
        ),
      ),
    ),
    ...textareas.flatMap((textarea) =>
      ['textContent', 'innerText', 'innerHTML', 'defaultValue'].map(
        (property) =>
          propertyOverride(textarea, property, (descriptor) =>
            emptyTextareaText(
              descriptor,
              (node) => (isHtmlTextarea(node) ? node : null),
              valueReads,
            ),
          ),
      ),
    ),
    propertyOverride(globalThis, 'getComputedStyle', maskedComputedStyle),
  ]
  const installed: PropertyOverride[] = []
  const outer = declaredMasked
  try {
    for (const override of overrides) {
      Object.defineProperty(override.target, override.property, override.masked)
      installed.push(override)
    }
    // Nested scopes may strengthen privacy, never weaken a full outer walk into mark-only.
    // Undefined means no active scope, so a standalone conformance scope can remain mark-only.
    declaredMasked = declared || outer === true
    return walk()
  } finally {
    declaredMasked = outer
    restoreOverrides(installed)
  }
}
