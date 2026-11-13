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

/** One reversible own-property shadow, including an inherited property's original absence. */
type PrototypeOverride = {
  prototype: object
  property: string
  original: PropertyDescriptor | undefined
  masked: PropertyDescriptor
}

const PRIVATE_VALUE_ATTRIBUTES = new Set([
  'value',
  'aria-valuetext',
  'aria-valuenow',
])

function isPrivateField(field: Field): boolean {
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
function prototypeOverride(
  prototype: object,
  property: string,
  mask: (descriptor: PropertyDescriptor) => PropertyDescriptor,
): PrototypeOverride {
  const original = Object.getOwnPropertyDescriptor(prototype, property)
  if (
    original?.configurable === false ||
    (!original && !Object.isExtensible(prototype))
  )
    throw new TypeError(`snapshot field ${property} cannot be overridden`)
  let owner: object | null = prototype
  while (owner) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, property)
    if (descriptor) {
      // An inherited non-configurable property can be shadowed, but our temporary own
      // property must be configurable so cleanup can remove it.
      return {
        prototype,
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

/** A cleanup error must refuse the result, but cannot prevent the remaining restorations. */
function restoreOverrides(installed: PrototypeOverride[]): void {
  const failures: unknown[] = []
  for (const override of installed.reverse()) {
    try {
      if (override.original) {
        Object.defineProperty(
          override.prototype,
          override.property,
          override.original,
        )
      } else if (
        !Reflect.deleteProperty(override.prototype, override.property)
      ) {
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
 */
export function withMaskedFieldValues<T>(walk: () => T): T {
  const input = HTMLInputElement.prototype
  const textarea = HTMLTextAreaElement.prototype
  const valueReads = new Set<Field>()
  const overrides = [
    prototypeOverride(input, 'value', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    prototypeOverride(textarea, 'value', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    prototypeOverride(input, 'defaultValue', (descriptor) =>
      maskedValue(descriptor, valueReads),
    ),
    // Element.getAttribute is a hot path for every node. Shadow only the two control types.
    prototypeOverride(input, 'getAttribute', maskedAttributes),
    prototypeOverride(textarea, 'getAttribute', maskedAttributes),
    ...['nodeValue', 'textContent', 'data', 'wholeText'].map((property) =>
      prototypeOverride(Text.prototype, property, (descriptor) =>
        emptyTextareaText(descriptor, parentTextarea, valueReads),
      ),
    ),
    ...['textContent', 'innerText', 'innerHTML', 'defaultValue'].map(
      (property) =>
        prototypeOverride(textarea, property, (descriptor) =>
          emptyTextareaText(
            descriptor,
            (node) => (node instanceof HTMLTextAreaElement ? node : null),
            valueReads,
          ),
        ),
    ),
  ]
  const installed: PrototypeOverride[] = []
  try {
    for (const override of overrides) {
      Object.defineProperty(
        override.prototype,
        override.property,
        override.masked,
      )
      installed.push(override)
    }
    return walk()
  } finally {
    restoreOverrides(installed)
  }
}
