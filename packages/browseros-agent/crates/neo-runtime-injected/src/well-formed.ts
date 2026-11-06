/**
 * Every page call's answer, made safe to cross to Rust: each lone surrogate in a string result or
 * a thrown error's message becomes U+FFFD. Neo-only: Rust holds strings as UTF-8, and browseros-cdp
 * drops a CDP reply whose JSON escapes a lone surrogate, so one truncated emoji in the page (in a
 * read's text, a click's element preview, or a strict mode violation listing its matches) would
 * otherwise leave the call waiting until its timeout. Playwright passes the lone surrogate through.
 *
 * `install.ts` wraps every glue function in `wellFormedCall`, the one place all page calls pass.
 * Page objects in a result (an element, an armed interceptor's `{stop}`) are kept as they are,
 * by identity: Rust holds them as handles.
 */

/** Runs one glue function and returns its result, or throws its error, well-formed. */
export function wellFormedCall(run: () => unknown): unknown {
  let result: unknown
  try {
    result = run()
  } catch (error) {
    throw wellFormedError(error)
  }
  if (result instanceof Promise) {
    return result.then(wellFormedResult, (error: unknown) => {
      throw wellFormedError(error)
    })
  }
  return wellFormedResult(result)
}

/**
 * Strings well-formed, through arrays and plain data objects. A page object stops the walk: a DOM
 * node or any other class instance, and a plain object holding a function (an armed interceptor).
 */
function wellFormedResult(value: unknown): unknown {
  if (typeof value === 'string') return value.toWellFormed()
  if (Array.isArray(value)) return value.map(wellFormedResult)
  if (!isPlainData(value)) return value
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => [key, wellFormedResult(item)]),
  )
}

function isPlainData(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === 'object' &&
    value !== null &&
    Object.getPrototypeOf(value) === Object.prototype &&
    !Object.values(value).some((item) => typeof item === 'function')
  )
}

/**
 * The same error with its message and stack well-formed, which is what CDP's
 * `exception.description` is made from.
 */
function wellFormedError(error: unknown): unknown {
  if (typeof error === 'string') return error.toWellFormed()
  if (error instanceof Error) {
    error.message = error.message.toWellFormed()
    if (typeof error.stack === 'string')
      error.stack = error.stack.toWellFormed()
  }
  return error
}
