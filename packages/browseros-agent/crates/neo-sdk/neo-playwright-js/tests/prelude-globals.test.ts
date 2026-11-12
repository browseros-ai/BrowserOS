// The prelude asset runs in claw-sdk's QuickJS runtime, which has ECMAScript's own globals
// plus neo-sdk's URL/base64 interfaces. It has no Intl, structuredClone, process or Buffer.
// This reads every global the bundle names, used or not, so a code path that runs only on
// some input cannot slip one in.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { type Node as AstNode, parse } from 'acorn'

/** The global object's properties in ECMA-262, which QuickJS implements. Intl is ECMA-402. */
const ECMASCRIPT_GLOBALS = new Set([
  'globalThis',
  'Infinity',
  'NaN',
  'undefined',
  'eval',
  'isFinite',
  'isNaN',
  'parseFloat',
  'parseInt',
  'decodeURI',
  'decodeURIComponent',
  'encodeURI',
  'encodeURIComponent',
  'escape',
  'unescape',
  'AggregateError',
  'Array',
  'ArrayBuffer',
  'Atomics',
  'BigInt',
  'BigInt64Array',
  'BigUint64Array',
  'Boolean',
  'DataView',
  'Date',
  'Error',
  'EvalError',
  'FinalizationRegistry',
  'Float32Array',
  'Float64Array',
  'Function',
  'Int8Array',
  'Int16Array',
  'Int32Array',
  'Iterator',
  'JSON',
  'Map',
  'Math',
  'Number',
  'Object',
  'Promise',
  'Proxy',
  'RangeError',
  'ReferenceError',
  'Reflect',
  'RegExp',
  'Set',
  'SharedArrayBuffer',
  'String',
  'Symbol',
  'SyntaxError',
  'TypeError',
  'Uint8Array',
  'Uint8ClampedArray',
  'Uint16Array',
  'Uint32Array',
  'URIError',
  'WeakMap',
  'WeakRef',
  'WeakSet',
])

// These additional free names are pinned by neo-sdk's real QuickJS realm tests.
const REALM_GLOBALS = new Set(['URL', 'atob', 'btoa'])

type Visit = (node: AstNode, parent: AstNode | undefined, key: string) => void

function walk(node: AstNode, visit: Visit, parent?: AstNode, key = ''): void {
  visit(node, parent, key)
  for (const [childKey, child] of Object.entries(node)) {
    const children = Array.isArray(child) ? child : [child]
    for (const item of children)
      if (isNode(item)) walk(item, visit, node, childKey)
  }
}

function isNode(value: unknown): value is AstNode {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof Reflect.get(value, 'type') === 'string'
  )
}

const field = (node: AstNode, name: string): unknown => Reflect.get(node, name)
const list = (node: AstNode, name: string): unknown[] => {
  const value = field(node, name)
  return Array.isArray(value) ? value : []
}

/** Names a pattern binds: `a`, `{ a, b: [c] }`, `...d`, `e = 1`. */
function bound(pattern: unknown, into: Set<string>): void {
  if (!isNode(pattern)) return
  switch (pattern.type) {
    case 'Identifier':
      into.add(String(field(pattern, 'name')))
      return
    case 'ObjectPattern':
      for (const property of list(pattern, 'properties'))
        bound(
          isNode(property) && property.type === 'RestElement'
            ? field(property, 'argument')
            : isNode(property) && field(property, 'value'),
          into,
        )
      return
    case 'ArrayPattern':
      for (const element of list(pattern, 'elements')) bound(element, into)
      return
    case 'RestElement':
      bound(field(pattern, 'argument'), into)
      return
    case 'AssignmentPattern':
      bound(field(pattern, 'left'), into)
      return
  }
}

/**
 * Identifiers the bundle reads that nothing in it declares. Declarations are taken from the
 * whole file rather than per scope: a bundle's locals do not shadow host globals by accident,
 * and over-approximating bindings can only hide a global, never invent one.
 */
function freeIdentifiers(source: string): Set<string> {
  const program = parse(source, { ecmaVersion: 'latest', sourceType: 'script' })
  const declared = new Set<string>(['arguments'])
  const read = new Set<string>()
  walk(program, (node, parent, key) => {
    if (node.type === 'VariableDeclarator') bound(field(node, 'id'), declared)
    if (
      node.type === 'FunctionDeclaration' ||
      node.type === 'FunctionExpression' ||
      node.type === 'ArrowFunctionExpression'
    ) {
      bound(field(node, 'id'), declared)
      for (const param of list(node, 'params')) bound(param, declared)
    }
    if (node.type === 'ClassDeclaration' || node.type === 'ClassExpression')
      bound(field(node, 'id'), declared)
    if (node.type === 'CatchClause') bound(field(node, 'param'), declared)
    if (node.type !== 'Identifier' || !parent) return
    const computed = field(parent, 'computed') === true
    const notAReference =
      (parent.type === 'MemberExpression' && key === 'property' && !computed) ||
      ((parent.type === 'Property' ||
        parent.type === 'MethodDefinition' ||
        parent.type === 'PropertyDefinition') &&
        key === 'key' &&
        !computed) ||
      ((parent.type === 'LabeledStatement' ||
        parent.type === 'BreakStatement' ||
        parent.type === 'ContinueStatement') &&
        key === 'label') ||
      parent.type === 'MetaProperty'
    if (!notAReference) read.add(String(field(node, 'name')))
  })
  return new Set([...read].filter((name) => !declared.has(name)))
}

it('tests that the prelude asset names only globals provided by the production QuickJS realm', () => {
  const prelude = readFileSync(
    new URL('../../neo-playwright/assets/prelude.js', import.meta.url),
    'utf8',
  )
  const hostGlobals = [...freeIdentifiers(prelude)].filter(
    (name) => !ECMASCRIPT_GLOBALS.has(name) && !REALM_GLOBALS.has(name),
  )
  expect(hostGlobals).toEqual([])
})

it('tests that the globals check sees a host global the prelude must not use', () => {
  const free = freeIdentifiers(
    `(() => { const local = 1; return [new URL('a:b'), Intl.DateTimeFormat, structuredClone({}), process.env, Buffer.from(''), local.x, ({ key: 1 }).key] })()`,
  )
  expect(
    [...free].filter((name) => !ECMASCRIPT_GLOBALS.has(name)).sort(),
  ).toEqual(['Buffer', 'Intl', 'URL', 'process', 'structuredClone'])
})
