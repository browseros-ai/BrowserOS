/**
 * An `evaluate` result as the script got it. Neo records the page's serialized form of the value
 * (`{ id, o: [{ k, v }] }` for an object, `{ v: 'undefined' }`, `{ d: … }` for a Date, …), and
 * the prelude turns it back into a value with Playwright's `parseEvaluationResultValue`
 * (`neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers.ts`).
 * This prints the value that function returns, without building it: JSON's layout for
 * objects and arrays, and JavaScript's own words for what JSON cannot say (`undefined`, `NaN`,
 * `Date`, `Map`, a circular reference). Each case below is one branch of that function.
 *
 * Neo's trace can cut a long value: an array or entry list then ends in `{ $more: N }`, and a
 * string in `…[+N bytes]`. Both print as they are, so a cut value reads as cut.
 */

const INDENT = '  '

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** A string the script got: its own text at the top, quoted inside a structure. */
export function pageValueText(value: unknown): string {
  return typeof value === 'string' ? value : shown(value, '', new Set())
}

function shown(value: unknown, indent: string, seen: Set<unknown>): string {
  if (value === undefined) return 'undefined'
  if (!isRecord(value)) return JSON.stringify(value) ?? String(value)
  if ('ref' in value) return '[Circular]'
  if ('v' in value) return typeof value.v === 'string' ? value.v : 'undefined'
  if ('d' in value) return `Date ${JSON.stringify(value.d)}`
  if ('u' in value) return `URL ${JSON.stringify(value.u)}`
  if ('bi' in value) return `${String(value.bi)}n`
  if ('e' in value && isRecord(value.e)) {
    return `${String(value.e.n ?? 'Error')}: ${String(value.e.m ?? '')}`
  }
  if ('r' in value && isRecord(value.r)) {
    return `/${String(value.r.p ?? '')}/${String(value.r.f ?? '')}`
  }
  if ('h' in value) return 'JSHandle'
  if ('fn' in value) return 'function'
  if ('ta' in value && isRecord(value.ta))
    return `${String(value.ta.k)} [bytes]`
  if ('ab' in value) return 'ArrayBuffer [bytes]'
  if ('$more' in value) return `… ${String(value.$more)} more`
  if (seen.has(value)) return '[Circular]'
  seen.add(value)
  const inner = indent + INDENT
  if (Array.isArray(value.a)) {
    return block(
      '[',
      ']',
      value.a.map((item) => shown(item, inner, seen)),
      indent,
    )
  }
  if (Array.isArray(value.o)) {
    return block(
      '{',
      '}',
      value.o.map((entry) =>
        isRecord(entry) && 'k' in entry
          ? `${JSON.stringify(String(entry.k))}: ${shown(entry.v, inner, seen)}`
          : shown(entry, inner, seen),
      ),
      indent,
    )
  }
  if (Array.isArray(value.m)) {
    return block(
      'Map {',
      '}',
      value.m.map((entry) =>
        isRecord(entry) && 'k' in entry
          ? `${shown(entry.k, inner, seen)} => ${shown(entry.v, inner, seen)}`
          : shown(entry, inner, seen),
      ),
      indent,
    )
  }
  if (Array.isArray(value.s)) {
    return block(
      'Set {',
      '}',
      value.s.map((item) => shown(item, inner, seen)),
      indent,
    )
  }
  // Not one of the page's forms: printed as recorded.
  return JSON.stringify(value, null, 2) ?? String(value)
}

function block(
  open: string,
  close: string,
  items: string[],
  indent: string,
): string {
  if (items.length === 0) return `${open}${close}`
  const inner = indent + INDENT
  return `${open}\n${items.map((item) => `${inner}${item}`).join(',\n')}\n${indent}${close}`
}
