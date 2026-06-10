import { resolveRef } from './naming'
import type { ProtocolProperty, ProtocolType } from './protocol-parser'

const PRIMITIVE_MAP: Record<string, string> = {
  string: 'string',
  integer: 'number',
  number: 'number',
  boolean: 'boolean',
  any: 'unknown',
  binary: 'string',
  object: 'Record<string, unknown>',
}

export interface ImportRef {
  domain: string
  typeName: string
}

export type RefResolver = (ref: string) => string

export function collectImports(
  items: Array<{
    $ref?: string
    items?: { $ref?: string }
    properties?: ProtocolProperty[]
  }>,
  currentDomain: string,
): ImportRef[] {
  const refs: ImportRef[] = []

  for (const item of items) {
    if (item.$ref) {
      const resolved = resolveRef(item.$ref)
      if (resolved.domain && resolved.domain !== currentDomain) {
        refs.push({ domain: resolved.domain, typeName: resolved.typeName })
      }
    }
    if (item.items?.$ref) {
      const resolved = resolveRef(item.items.$ref)
      if (resolved.domain && resolved.domain !== currentDomain) {
        refs.push({ domain: resolved.domain, typeName: resolved.typeName })
      }
    }
    if (item.properties) {
      refs.push(...collectImports(item.properties, currentDomain))
    }
  }

  return refs
}

function resolveTypeExpression(
  prop: {
    type?: string
    $ref?: string
    enum?: string[]
    items?: { type?: string; $ref?: string }
    properties?: ProtocolProperty[]
  },
  resolve: RefResolver,
): string {
  if (prop.$ref) return resolve(prop.$ref)

  if (prop.type === 'string' && prop.enum) {
    return prop.enum.map((v) => `'${v}'`).join(' | ')
  }

  if (prop.type === 'array') {
    if (prop.items) {
      const inner = resolveTypeExpression(prop.items, resolve)
      return inner.includes('|') ? `(${inner})[]` : `${inner}[]`
    }
    return 'unknown[]'
  }

  if (prop.type === 'object' && prop.properties && prop.properties.length > 0) {
    const fields = prop.properties.map((p) => {
      const opt = p.optional ? '?' : ''
      const ts = resolveTypeExpression(p, resolve)
      return `    ${p.name}${opt}: ${ts}`
    })
    return `{\n${fields.join('\n')}\n  }`
  }

  if (prop.type && PRIMITIVE_MAP[prop.type]) {
    return PRIMITIVE_MAP[prop.type]
  }

  return 'unknown'
}

export function emitType(type: ProtocolType, resolve: RefResolver): string {
  if (type.type === 'object' && type.properties && type.properties.length > 0) {
    return emitInterface(type.id, type.properties, resolve)
  }

  if (type.type === 'string' && type.enum) {
    const members = type.enum.map((v) => `  | '${v}'`).join('\n')
    return `export type ${type.id} =\n${members}\n`
  }

  if (type.type === 'array' && type.items) {
    const inner = resolveTypeExpression(type.items, resolve)
    return `export type ${type.id} = ${inner}[]\n`
  }

  const ts = PRIMITIVE_MAP[type.type] ?? 'unknown'
  return `export type ${type.id} = ${ts}\n`
}

export function emitInterface(
  name: string,
  properties: ProtocolProperty[],
  resolve: RefResolver,
): string {
  const lines: string[] = [`export interface ${name} {`]

  for (const prop of properties) {
    const opt = prop.optional ? '?' : ''
    const ts = resolveTypeExpression(prop, resolve)
    lines.push(`  ${prop.name}${opt}: ${ts}`)
  }

  lines.push('}\n')
  return lines.join('\n')
}
