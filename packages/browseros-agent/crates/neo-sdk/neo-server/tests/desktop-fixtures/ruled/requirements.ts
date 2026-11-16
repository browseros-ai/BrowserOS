import { readdir, readFile, realpath } from 'node:fs/promises'
import { join, relative, resolve, sep } from 'node:path'
import {
  bodyPaths,
  requireBodyPath,
  requireCondition,
  sha256,
} from '../cases/requirements'
import { requirePublicText, serializeJson } from '../cases/serialization'
import type { CaseSpec, RuledCaseSpec } from '../model'

// This digest locks approved original tuples independently of the editable rule records.
const identityDigest =
  'f85b7bb1c67f0ffe1ac17bfca03c1e10cdb8f262feef92957958835379555e96'

export function requireRuledBodyPath(path: string): void {
  requireCondition(
    /^ruled\/bodies\/(?:interactions\/|reads\/)?[a-z0-9-]+\.txt$/.test(path),
    `Invalid authored function-body path ${path}`,
  )
}

/** The authored scope is pass-only and disjoint from the immutable import, including aliases. */
export function validateRuledCases(
  rules: readonly RuledCaseSpec[],
  baseline: readonly CaseSpec[],
): void {
  requireCondition(
    rules.length === 44,
    'Expected exactly 44 authored SDK rulings',
  )
  const identities = rules.map(({ file, id, name }) =>
    JSON.stringify([file, id, name]),
  )
  requireCondition(
    sha256(`${identities.sort().join('\n')}\n`) === identityDigest,
    'Authored file, ID and original title identities differ from the approved 44',
  )
  const ids = new Set(baseline.map((spec) => spec.id))
  const importedPaths = new Set(
    baseline.flatMap((spec) => spec.steps.flatMap(bodyPaths)),
  )
  for (const spec of rules) {
    requireCondition(
      !ids.has(spec.id),
      `Duplicate expanded identity ${spec.id}`,
    )
    ids.add(spec.id)
    requireCondition(
      spec.file !== 'cases-click-recovery.ts',
      'Click-recovery differences remain excluded',
    )
    requireCondition(
      spec.file === `cases-${spec.id.replace(/-\d{3}$/, '')}.ts`,
      `ID/file mismatch for ${spec.id}`,
    )
    requireCondition(
      spec.expectation.kind === 'pass',
      `Authored SDK rule must pass: ${spec.id}`,
    )
    requireCondition(
      spec.proves.length > 0 && spec.ruling.sdkBehavior.length > 0,
      `Missing SDK intent for ${spec.id}`,
    )
    requireCondition(
      spec.ruling.citations.length > 0,
      `Missing public ruling citation for ${spec.id}`,
    )
    for (const citation of spec.ruling.citations) {
      requireCondition(
        /^[a-f0-9]{40}$/.test(citation.commit),
        `Unpinned ruling for ${spec.id}`,
      )
      requireCondition(
        /^(?:\d+-\d+)(?:,\d+-\d+)*$/.test(citation.lines),
        `Missing citation lines for ${spec.id}`,
      )
      requireCondition(
        citation.claim.length > 0,
        `Missing cited claim for ${spec.id}`,
      )
      requireCondition(
        /^packages\/[a-zA-Z0-9/_.-]+$/.test(citation.path) ||
          citation.path.startsWith(
            `https://github.com/microsoft/playwright/blob/${citation.commit}/`,
          ),
        `Ruling must cite public repository source for ${spec.id}`,
      )
    }
    requireCondition(
      spec.steps.length > 0,
      `Missing authored assertion for ${spec.id}`,
    )
    for (const path of spec.steps.flatMap(bodyPaths)) {
      if (path.startsWith('scripts/')) {
        requireBodyPath(path)
        requireCondition(
          importedPaths.has(path),
          `Unowned imported body ${path}`,
        )
      } else requireRuledBodyPath(path)
    }
    switch (spec.execution) {
      case 'ordinary':
        requireCondition(
          spec.hostSetup?.kind !== 'device-scale',
          `Ordinary rule cannot require DPR2: ${spec.id}`,
        )
        break
      case 'density':
        requireCondition(
          spec.id === 'screenshot-scale-001',
          'Only screenshot-scale-001 joins native DPR2',
        )
        break
      case 'terminal-select':
        requireCondition(
          spec.id === 'act-017' && spec.steps.length === 1,
          'Terminal select must be the single closed act-017 observation',
        )
        break
      default: {
        const exhaustive: never = spec
        throw new Error(`Unknown execution ${String(exhaustive)}`)
      }
    }
    requirePublicText(JSON.stringify(spec))
  }
  requireCondition(
    ids.size === 256,
    'Composed registry must contain 256 unique executions',
  )
  requireCondition(
    rules.filter((spec) => spec.execution === 'terminal-select').length === 1,
    'Expected one terminal observation',
  )
  requireCondition(
    rules.filter((spec) => spec.execution === 'density').length === 1,
    'Expected one additional DPR2 case',
  )
  const budget = rules.find((spec) => spec.id === 'read-eval-022')
  requireCondition(
    budget?.timeoutMs === 600_000 && budget.driverTimeoutMs === 20_000,
    'Budget probe must preserve SDK600000 with independent driver20000',
  )
  requireCondition(
    rules.filter((spec) => spec.driverTimeoutMs !== undefined).length === 1,
    'Only the budget probe changes the legacy driver budget',
  )
}

