/** Test-only assertions over Neo's public reads. This deliberately does not emulate Playwright matchers. */
import { AssertionError } from 'node:assert'
import { setTimeout as sleep } from 'node:timers/promises'
import { inspect, isDeepStrictEqual } from 'node:util'

const normalize = (value) =>
  value
    .replace(/[\u200b\u00ad]/g, '')
    .replace(/\s+/g, ' ')
    .trim()

function matches(
  received,
  expected,
  options = {},
  contains = false,
  whitespace = false,
) {
  if (Array.isArray(expected)) {
    if (!Array.isArray(received)) return false
    if (!contains && received.length !== expected.length) return false
    let index = 0
    return expected.every((item) => {
      if (!contains)
        return matches(received[index++], item, options, false, whitespace)
      while (index < received.length)
        if (matches(received[index++], item, options, true, whitespace))
          return true
      return false
    })
  }
  if (expected instanceof RegExp) {
    if (typeof received !== 'string') return false
    const flags =
      options.ignoreCase === undefined
        ? expected.flags
        : options.ignoreCase
          ? `${expected.flags.replaceAll('i', '')}i`
          : expected.flags.replaceAll('i', '')
    return new RegExp(expected.source, flags).test(received)
  }
  if (typeof received === 'string' && typeof expected === 'string') {
    if (whitespace) {
      received = normalize(received)
      expected = normalize(expected)
    }
    if (options.ignoreCase) {
      received = received.toLowerCase()
      expected = expected.toLowerCase()
    }
    return contains ? received.includes(expected) : received === expected
  }
  return isDeepStrictEqual(received, expected)
}

async function check(read, expected, compare = matches) {
  const received = await read
  return { received, expected, pass: compare(received, expected) }
}

function textCheck(x, expected, options, contains = false) {
  const read = Array.isArray(expected)
    ? options.useInnerText
      ? x.allInnerTexts()
      : x.allTextContents()
    : options.useInnerText
      ? x.innerText(options)
      : x.textContent(options)
  return check(read, expected, (a, b) => matches(a, b, options, contains, true))
}

// Each entry is [required arguments before options, public read + comparison].
// biome-ignore format: Keep the coverage table at one line per check.
const checks = {
  toBeVisible: [0, (x, _a, o) => check(x.isVisible(o), o.visible ?? true)],
  toBeHidden: [0, (x, _a, o) => check(x.isHidden(o), true)],
  toBeEnabled: [0, (x, _a, o) => check(x.isEnabled(o), o.enabled ?? true)],
  toBeDisabled: [0, (x, _a, o) => check(x.isDisabled(o), true)],
  toBeChecked: [0, (x, _a, o) => check(o.indeterminate ? x.evaluate(el => el.indeterminate, undefined, o) : x.isChecked(o), o.checked ?? true)],
  toBeEditable: [0, (x, _a, o) => check(x.isEditable(o), o.editable ?? true)],
  toBeFocused: [0, (x, _a, o) => check(x.evaluate(el => el.getRootNode().activeElement === el && el.ownerDocument.hasFocus(), undefined, o), true)],
  toBeAttached: [0, (x, _a, o) => check(x.count().then(n => n > 0), o.attached ?? true)],
  toBeEmpty: [0, (x, _a, o) => check(x.evaluate(el => 'value' in el ? el.value : el.textContent, undefined, o), '', (r, e) => matches(r, e, o, false, true))],
  toHaveText: [1, (x, a, o) => textCheck(x, a[0], o)],
  toContainText: [1, (x, a, o) => textCheck(x, a[0], o, true)],
  toHaveValue: [1, (x, a, o) => check(x.inputValue(o), a[0])],
  toHaveValues: [1, (x, a, o) => check(x.evaluate(el => Array.from(el.selectedOptions, option => option.value), undefined, o), a[0])],
  toHaveAttribute: [1, (x, a, o) => check(x.getAttribute(a[0], o), a.length === 1 ? 'attribute present' : a[1], (r, e) => a.length === 1 ? r !== null : matches(r, e, o))],
  toHaveClass: [1, (x, a, o) => check(Array.isArray(a[0]) ? x.evaluateAll(els => els.map(el => el.getAttribute('class') || '')) : x.getAttribute('class', o).then(value => value || ''), a[0], (r, e) => matches(r, e, o, false, true))],
  toHaveCount: [1, (x, a) => check(x.count(), a[0])],
  toHaveCSS: [2, (x, a, o) => check(x.evaluate((el, [name, pseudo]) => el.ownerDocument.defaultView.getComputedStyle(el, pseudo).getPropertyValue(name), [a[0], o.pseudo ? `::${o.pseudo.replace(/^:+/, '')}` : null], o), a[1])],
  toHaveId: [1, (x, a, o) => check(x.getAttribute('id', o), a[0])],
  toHaveJSProperty: [2, (x, a, o) => check(x.evaluate((el, name) => el[name], a[0], o), a[1], isDeepStrictEqual)],
  toHaveURL: [1, (x, a, o) => check(x.url(), a[0], (r, e) => typeof e === 'function' ? e(new URL(r)) : typeof URLPattern !== 'undefined' && e instanceof URLPattern ? e.test(r) : matches(r, e, o))],
  toHaveTitle: [1, (x, a, o) => check(x.title(), a[0], (r, e) => matches(r, e, o, false, true))],
  // TODO: toMatchAriaSnapshot is not covered; keep it a visible skip.
}

