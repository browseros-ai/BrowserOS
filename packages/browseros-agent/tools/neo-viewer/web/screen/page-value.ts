/**
 * An `evaluate` result as the script got it. Neo records the page's serialized form of the value
 * (`{ id, o: [{ k, v }] }` for an object, `{ v: 'undefined' }`, `{ d: … }` for a Date, …), and
 * the prelude turns it back into a value with Playwright's `parseEvaluationResultValue`
 * (`neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers.ts`).
 * This prints the value that function returns, without building it: JSON's layout for objects
 * and arrays, and JavaScript's own words for what JSON cannot say (`undefined`, `NaN`, `Date`,
 * `Map`, a typed array's numbers). Each case below is one branch of that function.
 *
 * References follow the decoder: `{ ref: id }` is the object serialized earlier under `id`, so it
 * prints that object again, as the script's value holds it twice; only a reference back to an
 * object still being printed is `[Circular]`. A handle index (`{ h }`) decodes to `undefined`,
 * as the run decodes it with no handles to hand.
 *
 * Neo's trace can cut a long value: an array or entry list then ends in `{ $more: N }`, and a
 * string in `…[+N bytes]`. Both print as they are, so a cut value reads as cut.
 */

const INDENT = '  '

/** `parseEvaluationResultValue`'s typed-array kinds. */
const TYPED_ARRAYS: Record<
  string,
  new (
    buffer: ArrayBufferLike,
  ) => ArrayLike<unknown>
> = {
  i8: Int8Array,
  ui8: Uint8Array,
  ui8c: Uint8ClampedArray,
  i16: Int16Array,
  ui16: Uint16Array,
  i32: Int32Array,
  ui32: Uint32Array,
  f32: Float32Array,
  f64: Float64Array,
  bi64: BigInt64Array,
  bui64: BigUint64Array,
}

type Node = Record<string, unknown>

function isRecord(value: unknown): value is Node {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The script's value in words: a string as its own text, anything else laid out. */
export function pageValueText(value: unknown): string {
  if (typeof value === 'string') return value
  const objects = new Map<number, Node>()
  collect(value, objects)
  return shown(value, '', objects, new Set())
}

/** Every object, array, Map and Set by its id, as the decoder registers them. */
function collect(value: unknown, objects: Map<number, Node>): void {
  if (Array.isArray(value)) {
    for (const item of value) collect(item, objects)
    return
  }
  if (!isRecord(value)) return
  if (typeof value.id === 'number') objects.set(value.id, value)
  for (const item of Object.values(value)) collect(item, objects)
}

function shown(
  value: unknown,
  indent: string,
  objects: Map<number, Node>,
  open: ReadonlySet<number>,
): string {
  if (value === undefined) return 'undefined'
  if (!isRecord(value)) return JSON.stringify(value) ?? String(value)
  if ('ref' in value) return reference(value, indent, objects, open)
  const scalar = scalarText(value)
  if (scalar !== null) return scalar
  const inner = indent + INDENT
  const within =
    typeof value.id === 'number' ? new Set(open).add(value.id) : open
  const show = (item: unknown) => shown(item, inner, objects, within)
  if (Array.isArray(value.a)) return block('[', ']', value.a.map(show), indent)
  if (Array.isArray(value.o)) {
    const key = (k: unknown) => JSON.stringify(String(k))
    return block('{', '}', entries(value.o, show, key, ':'), indent)
  }
  if (Array.isArray(value.m)) {
    return block('Map {', '}', entries(value.m, show, show, ' =>'), indent)
  }
  if (Array.isArray(value.s)) {
    return block('Set {', '}', value.s.map(show), indent)
  }
  // Not one of the page's forms: printed as recorded.
  return JSON.stringify(value, null, 2) ?? String(value)
}

/** `{ ref }`: the object it names again, or `[Circular]` while that object is still open. */
function reference(
  value: Node,
  indent: string,
  objects: Map<number, Node>,
  open: ReadonlySet<number>,
): string {
  const id = Number(value.ref)
  if (open.has(id)) return '[Circular]'
  const target = objects.get(id)
  return target ? shown(target, indent, objects, open) : 'undefined'
}

/** The forms that decode to one value: the special numbers, dates, errors, typed arrays. */
function scalarText(value: Node): string | null {
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
  if ('h' in value) return 'undefined'
  if ('fn' in value) return 'function'
  if ('ta' in value && isRecord(value.ta)) return typedArray(value.ta)
  if ('ab' in value && isRecord(value.ab)) {
    return `ArrayBuffer [${[...bytes(value.ab.b)].join(', ')}]`
  }
  if ('$more' in value) return `… ${String(value.$more)} more`
  return null
}

/** `base64ToTypedArray`: the bytes read as the named kind, shown as its numbers. */
function typedArray(ta: Node): string {
  const kind = String(ta.k)
  const Kind = TYPED_ARRAYS[kind]
  if (!Kind) return `${kind} [${String(ta.b)}]`
  const items = Array.from(new Kind(bytes(ta.b).buffer), (item) =>
    typeof item === 'bigint' ? `${item}n` : String(item),
  )
  return `${Kind.name} [${items.join(', ')}]`
}

/** The base64 text's bytes. */
function bytes(base64: unknown): Uint8Array {
  const binary = atob(String(base64))
  return Uint8Array.from(binary, (char) => char.charCodeAt(0))
}

/** `{ k, v }` entries as `key: value` (`key => value` in a Map). */
function entries(
  items: unknown[],
  show: (item: unknown) => string,
  key: (key: unknown) => string,
  arrow: string,
): string[] {
  return items.map((entry) =>
    isRecord(entry) && 'k' in entry
      ? `${key(entry.k)}${arrow} ${show(entry.v)}`
      : show(entry),
  )
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
