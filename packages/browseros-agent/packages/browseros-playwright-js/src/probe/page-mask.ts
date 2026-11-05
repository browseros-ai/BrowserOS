/**
 * The mask the probe puts on page text before Playwright's engine cuts or re-encodes it.
 *
 * The engine shortens page text inside the page: a preview keeps 50 characters of an
 * element's text, and a locator hint keeps the words before a cut or drops a trailing
 * number. Rust masks whole values, and a value cut short no longer matches, so the hint
 * `getByRole('heading', { name: 'Signed in with hunter2-secret-' })` would leave the page
 * with most of a password in it. Hints also quote and escape text (`\'`, `\.`), which a mask
 * of plain values misses too.
 *
 * So each call carries the raw values of every sensitive field of its frame's site (Rust's
 * site sweep), to which the probe adds its own frame's values as they are during the call
 * (index.ts), and every place where the engine cuts or re-encodes page text for a preview or
 * a hint calls `maskPageText` on the text first: the string helpers scripts/build.ts swaps
 * in for InjectedScript and the selector generator (masked-strings.ts), and the generator's
 * own `suitableTextAlternatives` and `escapeClassName`, which the build has mask their input.
 * A call that reaches one of those places without a mask is answered `needsMask` instead of
 * its reply, and Rust asks again with the mask. Rust masks every reply again afterwards with
 * the same values, which changes nothing here because both passes use the whole set.
 */
import type { Mask } from './protocol'

/** What a masked value becomes: core's `ECHO_MASK`, the token Rust's mask writes too. */
const ECHO_MASK = '[private]'

/**
 * Rust sends only values of at least this many characters (core's `MIN_SUBSTRING_LEN`), and the
 * frame's own values are held to the same; a form of one that comes out shorter is not masked
 * either, since it would mask ordinary text.
 */
const MIN_LENGTH = 4

/** One call's state: its mask, and whether page text reached a cut without one. */
type CallMask = { forms: string[] | undefined; unmasked: boolean }

/** The call running now. Probe calls are synchronous and never nest, so one slot holds it. */
let running: CallMask | undefined

/**
 * Runs one call with `mask` in force. `needsMask` is whether the call produced page text for
 * a preview or a hint with no mask to put on it, in which case its value must not leave the
 * page.
 */
export function underMask<T>(
  mask: Mask | undefined,
  run: () => T,
): { value: T; needsMask: boolean } {
  const call: CallMask = {
    forms: mask === undefined ? undefined : formsOf(mask),
    unmasked: false,
  }
  const outer = running
  running = call
  try {
    const value = run()
    return { value, needsMask: call.unmasked }
  } finally {
    running = outer
  }
}

/**
 * `text` with every form of every value of the call's mask replaced by `[private]`, leftmost
 * and longest first, as core's `FieldValues` replaces them. Without a mask the call is marked
 * as needing one and `text` comes back as it is, never to leave the page.
 */
export function maskPageText(text: string): string {
  const call = running
  if (!call) return text
  if (!call.forms) {
    call.unmasked = true
    return text
  }
  return replaceForms(text, call.forms)
}

/**
 * Each value as page text can hold it. Rust's mask catches the same forms at the end (core's
 * `build_matcher`): the value itself, JSON-escaped, percent-encoded (`%20` also as `+`), and
 * HTML-escaped in text and in attribute values. Before a cut the text is also the engine's
 * whitespace-normalized copy of the page's (an element's text, a label, an accessible name),
 * so the value's own normalized form is matched as well.
 */
function formsOf(mask: Mask): string[] {
  const forms = new Set<string>()
  for (const value of mask) {
    if ([...value].length < MIN_LENGTH) continue
    const encoded = encodeURIComponent(value)
    for (const form of [
      value,
      JSON.stringify(value).slice(1, -1),
      encoded,
      encoded.replaceAll('%20', '+'),
      htmlEscaped(value, false),
      htmlEscaped(value, true),
      whiteSpaceNormalized(value),
      flatString(value),
    ])
      if ([...form].length >= MIN_LENGTH) forms.add(form)
  }
  return [...forms]
}

/** As Chromium's HTML serializer writes `value` in text, or in an attribute value. */
function htmlEscaped(value: string, attribute: boolean): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll(' ', '&nbsp;')
    .replaceAll('"', attribute ? '&quot;' : '"')
}

/**
 * stringUtils' `normalizeWhiteSpace`, which element text and labels go through. Upstream's
 * own copy may cache what it normalizes for as long as the world lives, so it is not reused.
 */
function whiteSpaceNormalized(value: string): string {
  return value
    .replace(/[\u200b\u00ad]/g, '')
    .trim()
    .replace(/\s+/g, ' ')
}

/** roleUtils' `asFlatString`, which accessible names go through: it keeps U+00A0. */
function flatString(value: string): string {
  return value
    .split(' ')
    .map((chunk) =>
      chunk.replace(/\r\n/g, '\n').replace(/[​­]/g, '').replace(/\s\s*/g, ' '),
    )
    .join(' ')
    .trim()
}

/** Leftmost-longest replacement of `forms` in `text`, in one pass. */
function replaceForms(text: string, forms: string[]): string {
  const found: { start: number; end: number }[] = []
  for (const form of forms) {
    for (
      let start = text.indexOf(form);
      start !== -1;
      start = text.indexOf(form, start + 1)
    )
      found.push({ start, end: start + form.length })
  }
  if (found.length === 0) return text
  found.sort((a, b) => a.start - b.start || b.end - a.end)
  let masked = ''
  let cursor = 0
  for (const { start, end } of found) {
    if (start < cursor) continue
    masked += text.slice(cursor, start) + ECHO_MASK
    cursor = end
  }
  return masked + text.slice(cursor)
}
