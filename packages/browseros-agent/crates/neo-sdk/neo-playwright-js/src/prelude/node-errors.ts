/**
 * Node's errors for a bad argument to the Node APIs the prelude gives scripts (buffer.ts):
 * Node's error class, its `code` and its message, with Node's way of naming what it received,
 * so a script that branches on them, or prints them, sees what it would under Node.
 */

/** A Node error: `name` TypeError or RangeError, Node's `code` and message. */
export function nodeError(
  Kind: TypeErrorConstructor | RangeErrorConstructor,
  code: string,
  message: string,
): Error {
  const error = new Kind(message)
  Reflect.set(error, 'code', code)
  return error
}

/** Node's `determineSpecificType`: how its argument errors name what they received. */
export function received(value: unknown): string {
  if (value === null) return 'Received null'
  if (value === undefined) return 'Received undefined'
  switch (typeof value) {
    case 'function':
      return `Received function ${value.name}`
    case 'object': {
      const named: unknown = Reflect.get(value, 'constructor')
      return typeof named === 'function'
        ? `Received an instance of ${named.name}`
        : 'Received [Object: null prototype] {}'
    }
    case 'number':
      return `Received type number (${Object.is(value, -0) ? '-0' : String(value)})`
    case 'bigint':
      return `Received type bigint (${value}n)`
    case 'string': {
      const shown = value.length > 28 ? `${value.slice(0, 25)}...` : value
      return shown.includes("'")
        ? `Received type string (${JSON.stringify(shown)})`
        : `Received type string ('${shown}')`
    }
    default:
      return `Received type ${typeof value} (${String(value)})`
  }
}

/**
 * Node's `ERR_OUT_OF_RANGE` for argument `name`, which must be `range`: the value as Node's
 * `inspect` prints a number, with `_` between thousands past 2 ** 32.
 */
export function outOfRange(name: string, range: string, value: number): Error {
  const shown =
    Number.isInteger(value) && Math.abs(value) > 2 ** 32
      ? withSeparators(String(value))
      : Object.is(value, -0)
        ? '-0'
        : String(value)
  return nodeError(
    RangeError,
    'ERR_OUT_OF_RANGE',
    `The value of "${name}" is out of range. It must be ${range}. Received ${shown}`,
  )
}

/** Node's `addNumericalSeparator`. */
function withSeparators(digits: string): string {
  let grouped = ''
  let end = digits.length
  const start = digits.startsWith('-') ? 1 : 0
  for (; end >= start + 4; end -= 3)
    grouped = `_${digits.slice(end - 3, end)}${grouped}`
  return `${digits.slice(0, end)}${grouped}`
}

/**
 * Node's `ERR_INVALID_ARG_VALUE` for argument `value`, as a fill with no bytes to repeat gets
 * it. Node prints the value with `inspect`; this prints the two kinds that can be refused that
 * way, a string (quoted and escaped as `inspect` does) and an empty view.
 */
export function invalidValue(value: unknown): Error {
  let shown = inspected(value)
  if (shown.length > 128) shown = `${shown.slice(0, 128)}...`
  return nodeError(
    TypeError,
    'ERR_INVALID_ARG_VALUE',
    `The argument 'value' is invalid. Received ${shown}`,
  )
}

/** `inspect`'s escapes for the characters it does not print as they are. */
const ESCAPES = new Map([
  [8, '\\b'],
  [9, '\\t'],
  [10, '\\n'],
  [12, '\\f'],
  [13, '\\r'],
  [92, '\\\\'],
])

function inspected(value: unknown): string {
  if (typeof value !== 'string') {
    const kind: unknown = Reflect.get(Object(value), 'constructor')
    const name = typeof kind === 'function' ? kind.name : 'Object'
    return name === 'Buffer' ? '<Buffer >' : `${name}(0) []`
  }
  // inspect's quote: single, else double when the text has a single quote, else a backtick.
  const quote = !value.includes("'")
    ? "'"
    : !value.includes('"')
      ? '"'
      : !value.includes('`') && !value.includes('${')
        ? '`'
        : "'"
  // Only the first 128 characters are shown, and each is at least one character escaped.
  const source = value.slice(0, 129)
  let text = ''
  for (let index = 0; index < source.length; index++) {
    const code = source.charCodeAt(index)
    const pair =
      code >= 0xd800 &&
      code <= 0xdbff &&
      (source.charCodeAt(index + 1) & 0xfc00) === 0xdc00
    if (pair) {
      text += source.slice(index, index + 2)
      index++
    } else if (code === 0x27 && quote === "'") text += "\\'"
    else if (ESCAPES.has(code)) text += ESCAPES.get(code)
    else if (code < 0x20 || (code >= 0x7f && code < 0xa0))
      text += `\\x${code.toString(16).toUpperCase().padStart(2, '0')}`
    else if (code >= 0xd800 && code <= 0xdfff) text += `\\u${code.toString(16)}`
    else text += source[index]
  }
  return `${quote}${text}${quote}`
}
