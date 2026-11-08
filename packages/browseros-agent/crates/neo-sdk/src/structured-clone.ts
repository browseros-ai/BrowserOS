/**
 * Neo: the realm's `structuredClone`, which QuickJS lacks and a Node Playwright script has as a
 * global. `realm.rs` installs it (`globals.ts`).
 *
 * It clones what the HTML structured clone algorithm clones out of plain JavaScript: primitives,
 * their wrapper objects, `Date`, `RegExp`, `ArrayBuffer`, typed arrays and `DataView`, `Map`,
 * `Set`, `Error`s, arrays, and every other object as a plain object of its own enumerable string
 * keys (its prototype is not kept, as in Node). Shared and cyclic references stay shared and
 * cyclic. A function, a symbol, a `Promise`, a `WeakMap`/`WeakSet`/`WeakRef`, a `URL` or
 * `URLSearchParams` throws a
 * `DOMException` named `DataCloneError`, as in Node; the `transfer` option is refused the same
 * way, since nothing here can detach a buffer.
 */

import { URL, URLSearchParams } from './url'

type TypedArrayKind = {
  new (buffer: ArrayBuffer, byteOffset: number, length: number): ArrayBufferView
  BYTES_PER_ELEMENT: number
}

/** Platform: the error classes V8's serializer keeps by name; any other `Error` clones as an
 * `Error` (V8 `ValueSerializer::WriteJSError`). */
const ERRORS: Record<string, ErrorConstructor> = {
  Error,
  EvalError,
  RangeError,
  ReferenceError,
  SyntaxError,
  TypeError,
  URIError,
}

/** Platform: ECMAScript's built-in typed array classes, which a clone keeps. */
const TYPED_ARRAYS: TypedArrayKind[] = [
  Int8Array,
  Uint8Array,
  Uint8ClampedArray,
  Int16Array,
  Uint16Array,
  Int32Array,
  Uint32Array,
  Float32Array,
  Float64Array,
  BigInt64Array,
  BigUint64Array,
]

export function structuredClone(
  ...args: [value?: unknown, options?: { transfer?: unknown[] }]
): unknown {
  if (args.length === 0)
    throw new TypeError('The "value" argument must be specified')
  const [value, options] = args
  if (options?.transfer !== undefined && [...options.transfer].length > 0)
    throw cannotClone("Neo's structuredClone cannot transfer")
  return clone(value, new Map())
}

function clone(value: unknown, memory: Map<object, unknown>): unknown {
  if (typeof value === 'function' || typeof value === 'symbol')
    throw cannotClone(`${String(value)} could not be cloned.`)
  if (typeof value !== 'object' || value === null) return value
  const seen = memory.get(value)
  if (seen !== undefined) return seen
  const remember = <T>(copy: T): T => {
    memory.set(value, copy)
    return copy
  }

  if (value instanceof Boolean) return remember(Object(value.valueOf()))
  if (value instanceof Number) return remember(Object(value.valueOf()))
  if (value instanceof String) return remember(Object(value.valueOf()))
  if (value instanceof BigInt) return remember(Object(value.valueOf()))
  if (value instanceof Date) return remember(new Date(value.getTime()))
  if (value instanceof RegExp)
    return remember(new RegExp(value.source, value.flags))
  if (value instanceof ArrayBuffer) return remember(value.slice(0))
  if (ArrayBuffer.isView(value)) {
    const buffer = clone(value.buffer, memory) as ArrayBuffer
    if (value instanceof DataView)
      return remember(new DataView(buffer, value.byteOffset, value.byteLength))
    const kind = typedArrayKind(value)
    const length = value.byteLength / kind.BYTES_PER_ELEMENT
    return remember(new kind(buffer, value.byteOffset, length))
  }
  if (value instanceof Map) {
    const copy = remember(new Map())
    for (const [key, entry] of value)
      copy.set(clone(key, memory), clone(entry, memory))
    return copy
  }
  if (value instanceof Set) {
    const copy = remember(new Set())
    for (const entry of value) copy.add(clone(entry, memory))
    return copy
  }
  if (value instanceof Error) {
    const kind = ERRORS[value.name] ?? Error
    const copy = remember(new kind(value.message))
    if (value.stack !== undefined)
      Object.defineProperty(copy, 'stack', {
        value: value.stack,
        writable: true,
        configurable: true,
      })
    return copy
  }
  if (
    value instanceof Promise ||
    value instanceof WeakMap ||
    value instanceof WeakSet ||
    value instanceof WeakRef ||
    value instanceof URL ||
    value instanceof URLSearchParams
  )
    throw cannotClone(
      `${Object.prototype.toString.call(value)} could not be cloned.`,
    )

  const copy = remember(
    Array.isArray(value) ? new Array(value.length) : {},
  ) as Record<string, unknown>
  // Defined, not assigned: a `__proto__` key stays an own key instead of setting the prototype.
  for (const key of Object.keys(value))
    Object.defineProperty(copy, key, {
      value: clone((value as Record<string, unknown>)[key], memory),
      writable: true,
      enumerable: true,
      configurable: true,
    })
  return copy
}

/** The typed array class a clone of `view` takes: its built-in class, so a `Buffer` comes back a
 * `Uint8Array`, as in Node. */
function typedArrayKind(view: ArrayBufferView): TypedArrayKind {
  return (
    TYPED_ARRAYS.find((kind) => view instanceof kind) ??
    (view.constructor as TypedArrayKind)
  )
}

function cannotClone(message: string): DOMException {
  return new DOMException(message, 'DataCloneError')
}
