import { readdir, readFile, realpath } from 'node:fs/promises'
import { join, relative, resolve, sep } from 'node:path'
import { actCases } from './cases/act'
import { captureIoCases } from './cases/capture-io'
import { clickRecoveryCases } from './cases/click-recovery'
import { fillCases } from './cases/fill'
import { frameRefsCases } from './cases/frame-refs'
import { isolationCases } from './cases/isolation'
import { labelsCases } from './cases/labels'
import { navigateSnapshotCases } from './cases/navigate-snapshot'
import { outputsCases } from './cases/outputs'
import { readEvalCases } from './cases/read-eval'
import {
  bodyPaths,
  expectedFamilyCounts,
  requireBodyPath,
  requireCondition,
  sha256,
  validateCaseContract,
} from './cases/requirements'
import { screenshotScaleCases } from './cases/screenshot-scale'
import { sdkInputCases } from './cases/sdk-input'
import { sdkNavigationCases } from './cases/sdk-navigation'
import { sdkReadEvalCases } from './cases/sdk-read-eval'
import {
  familyModule,
  requirePublicText,
  serializeFamily,
  serializeJson,
} from './cases/serialization'
import { snapshotConcurrencyCases } from './cases/snapshot-concurrency'
import { tabsCases } from './cases/tabs'
import type { CaseSpec, ExecutableCase } from './model'
import { ruledCases } from './ruled/cases'
import {
  requireRuledBodyPath,
  validateRuledBodies,
  validateRuledCases,
} from './ruled/requirements'

/** Frozen import ownership: serialization and raw bytes are checked independently of authored rules. */
export const baselineCases: readonly CaseSpec[] = [
  ...actCases,
  ...captureIoCases,
  ...clickRecoveryCases,
  ...fillCases,
  ...frameRefsCases,
  ...isolationCases,
  ...labelsCases,
  ...navigateSnapshotCases,
  ...outputsCases,
  ...readEvalCases,
  ...screenshotScaleCases,
  ...sdkInputCases,
  ...sdkNavigationCases,
  ...sdkReadEvalCases,
  ...snapshotConcurrencyCases,
  ...tabsCases,
]

/** Compose owners once; every ordinary case thereafter follows the same strict Harness. */
export const cases: readonly ExecutableCase[] = [
  ...baselineCases.map(
    (spec) =>
      ({
        ...spec,
        execution:
          spec.hostSetup?.kind === 'device-scale' ? 'density' : 'ordinary',
      }) satisfies ExecutableCase,
  ),
  ...ruledCases,
].sort((a, b) => a.file.localeCompare(b.file) || a.id.localeCompare(b.id))

export interface BodyInventory {
  commonSha256: string
  bodies: { path: string; sha256: string }[]
}

const suiteRoot = import.meta.dir

export async function loadBody(script: string): Promise<string> {
  if (script.startsWith('ruled/')) requireRuledBodyPath(script)
  else requireBodyPath(script)
  return readFile(join(suiteRoot, script), 'utf8')
}

export async function loadCommon(): Promise<string> {
  return readFile(join(suiteRoot, 'common.txt'), 'utf8')
}

function digest(value: unknown): string {
  requireCondition(
    typeof value === 'string' && /^[a-f0-9]{64}$/.test(value),
    'Body digests must be SHA-256 hex strings',
  )
  return value
}

function parseBodyInventory(value: unknown): BodyInventory {
  if (
    value === null ||
    typeof value !== 'object' ||
    !('commonSha256' in value) ||
    !('bodies' in value)
  )
    throw new Error('Malformed body inventory')
  requireCondition(
    Object.keys(value).sort().join(',') === 'bodies,commonSha256',
    'Unexpected body inventory fields',
  )
  const { commonSha256, bodies } = value
  requireCondition(
    Array.isArray(bodies),
    'Body inventory must list paths and raw hashes',
  )
  return {
    commonSha256: digest(commonSha256),
    bodies: bodies.map((entry: unknown) => {
      if (
        entry === null ||
        typeof entry !== 'object' ||
        !('path' in entry) ||
        !('sha256' in entry)
      )
        throw new Error('Malformed body entry')
      requireCondition(
        Object.keys(entry).sort().join(',') === 'path,sha256',
        'Unexpected body entry fields',
      )
      requireCondition(
        typeof entry.path === 'string',
        'Body path must be a string',
      )
      requireBodyPath(entry.path)
      return { path: entry.path, sha256: digest(entry.sha256) }
    }),
  }
}

async function readContained(path: string): Promise<Buffer> {
  const absolute = resolve(suiteRoot, path)
  requireCondition(
    (await realpath(absolute)).startsWith(`${await realpath(suiteRoot)}${sep}`),
    `Body resolves outside suite: ${path}`,
  )
  return readFile(absolute)
}

/** Validates committed files only. Parsing AsyncFunction bodies never evaluates fixture code. */
export async function validateInventory(): Promise<void> {
  validateCaseContract(baselineCases)
  validateRuledCases(ruledCases, baselineCases)
  await validateRuledBodies(ruledCases, suiteRoot)
  for (const file of Object.keys(expectedFamilyCounts)) {
    const content = await readFile(
      join(suiteRoot, familyModule(file).path),
      'utf8',
    )
    requireCondition(
      content ===
        serializeFamily(
          file,
          baselineCases.filter((entry) => entry.file === file),
        ),
      `Generated registry differs for ${file}; rerun the explicit importer`,
    )
    requirePublicText(content)
  }
  const indexText = await readFile(join(suiteRoot, 'bodies.json'), 'utf8')
  const index: unknown = JSON.parse(indexText)
  const inventory = parseBodyInventory(index)
  requireCondition(
    indexText === serializeJson(inventory),
    'Body inventory is not canonically serialized',
  )
  requireCondition(inventory.bodies.length === 155, 'Expected 155 body hashes')
  requireCondition(
    new Set(inventory.bodies.map((entry) => entry.sha256)).size === 155,
    'Every imported body must have a distinct raw hash',
  )
  const referenced = [
    ...new Set(
      baselineCases.flatMap((entry) => entry.steps.flatMap(bodyPaths)),
    ),
  ].sort()
  requireCondition(
    JSON.stringify(inventory.bodies.map((entry) => entry.path)) ===
      JSON.stringify(referenced),
    'Body hash inventory differs from referenced ordered bodies',
  )
  const found = (
    await readdir(join(suiteRoot, 'scripts'), {
      recursive: true,
      withFileTypes: true,
    })
  )
    .filter((entry) => entry.isFile() || entry.isSymbolicLink())
    .map((entry) => relative(suiteRoot, join(entry.parentPath, entry.name)))
    .sort()
  requireCondition(
    JSON.stringify(found) === JSON.stringify(referenced),
    'Unreferenced or missing files in body tree',
  )
  const common = await readContained('common.txt')
  requireCondition(
    sha256(common) === inventory.commonSha256,
    'Shared assertion helper raw hash differs',
  )
  requirePublicText(common.toString('utf8'))
  for (const entry of inventory.bodies) {
    const bytes = await readContained(entry.path)
    requireCondition(
      sha256(bytes) === entry.sha256,
      `Raw body hash differs for ${entry.path}`,
    )
    const body = bytes.toString('utf8')
    requirePublicText(body)
    // Constructing an AsyncFunction compiles its body but does not run browser actions or assertions.
    Reflect.construct((async () => {}).constructor, [
      'args',
      `${common.toString('utf8')}\n${body}`,
    ])
  }
}
