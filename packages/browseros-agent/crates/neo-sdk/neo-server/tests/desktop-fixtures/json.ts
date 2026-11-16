import type { JsonObject, JsonValue } from './model'

/** JSON crosses the process and host boundaries here; internal code only sees JSON values. */
export function jsonValue(value: unknown): JsonValue {
  if (
    value === null ||
    typeof value === 'string' ||
    typeof value === 'boolean'
  ) {
    return value
  }
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (Array.isArray(value)) {
    return value.map((item: unknown) => jsonValue(item))
  }
  if (typeof value === 'object') {
    const result: JsonObject = {}
    for (const [key, child] of Object.entries(value)) {
      result[key] = jsonValue(child)
    }
    return result
  }
  throw new Error(`Expected JSON value, received ${typeof value}`)
}

export function jsonObject(value: unknown, label: string): JsonObject {
  if (value === null || Array.isArray(value) || typeof value !== 'object') {
    throw new Error(`${label} must be a JSON object`)
  }
  const parsed = jsonValue(value)
  if (parsed === null || Array.isArray(parsed) || typeof parsed !== 'object') {
    throw new Error(`${label} must be a JSON object`)
  }
  return parsed
}

export function stringField(object: JsonObject, key: string): string {
  const value = object[key]
  if (typeof value !== 'string') throw new Error(`${key} must be a string`)
  return value
}

export function numberField(object: JsonObject, key: string): number {
  const value = object[key]
  if (typeof value !== 'number') throw new Error(`${key} must be a number`)
  return value
}

/** Canonical key order permits exact JSON comparisons without weakening array order. */
export function canonicalJson(value: JsonValue): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value)
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
    .join(',')}}`
}
