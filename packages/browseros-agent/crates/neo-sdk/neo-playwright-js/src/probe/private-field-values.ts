/**
 * Masks page-declared private values during one synchronous snapshot walk in this frame's
 * isolated world. The DOM's stored values and the page world's native getters stay untouched.
 */
const PRIVATE_AUTOCOMPLETE = new Set([
  'current-password',
  'new-password',
  'one-time-code',
  'cc-number',
  'cc-csc',
])

type Field = HTMLInputElement | HTMLTextAreaElement

const FIELD_TAGS: ReadonlySet<string> = new Set(['input', 'textarea'])

/** One reversible own-property shadow, including an inherited property's original absence. */
type PropertyOverride = {
  target: object
  property: string
  original: PropertyDescriptor | undefined
  masked: PropertyDescriptor
}

const PRIVATE_VALUE_ATTRIBUTES = new Set([
  'value',
  'aria-valuetext',
  'aria-valuenow',
])

function isPrivateField(field: Element): boolean {
  return (
    field.getAttribute('type')?.toLowerCase() === 'password' ||
    (field.getAttribute('autocomplete') ?? '')
      .toLowerCase()
      .split(/\s+/)
      .some((token) => PRIVATE_AUTOCOMPLETE.has(token))
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
  return node.parentElement instanceof HTMLTextAreaElement
    ? node.parentElement
    : null
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
      // Every element reaches this hot path. Test its tag before reading privacy attributes,
      // so ordinary elements retain the native object after one set membership check.
      if (!FIELD_TAGS.has(element.localName)) return style
      if (!isPrivateField(element)) return style
      return new Proxy(style, {
        get(target, key) {
          if (key === 'content') return 'none'
          if (key === 'getPropertyValue')
            return (property: string) =>
              property === 'content'
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
 * Owns every field read the engine uses until `walk` returns or throws. The entire marking
 * retry stays inside this scope, so its fallback cannot create a clear-value tree. Validate
 * every patch before installing any; cleanup attempts all installed patches in reverse order.
 * The style guard shares this registry so it restores the function it found, including a
 * pointer-target wrapper when the two scopes are nested in either order.
 */
export function withMaskedFieldValues<T>(walk: () => T): T {
  const input = HTMLInputElement.prototype
  const textarea = HTMLTextAreaElement.prototype
  const valueReads = new Set<Field>()
  const overrides = [
    propertyOverride(input, 'value', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    propertyOverride(textarea, 'value', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    propertyOverride(input, 'defaultValue', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    // Element.getAttribute is a hot path for every node. Shadow only the two control types.
    propertyOverride(input, 'getAttribute', maskedAttributes),
    propertyOverride(textarea, 'getAttribute', maskedAttributes),
    ...['nodeValue', 'textContent', 'data', 'wholeText'].map((property) =>
      propertyOverride(Text.prototype, property, (descriptor) =>
        emptyTextareaText(descriptor, parentTextarea, valueReads),
      ),
    ),
    ...['textContent', 'innerText', 'innerHTML', 'defaultValue'].map(
      (property) =>
        propertyOverride(textarea, property, (descriptor) =>
          emptyTextareaText(
            descriptor,
            (node) => (node instanceof HTMLTextAreaElement ? node : null),
            valueReads,
          ),
        ),
    ),
    propertyOverride(globalThis, 'getComputedStyle', maskedComputedStyle),
  ]
  const installed: PropertyOverride[] = []
  try {
    for (const override of overrides) {
      Object.defineProperty(override.target, override.property, override.masked)
      installed.push(override)
    }
    return walk()
  } finally {
    restoreOverrides(installed)
  }
}
