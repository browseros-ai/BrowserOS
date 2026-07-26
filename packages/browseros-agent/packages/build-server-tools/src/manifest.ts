import { readFileSync } from 'node:fs'

import type {
  BuildTarget,
  ResourceManifest,
  ResourceRule,
  TargetArch,
  TargetOs,
} from './types'

const TARGET_OS_VALUES = new Set<TargetOs>(['linux', 'macos', 'windows'])
const TARGET_ARCH_VALUES = new Set<TargetArch>(['x64', 'arm64'])

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((entry) => typeof entry === 'string')
  )
}

function validateRule(rule: ResourceRule): void {
  if (!rule.name || rule.name.trim().length === 0) {
    throw new Error('Manifest rule is missing name')
  }
  const hasSourcePath =
    (rule.source.type === 'r2' && rule.source.key) ||
    (rule.source.type === 'local' && rule.source.path)
  if (!hasSourcePath || !rule.destination) {
    throw new Error(
      `Manifest rule ${rule.name} is missing source path or destination`,
    )
  }
  if (rule.recursive && rule.source.type !== 'local') {
    throw new Error(
      `Manifest rule ${rule.name} uses recursive with non-local source`,
    )
  }
}

function parseSource(raw: unknown): ResourceRule['source'] {
  if (typeof raw !== 'object' || raw === null) {
    throw new Error('Manifest source must be an object')
  }
  const source = raw as Record<string, unknown>
  if (source.type === 'r2') {
    const key = source.key
    if (typeof key !== 'string' || key.length === 0) {
      throw new Error('Manifest source key is required')
    }
    return { type: 'r2', key }
  }
  if (source.type === 'local') {
    const path = source.path
    if (typeof path !== 'string' || path.length === 0) {
      throw new Error('Manifest source path is required')
    }
    return { type: 'local', path }
  }
  throw new Error(`Unsupported source type in manifest: ${String(source.type)}`)
}

function parseRule(raw: unknown): ResourceRule {
  if (typeof raw !== 'object' || raw === null) {
    throw new Error('Manifest contains an invalid rule entry')
  }
  const item = raw as Record<string, unknown>
  const rule: ResourceRule = {
    name: String(item.name ?? ''),
    source: parseSource(item.source),
    destination: String(item.destination ?? ''),
    executable: item.executable === true,
    recursive: item.recursive === true,
  }
  if (isStringArray(item.os)) {
    validateStringUnion(item.os, TARGET_OS_VALUES, 'os')
    rule.os = item.os as ResourceRule['os']
  }
  if (isStringArray(item.arch)) {
    validateStringUnion(item.arch, TARGET_ARCH_VALUES, 'arch')
    rule.arch = item.arch as ResourceRule['arch']
  }
  validateRule(rule)
  return rule
}

function validateStringUnion<T extends string>(
  values: string[],
  allowed: Set<T>,
  field: string,
): void {
  const invalid = values.filter((value) => !allowed.has(value as T))
  if (invalid.length > 0) {
    throw new Error(
      `Manifest rule has invalid ${field} value(s): ${invalid.join(', ')}`,
    )
  }
}

export function loadManifest(path: string): ResourceManifest {
  const raw = JSON.parse(readFileSync(path, 'utf-8')) as Record<string, unknown>
  if (!Array.isArray(raw.resources)) {
    throw new Error(`Manifest is missing resources array: ${path}`)
  }
  return {
    resources: raw.resources.map((entry) => parseRule(entry)),
  }
}

export function getTargetRules(
  manifest: ResourceManifest,
  target: BuildTarget,
): ResourceRule[] {
  return manifest.resources.filter((rule) => {
    if (rule.os && !rule.os.includes(target.os)) {
      return false
    }
    if (rule.arch && !rule.arch.includes(target.arch)) {
      return false
    }
    return true
  })
}