/** Authored tree equality and hashes fail closed on missing, unreferenced, or overwritten bodies. */
export async function validateRuledBodies(
  rules: readonly RuledCaseSpec[],
  suiteRoot: string,
): Promise<void> {
  const text = await readFile(join(suiteRoot, 'ruled/bodies.json'), 'utf8')
  const input: unknown = JSON.parse(text)
  requireCondition(
    Array.isArray(input),
    'Authored body inventory must be an array',
  )
  const entries = input.map((entry: unknown) => {
    if (
      entry === null ||
      typeof entry !== 'object' ||
      !('path' in entry) ||
      !('sha256' in entry)
    )
      throw new Error('Malformed authored body entry')
    requireCondition(
      Object.keys(entry).sort().join(',') === 'path,sha256',
      'Unexpected authored body fields',
    )
    requireCondition(
      typeof entry.path === 'string',
      'Authored path must be text',
    )
    requireRuledBodyPath(entry.path)
    requireCondition(
      typeof entry.sha256 === 'string' && /^[a-f0-9]{64}$/.test(entry.sha256),
      'Authored body digest must be SHA256',
    )
    return { path: entry.path, sha256: entry.sha256 }
  })
  requireCondition(
    text === serializeJson(entries),
    'Authored hash inventory must be canonical',
  )
  const referenced = [
    ...new Set(
      rules
        .flatMap((spec) => spec.steps.flatMap(bodyPaths))
        .filter((path) => path.startsWith('ruled/')),
    ),
  ].sort()
  requireCondition(
    JSON.stringify(entries.map((entry) => entry.path)) ===
      JSON.stringify(referenced),
    'Authored hashes differ from referenced bodies',
  )
  const found = (
    await readdir(join(suiteRoot, 'ruled/bodies'), {
      recursive: true,
      withFileTypes: true,
    })
  )
    .filter((entry) => entry.isFile() || entry.isSymbolicLink())
    .map((entry) => relative(suiteRoot, join(entry.parentPath, entry.name)))
    .sort()
  requireCondition(
    JSON.stringify(found) === JSON.stringify(referenced),
    'Authored body tree has missing or unowned files',
  )
  const root = await realpath(suiteRoot)
  const common = await readFile(join(suiteRoot, 'common.txt'), 'utf8')
  for (const entry of entries) {
    const path = resolve(suiteRoot, entry.path)
    requireCondition(
      (await realpath(path)).startsWith(`${root}${sep}`),
      `Authored body escapes suite: ${entry.path}`,
    )
    const bytes = await readFile(path)
    requireCondition(
      sha256(bytes) === entry.sha256,
      `Authored raw body hash differs: ${entry.path}`,
    )
    const body = bytes.toString('utf8')
    requirePublicText(body)
    Reflect.construct((async () => {}).constructor, [
      'args',
      `${common}\n${body}`,
    ])
  }
}
