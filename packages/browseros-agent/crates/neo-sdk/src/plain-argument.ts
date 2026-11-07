/**
 * Neo: `dispatchEvent`'s `eventInit` on the wire, until med-pw-eval's `serializeArgument`
 * (neo-sdk's `js-handle.ts`, the evaluation seam) is on this branch. It writes Playwright's
 * protocol `SerializedArgument` for the values a script can pass without handles, in the shape
 * of Playwright's own `serializeValue`, and sends no handles: Rust keeps the plain JSON part and
 * refuses the rest before any page call (`neo-runtime/src/element_ops.rs`). A function or a
 * typed array throws here, before anything is sent.
 */

import type {
  SerializedArgument,
  SerializedValue,
} from '@browseros/neo-runtime-spec/serve'

/** Neo: the `eventInit` of `Frame.dispatchEvent`, with no handles. */
export function serializePlainArgument(value: unknown): SerializedArgument {
  return {
    value: serialize(value, { lastId: 0, visited: new Map() }, []),
    handles: [],
  }
}

/** Playwright: `protocol/src/serializers.ts` `innerSerializeValue`: `return { v: 'undefined' };` */
function serialize(
  value: unknown,
  visitorInfo: { lastId: number; visited: Map<object, number> },
  accessChain: Array<string | number>,
): SerializedValue {
  if (typeof value === 'symbol') return { v: 'undefined' }
  if (Object.is(value, undefined)) return { v: 'undefined' }
  if (Object.is(value, null)) return { v: 'null' }
  if (Object.is(value, Number.NaN)) return { v: 'NaN' }
  if (Object.is(value, Number.POSITIVE_INFINITY)) return { v: 'Infinity' }
  if (Object.is(value, Number.NEGATIVE_INFINITY)) return { v: '-Infinity' }
  if (Object.is(value, -0)) return { v: '-0' }
  if (typeof value === 'boolean') return { b: value }
  if (typeof value === 'number') return { n: value }
  if (typeof value === 'string') return { s: value }
  if (typeof value === 'bigint') return { bi: value.toString() }
  if (value instanceof Error)
    return { e: { n: value.name, m: value.message, s: value.stack || '' } }
  if (value instanceof Date) return { d: value.toJSON() }
  if (typeof URL !== 'undefined' && value instanceof URL)
    return { u: value.toJSON() }
  if (value instanceof RegExp) return { r: { p: value.source, f: value.flags } }
  // Deviation: `protocol/src/serializers.ts` `innerSerializeValue`: `const typedArrayKind = constructorToTypedArrayKind.get(value.constructor);` Playwright sends a typed array's bytes; the realm has no `Buffer` to write them with, so Neo refuses one here.
  if (ArrayBuffer.isView(value)) refuseTypedArray()
  if (typeof value === 'object' && value !== null) {
    const seen = visitorInfo.visited.get(value)
    if (seen) return { ref: seen }
    if (Array.isArray(value)) {
      const id = ++visitorInfo.lastId
      visitorInfo.visited.set(value, id)
      const a = value.map((item, i) =>
        serialize(item, visitorInfo, [...accessChain, i]),
      )
      return { a, id }
    }
    const id = ++visitorInfo.lastId
    visitorInfo.visited.set(value, id)
    const o = Object.keys(value).map((name) => ({
      k: name,
      v: serialize((value as Record<string, unknown>)[name], visitorInfo, [
        ...accessChain,
        name,
      ]),
    }))
    return { o, id }
  }
  throw new Error(
    `Attempting to serialize unexpected value${accessChainToDisplayString(accessChain)}: ${String(value)}`,
  )
}

/** Neo: the typed-array refusal, worded as Rust's refusal of the values it cannot dispatch
 * (`neo-runtime/src/element_ops.rs` `EVENT_INIT_UNSERVED`). */
function refuseTypedArray(): never {
  throw new Error(
    "Neo's dispatchEvent takes plain eventInit values (strings, finite numbers, booleans, null, arrays and plain objects); this one holds a typed array",
  )
}

/** Playwright: `protocol/src/serializers.ts` `accessChainToDisplayString` */
function accessChainToDisplayString(
  accessChain: Array<string | number>,
): string {
  const chainString = accessChain
    .map((accessor, i) => {
      if (typeof accessor === 'string') return i ? `.${accessor}` : accessor
      return `[${accessor}]`
    })
    .join('')
  return chainString.length > 0 ? ` at position "${chainString}"` : ''
}
