/** Neo: Lossless values for the private host/QuickJS transport. Strings never pass through UTF-8. */
export function codecFactory({ identify, resolve }) {
  class CodecError extends Error {
    name = 'HarnessCodecError'
  }
  const units = (text) =>
    Array.from({ length: text.length }, (_, i) => text.charCodeAt(i))
  const string = (data) => {
    if (
      !Array.isArray(data) ||
      data.some((unit) => !Number.isInteger(unit) || unit < 0 || unit > 65535)
    )
      throw new CodecError('Invalid UTF-16 units')
    let result = ''
    for (let i = 0; i < data.length; i += 8192)
      result += String.fromCharCode(...data.slice(i, i + 8192))
    return result
  }
  function encode(value, ancestors = new Set()) {
    if (value === undefined) return { tag: 'undefined' }
    if (value === null) return { tag: 'null' }
    if (typeof value === 'boolean') return { tag: 'boolean', value }
    if (typeof value === 'string') return { tag: 'string', units: units(value) }
    if (typeof value === 'bigint')
      return { tag: 'bigint', decimal: String(value) }
    if (typeof value === 'number') {
      if (Object.is(value, -0)) return { tag: 'negativeZero' }
      if (Number.isNaN(value)) return { tag: 'nan' }
      if (value === Infinity) return { tag: 'infinity' }
      if (value === -Infinity) return { tag: 'negativeInfinity' }
      return { tag: 'number', value }
    }
    if (typeof value !== 'object')
      throw new CodecError(`Untransportable ${typeof value}`)
    const ref = identify(value)
    if (ref) return { tag: 'object', ref }
    if (value instanceof RegExp)
      return {
        tag: 'regexp',
        sourceUnits: units(value.source),
        flags: value.flags,
        lastIndex: encode(value.lastIndex),
      }
    if (ancestors.has(value)) throw new CodecError('Untransportable cycle')
    ancestors.add(value)
    try {
      if (Array.isArray(value)) {
        return {
          tag: 'array',
          values: Array.from({ length: value.length }, (_, i) =>
            i in value ? encode(value[i], ancestors) : { tag: 'hole' },
          ),
        }
      }
      if (
        Object.getPrototypeOf(value) !== Object.prototype &&
        Object.getPrototypeOf(value) !== null
      )
        throw new CodecError(
          `Untransportable object ${value.constructor?.name || 'unknown'}`,
        )
      if (Object.getOwnPropertySymbols(value).length)
        throw new CodecError('Untransportable symbol key')
      return {
        tag: 'record',
        nullPrototype: Object.getPrototypeOf(value) === null,
        entries: Object.keys(value).map((key) => [
          units(key),
          encode(value[key], ancestors),
        ]),
      }
    } finally {
      ancestors.delete(value)
    }
  }
  function decode(value) {
    if (!value || typeof value !== 'object')
      throw new CodecError('Invalid tagged value')
    switch (value.tag) {
      case 'undefined':
        return undefined
      case 'null':
        return null
      case 'boolean':
        if (typeof value.value === 'boolean') return value.value
        break
      case 'number':
        if (typeof value.value === 'number' && Number.isFinite(value.value))
          return value.value
        break
      case 'negativeZero':
        return -0
      case 'nan':
        return NaN
      case 'infinity':
        return Infinity
      case 'negativeInfinity':
        return -Infinity
      case 'string':
        return string(value.units)
      case 'bigint':
        return BigInt(value.decimal)
      case 'object':
        return resolve(value.ref)
      case 'regexp': {
        const result = new RegExp(string(value.sourceUnits), value.flags)
        result.lastIndex = decode(value.lastIndex)
        return result
      }
      case 'array': {
        const result = new Array(value.values.length)
        value.values.forEach((item, i) => {
          if (item.tag !== 'hole') result[i] = decode(item)
        })
        return result
      }
      case 'record': {
        const result = Object.create(
          value.nullPrototype ? null : Object.prototype,
        )
        for (const [key, item] of value.entries)
          Object.defineProperty(result, string(key), {
            value: decode(item),
            writable: true,
            configurable: true,
            enumerable: true,
          })
        return result
      }
    }
    throw new CodecError(`Invalid tagged value ${value.tag}`)
  }
  return {
    encode,
    decode,
    units,
    string,
    isError: (value) => value instanceof CodecError,
  }
}
