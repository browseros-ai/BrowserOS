import { applyEdits, modify, parse } from 'jsonc-parser'

import type { McpServerSpec } from '../types'
import { buildEntryValue, type ResolvedShapes, transformKey } from './shape'

const FORMATTING = {
  formattingOptions: { tabSize: 2, insertSpaces: true },
} as const

/**
 * JSON / JSONC config emitter. Uses `jsonc-parser` to preserve
 * comments and formatting of the target file across edits.
 */

export function jsonRead(raw: string, shapes: ResolvedShapes): string[] {
  if (!raw.trim()) return []
  let parsed: unknown
  try {
    parsed = parse(raw)
  } catch {
    return []
  }
  if (!parsed || typeof parsed !== 'object') return []
  const container = (parsed as Record<string, unknown>)[shapes.topLevelKey]
  if (!container || typeof container !== 'object') return []
  return Object.keys(container as Record<string, unknown>)
}

export function jsonAdd(
  raw: string,
  name: string,
  spec: McpServerSpec,
  shapes: ResolvedShapes,
): string {
  const seed = raw.trim() ? raw : '{}'
  const key = transformKey(name, shapes.stdio)
  const value = buildEntryValue(spec, shapes)
  const edits = modify(seed, [shapes.topLevelKey, key], value, FORMATTING)
  return applyEdits(seed, edits)
}

export function jsonRemove(
  raw: string,
  name: string,
  shapes: ResolvedShapes,
): string {
  if (!raw.trim()) return raw
  const key = transformKey(name, shapes.stdio)
  const edits = modify(raw, [shapes.topLevelKey, key], undefined, FORMATTING)
  return applyEdits(raw, edits)
}