async function beforeDeadline(promise, remaining) {
  if (remaining === Infinity) return await promise
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('read timed out')), remaining)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function poll(actual, name, read, args, options, isNot) {
  const timeout = options.timeout ?? 5000
  const deadline = timeout === 0 ? Infinity : performance.now() + timeout
  let last = {
    received: 'no successful read',
    expected: args.length ? args : true,
  }
  let readError
  do {
    try {
      last = await beforeDeadline(
        read(actual, args, {
          ...options,
          timeout: Math.max(1, Math.min(100, deadline - performance.now())),
        }),
        Math.max(1, deadline - performance.now()),
      )
      readError = undefined
      if (last.pass !== isNot) return
    } catch (error) {
      readError = error
      if (error instanceof TypeError) break
    }
    const remaining = deadline - performance.now()
    if (remaining <= 0) break
    await sleep(Math.min(100, remaining))
  } while (performance.now() < deadline)
  const detail = readError
    ? `; read failed: ${String(readError.message ?? readError).replace(/\bx\./g, `${actual.constructor.name}.`)}`
    : ''
  throw new AssertionError({
    message: `expectLoop: ${isNot ? 'not.' : ''}${name} failed${detail}\nExpected: ${inspect(last.expected)}\nLast received: ${inspect(last.received)}`,
    actual: last.received,
    expected: last.expected,
    operator: `${isNot ? 'not.' : ''}${name}`,
    cause: readError,
  })
}

export function expectLoop(
  actual,
  { info = () => undefined } = {},
  isNot = false,
) {
  return new Proxy(
    {},
    {
      get(_, name) {
        if (name === 'not') return expectLoop(actual, { info }, !isNot)
        if (typeof name !== 'string' || name === 'then') return undefined
        return async (...args) => {
          const testInfo = info()
          testInfo?.annotations.push({ type: 'expectLoop', description: name })
          if (!Object.hasOwn(checks, name)) {
            const reason = `expectLoop: ${name} is not covered`
            testInfo?.skip(true, reason)
            throw new Error(reason)
          }
          const [arity, read] = checks[name]
          const last = args.at(-1)
          const options =
            args.length > arity && last?.constructor === Object
              ? args.pop()
              : {}
          return poll(actual, name, read, args, options, isNot)
        }
      },
    },
  )
}

export function createExpect(realExpect, { isNeo, info } = {}) {
  return Object.assign(
    (actual, ...args) =>
      isNeo(actual)
        ? expectLoop(actual, { info })
        : realExpect(actual, ...args),
    realExpect,
  )
}
