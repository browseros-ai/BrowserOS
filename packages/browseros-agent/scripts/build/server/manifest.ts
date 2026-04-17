import { readFileSync } from 'node:fs'

import type { BuildTarget, ResourceManifest, ResourceRule } from './types'

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
  }
  if (isStringArray(item.os)) {
    rule.os = item.os as ResourceRule['os']
  }
  if (isStringArray(item.arch)) {
    rule.arch = item.arch as ResourceRule['arch']
  }
  validateRule(rule)
  return rule
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
