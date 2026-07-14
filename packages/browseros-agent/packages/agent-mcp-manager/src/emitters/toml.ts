import TOML from '@iarna/toml'

import type { McpServerSpec } from '../types'
import { buildEntryValue, type ResolvedShapes, transformKey } from './shape'

/**
 * TOML config emitter. Codex is currently the only TOML consumer in
 * the catalog. The @iarna/toml package does not preserve comments on
 * round-trip; entries hand-authored with comments will lose them if
 * the emitter rewrites the file.
 */

function parseDoc(raw: string): Record<string, unknown> {
  if (!raw.trim()) return {}
  return TOML.parse(raw) as Record<string, unknown>
}

function ensureMap(
  doc: Record<string, unknown>,
  tableKey: string,
): Record<string, unknown> {
  let table = doc[tableKey]
  if (!table || typeof table !== 'object') {
    table = {}
    doc[tableKey] = table
  }
  return table as Record<string, unknown>
}

export function tomlRead(raw: string, shapes: ResolvedShapes): string[] {
  const doc = parseDoc(raw)
  const table = doc[shapes.topLevelKey]
  if (!table || typeof table !== 'object') return []
  return Object.keys(table as Record<string, unknown>)
}

export function tomlAdd(
  raw: string,
  name: string,
  spec: McpServerSpec,
  shapes: ResolvedShapes,
): string {
  const doc = parseDoc(raw)
  const table = ensureMap(doc, shapes.topLevelKey)
  const key = transformKey(name, shapes.stdio)
  const value = buildEntryValue(spec, shapes)
  // Codex expects `http_headers` (not `headers`) for the header map on
  // http entries. `HttpShape.headerField` in the catalog handles that.
  table[key] = value
  return TOML.stringify(doc as TOML.JsonMap)
}

export function tomlRemove(
  raw: string,
  name: string,
  shapes: ResolvedShapes,
): string {
  if (!raw.trim()) return raw
  const doc = parseDoc(raw)
  const table = doc[shapes.topLevelKey]
  if (table && typeof table === 'object') {
    const t = table as Record<string, unknown>
    const key = transformKey(name, shapes.stdio)
    if (key in t) delete t[key]
    if (Object.keys(t).length === 0) delete doc[shapes.topLevelKey]
  }
  return TOML.stringify(doc as TOML.JsonMap)
}
