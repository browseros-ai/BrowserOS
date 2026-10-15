/**
 * Resolves the authored split OpenAPI contract for fixture and live conformance.
 * OAS nullable/allOf is translated to JSON Schema before the real AJV validator
 * runs; no response values are coerced, defaulted, or stripped by these checks.
 */
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { parse } from 'yaml'

export type Node = Record<string, unknown>
export const contractDir = resolve(import.meta.dir, '..')
const documents = new Map<string, Node>()
export function record(value: unknown): Node {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`Expected an object, got ${JSON.stringify(value)}`)
  }
  return value as Node
}
export function document(file = 'openapi.yaml'): Node {
  const path = resolve(contractDir, file)
  if (!documents.has(path)) {
    documents.set(path, record(parse(readFileSync(path, 'utf8'))))
  }
  return documents.get(path) as Node
}
export function dereference(value: unknown, from: string): [Node, string] {
  const node = record(value)
  if (typeof node.$ref !== 'string') return [node, from]
  const [file, fragment = ''] = node.$ref.split('#')
  const target = file ? resolve(dirname(from), file) : from
  let result: unknown = document(target)
  for (const part of fragment.split('/').slice(1)) {
    result = record(result)[part.replaceAll('~1', '/').replaceAll('~0', '~')]
  }
  return dereference(result, target)
}
export function schemaFor(name: string): Node {
  const schema = record(record(document().components).schemas)[name]
  if (!schema) throw new Error(`Unknown schema: ${name}`)
  return jsonSchema(schema, join(contractDir, 'openapi.yaml'))
}
function jsonSchema(value: unknown, from: string): Node {
  const [node, origin] = dereference(value, from)
  const result: Node = {}
  for (const [key, item] of Object.entries(node)) {
    if (
      ['nullable', 'discriminator', 'example', 'xml', 'deprecated'].includes(
        key,
      )
    )
      continue
    if (['oneOf', 'anyOf', 'allOf'].includes(key)) {
      result[key] = (item as unknown[]).map((entry) =>
        jsonSchema(entry, origin),
      )
    } else if (key === 'properties') {
      result[key] = Object.fromEntries(
        Object.entries(record(item)).map(([field, shape]) => [
          field,
          jsonSchema(shape, origin),
        ]),
      )
    } else if (
      ['items', 'additionalProperties', 'not'].includes(key) &&
      typeof item === 'object' &&
      item !== null
    ) {
      result[key] = jsonSchema(item, origin)
    } else result[key] = item
  }
  return node.nullable ? { anyOf: [result, { type: 'null' }] } : result
}
// AJV is already pinned in the generator's dependency graph. Resolve it through
// openapi-typescript, so conformance needs no unrelated root manifest changes.
const generatorRequire = createRequire(
  import.meta.resolve('openapi-typescript'),
)
const redoclyRequire = createRequire(
  generatorRequire.resolve('@redocly/openapi-core'),
)
interface Validation {
  (value: unknown): boolean
  errors?: unknown
}
interface Validator {
  addFormat(name: string, validate: (value: string) => boolean): void
  compile(schema: unknown): Validation
  errorsText(errors: unknown): string
}
const Ajv = redoclyRequire('@redocly/ajv').default as new (
  options: Node,
) => Validator
const validator = new Ajv({
  allErrors: true,
  strict: false,
  validateFormats: true,
})
// Match zod 4's UUID acceptance (including the nil/max UUIDs); URL uses the
// same WHATWG parser as z.string().url(), rather than an http-only policy.
validator.addFormat('uuid', (value) =>
  /^([0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}|0{8}-0{4}-0{4}-0{4}-0{12}|f{8}-f{4}-f{4}-f{4}-f{12})$/i.test(
    value,
  ),
)
validator.addFormat('uri', (value) => URL.canParse(value))
const checks = new Map<string, Validation>()
export function assertSchema(name: string, value: unknown): void {
  let check = checks.get(name)
  if (!check) {
    check = validator.compile(schemaFor(name))
    checks.set(name, check)
  }
  if (!check(value))
    throw new Error(`${name}: ${validator.errorsText(check.errors)}`)
}
export function pathItem(path: string): [Node, string] {
  return dereference(
    record(document().paths)[path],
    join(contractDir, 'openapi.yaml'),
  )
}
export function operation(path: string, method: string): [Node, string] {
  const [item, origin] = pathItem(path)
  return [record(item[method]), origin]
}
export function assertResponse(
  path: string,
  method: string,
  status: number,
  value: unknown,
): void {
  const [op, origin] = operation(path, method)
  const response = record(op.responses)[status]
  if (!response)
    throw new Error(`Undocumented response ${method} ${path}: ${status}`)
  const [shape, from] = dereference(response, origin)
  const json = record(record(shape.content)['application/json'])
  const check = validator.compile(jsonSchema(json.schema, from))
  if (!check(value))
    throw new Error(
      `${method} ${path} ${status}: ${validator.errorsText(check.errors)}`,
    )
}
