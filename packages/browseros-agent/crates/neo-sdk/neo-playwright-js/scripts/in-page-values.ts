/**
 * The value-format golden, crates/neo-sdk/neo-playwright/tests/data/in-page-values.json: for each
 * case, the text Playwright's in-page serializer writes for it as a result (`page`), what Rust's
 * parse writes back for the prelude (`result`), and the text upstream's client and server send
 * the page for it as an argument (`arg`, or the client's `argError`).
 *
 * `page` and `arg` come from the vendored modules themselves, run here in Bun: the serializer the
 * page runs (`serializeAsCallArgument`) and, for arguments, the client's `serializeValue`, the
 * server's `parseSerializedValue` and the serializer again. Rust's page_value tests parse and
 * re-emit every `page` text against `result`; the prelude's tests read `result` and `arg` back
 * through the built prelude. One file, so the three runtimes cannot drift.
 *
 * Sources build every value deterministically: an Error carries a fixed stack, because a real
 * one names this script's own path. Each source runs in this realm, whose classes the
 * serializer's `instanceof` checks see.
 */
import { runInThisContext } from 'node:vm'
import { serializeAsCallArgument } from '../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import {
  parseSerializedValue,
  serializeValue,
} from '../../neo-playwright/third_party/playwright/packages/protocol/src/serializers'

type Case = {
  name: string
  /** A JS expression: the value a page script returns, or an agent passes. */
  source: string
  /** What Rust's parse writes back; absent when it is `page` in canonical form. */
  result?: unknown
  /** The agent's realm cannot build this value (QuickJS has no `URL`): no `arg`. */
  pageOnly?: true
  /**
   * upstream's channel validator refuses this argument (`validatorPrimitives.ts`, measured in
   * tier 1's evaluate-args); this script runs the serializers but not the validator.
   */
  validatorError?: string
}

const CASES: Case[] = [
  { name: 'undefined', source: 'undefined' },
  { name: 'null', source: 'null' },
  { name: 'NaN', source: 'NaN' },
  { name: 'Infinity', source: 'Infinity' },
  { name: '-Infinity', source: '-Infinity' },
  { name: '-0', source: '-0' },
  { name: 'boolean', source: 'true' },
  { name: 'integer', source: '42' },
  { name: 'fraction', source: '1.5' },
  { name: 'string', source: "'plain text'" },
  { name: 'BigInt', source: '10n' },
  { name: 'negative BigInt', source: '-5n' },
  { name: 'Date', source: 'new Date(0)' },
  {
    name: 'invalid Date',
    source: "new Date('not a date')",
    validatorError: 'page.evaluate: arg.value.d: expected string, got object',
  },
  { name: 'RegExp', source: '/a+b/gi' },
  {
    name: 'Error',
    source:
      "Object.assign(new TypeError('no'), { stack: 'TypeError: no\\n    at page' })",
  },
  { name: 'Uint8Array', source: 'Uint8Array.of(1, 2, 255)' },
  { name: 'Float64Array', source: 'Float64Array.of(1.5, -2)' },
  { name: 'BigInt64Array', source: 'BigInt64Array.of(5n, -1n)' },
  {
    name: 'ArrayBuffer',
    source: 'Uint8Array.of(1, 2).buffer',
    result: { o: [], id: 1 },
  },
  { name: 'nested arrays', source: '[1, [2, [3]], []]' },
  { name: 'undefined in an array', source: '[undefined, 1]' },
  { name: 'object', source: "({ a: 1, b: { c: 'x' }, d: [true] })" },
  {
    name: 'cycle',
    source: '(() => { const o = { n: 1 }; o.self = o; return o })()',
  },
  {
    name: 'shared object',
    source: '(() => { const x = { y: 1 }; return [x, x] })()',
  },
  { name: 'Map', source: "new Map([['k', 1]])" },
  { name: 'Set', source: 'new Set([1])' },
  { name: 'symbol', source: "Symbol('s')" },
  { name: 'function in an array', source: '[() => 1, 2]' },
  { name: 'function in an object', source: '({ f: () => 1, n: 2 })' },
  { name: 'toJSON member', source: '({ toJSON() { return 1 }, n: 2 })' },
  {
    name: 'toJSON with no own keys',
    source: "Object.create({ toJSON() { return 'json' } })",
  },
  { name: '__proto__ key', source: 'JSON.parse(\'{"__proto__": 1, "n": 2}\')' },
  { name: 'URL', source: "new URL('https://a.example/p?q=1')", pageOnly: true },
]

const fallThrough = (value: unknown) => ({ fallThrough: value })

/** The page's result text: what `assets/main-world.js` stringifies. */
function pageText(value: unknown): string {
  return JSON.stringify(serializeAsCallArgument(value, fallThrough))
}

/** upstream's argument pipeline: client `serializeValue`, server parse, then the page format. */
function argText(value: unknown): { arg: string } | { argError: string } {
  try {
    const wire = serializeValue(value, fallThrough)
    const onServer: unknown = parseSerializedValue(wire, [])
    return {
      arg: JSON.stringify(serializeAsCallArgument(onServer, fallThrough)),
    }
  } catch (error) {
    return { argError: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * What Rust's parse makes of a page text's structure, which a case's `result` defaults to: its
 * ids renumbered from 1 in order of appearance and its refs rewritten to match, as upstream's
 * server writes them again. The serializer numbers objects that way already, except the ones
 * it writes with id 0 (an object whose `toJSON` is not used, say), which take their place in the
 * count.
 */
function canonical(page: unknown): unknown {
  const ids = new Map<number, number>()
  let last = 0
  const walk = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(walk)
    if (value === null || typeof value !== 'object') return value
    const tagged = value as Record<string, unknown>
    if (typeof tagged.ref === 'number') return { ref: ids.get(tagged.ref) }
    if ('id' in tagged && ('a' in tagged || 'o' in tagged)) {
      const id = ++last
      if (tagged.id !== 0) ids.set(tagged.id as number, id)
      if ('a' in tagged) return { a: walk(tagged.a), id }
      const members = tagged.o as { k: string; v?: unknown }[]
      return {
        o: members.map((member) =>
          'v' in member ? { k: member.k, v: walk(member.v) } : { k: member.k },
        ),
        id,
      }
    }
    return tagged
  }
  return walk(page)
}

/** The golden, as scripts/build.ts writes it. */
export function inPageValuesDocument(): string {
  const values = CASES.map(
    ({ name, source, result, pageOnly, validatorError }) => {
      const value: unknown = runInThisContext(`(${source})`)
      const page = pageText(value)
      const arg = validatorError ? { argError: validatorError } : argText(value)
      return {
        name,
        source,
        page,
        result: result ?? canonical(JSON.parse(page)),
        ...(pageOnly ? {} : arg),
      }
    },
  )
  return `${JSON.stringify(
    {
      about:
        'Playwright in-page values, what Rust writes back for each and their argument texts, written by crates/neo-sdk/neo-playwright-js/scripts/in-page-values.ts. Do not edit: run bun run build in crates/neo-sdk/neo-playwright-js.',
      values,
    },
    null,
    2,
  )}\n`
}
