// What the evaluation parity files share (evaluate, evaluateHandle, frame-evaluate,
// locator-evaluate, jsHandle-jsonValue, jsHandle-dispose, evaluate-world, and the selector,
// handle and wait files: query, evaluateAll, evalOnSelector, locator-elementHandle,
// locator-evaluateHandle, getProperty, waitForFunction): `describe`, which each
// script gets first, so both engines report any value an evaluation can return as the same JSON:
// numbers JSON cannot carry (NaN, ±Infinity, -0), undefined, BigInt, Date, RegExp, URL, Map, Set,
// typed arrays, circular references, and an object's own keys whose value is undefined. Values are
// read through the public behaviour both realms share (`Object.is`, `instanceof Date`, `String`),
// so Node's and QuickJS's builtins report alike; a URL is read by its text (Neo's realm has no
// `URL` class: `vendor-shims.ts`).
//
// `outcome(promise)` settles a call as `{ value: describe(value) }` or
// `{ error: errorCategory(error), mentions }`, where `mentions` lists which of the strings a case
// names its message contains: the words Playwright's own tests check, never the whole sentence.

function describe(value, seen = new Map(), path = '$') {
  if (value === undefined) return { undefined: true }
  if (value === null) return null
  if (typeof value === 'number') {
    if (Object.is(value, -0)) return { number: '-0' }
    if (Number.isNaN(value)) return { number: 'NaN' }
    if (!Number.isFinite(value)) return { number: String(value) }
    return value
  }
  if (typeof value === 'bigint') return { bigint: String(value) }
  if (typeof value !== 'object' && typeof value !== 'function') return value
  if (typeof value === 'function') return { function: true }
  if (seen.has(value)) return { circular: seen.get(value) }
  if (value instanceof Date) return { date: value.toISOString() }
  if (value instanceof RegExp) return { regexp: String(value) }
  if (Object.prototype.toString.call(value) === '[object URL]')
    return { url: String(value), href: value.href }
  if (ArrayBuffer.isView(value))
    return {
      typed: Object.prototype.toString.call(value),
      items: Array.from(value, String),
    }
  if (value instanceof Map)
    return { map: [...value].map(([k, v]) => [describe(k), describe(v)]) }
  if (value instanceof Set) return { set: [...value].map((v) => describe(v)) }
  if (value instanceof Error)
    return { error: value.name, message: value.message }
  seen.set(value, path)
  if (Array.isArray(value))
    return value.map((item, index) => describe(item, seen, `${path}[${index}]`))
  const keys = Object.keys(value)
  return {
    object: Object.fromEntries(
      keys.map((key) => [key, describe(value[key], seen, `${path}.${key}`)]),
    ),
  }
}

/** Both engines' scripts start with these, after the harness's own `errorCategory`. */
export const VALUES = `${describe.toString()}
const outcome = async (call, mentions = []) => {
  try {
    return { value: describe(await call()) }
  } catch (error) {
    return {
      error: errorCategory(error),
      mentions: mentions.filter((word) => String(error && error.message).includes(word)),
    }
  }
}
`

/**
 * `settled(handle)` and `previews(handles)`: a handle's `toString()` once its preview is final,
 * for scripts that compare element handles. Neo's node previews are final at once; Playwright's
 * start as `JSHandle@node` and arrive by event.
 */
export const SETTLED = `
const settled = async (handle) => {
  if (!handle) return handle
  for (let attempt = 0; attempt < 100 && handle.toString() === 'JSHandle@node'; attempt++)
    await new Promise((resolve) => setTimeout(resolve, 20))
  return handle.toString()
}
const previews = async (list) => {
  const out = []
  for (const handle of list) out.push(await settled(handle))
  return out
}
`
