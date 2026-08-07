import YAML from 'yaml'

import type { McpServerSpec } from '../types'
import { buildEntryValue, type ResolvedShapes, transformKey } from './shape'

/**
 * YAML config emitter. Uses the `yaml` package which round-trips
 * comments and formatting through a Document API. We keep the whole
 * file as a Document object so hand-authored comments upstream survive
 * our edits.
 */

function parseDoc(raw: string): YAML.Document {
  if (!raw.trim()) return new YAML.Document({}, { keepSourceTokens: true })
  return YAML.parseDocument(raw, { keepSourceTokens: true })
}

export function yamlRead(raw: string, shapes: ResolvedShapes): string[] {
  if (!raw.trim()) return []
  const doc = parseDoc(raw)
  const container = doc.get(shapes.topLevelKey)
  if (!YAML.isMap(container)) return []
  return container.items
    .map((item) =>
      YAML.isScalar(item.key) ? String(item.key.value) : String(item.key),
    )
    .filter((k) => k.length > 0)
}

export function yamlAdd(
  raw: string,
  name: string,
  spec: McpServerSpec,
  shapes: ResolvedShapes,
): string {
  const doc = parseDoc(raw)
  const key = transformKey(name, shapes.stdio)
  const value = buildEntryValue(spec, shapes)
  const container = doc.get(shapes.topLevelKey)
  if (YAML.isMap(container)) {
    container.set(key, value)
  } else {
    doc.set(shapes.topLevelKey, { [key]: value })
  }
  return String(doc)
}

export function yamlRemove(
  raw: string,
  name: string,
  shapes: ResolvedShapes,
): string {
  if (!raw.trim()) return raw
  const doc = parseDoc(raw)
  const key = transformKey(name, shapes.stdio)
  const container = doc.get(shapes.topLevelKey)
  if (YAML.isMap(container)) {
    container.delete(key)
    if (container.items.length === 0) doc.delete(shapes.topLevelKey)
  }
  return String(doc)
}
