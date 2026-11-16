import { mkdir, readFile, realpath, writeFile } from 'node:fs/promises'
import { hostname } from 'node:os'
import { dirname, join, resolve, sep } from 'node:path'
import { historicalFailures } from './cases/history'
import {
  parseManifestCases,
  parseOriginalCases,
  parseSavedStatuses,
  type SavedStatus,
} from './cases/import-source'
import {
  bodyPaths,
  expectedFamilyCounts,
  requireCondition,
  sha256,
  validateCaseContract,
} from './cases/requirements'
import {
  familyModule,
  requirePublicText,
  serializeFamily,
  serializeJson,
} from './cases/serialization'
import type { BodyInventory } from './inventory'
import type { CaseSpec } from './model'

const manifestSizes: Readonly<Record<string, number>> = {
  'g1a.json': 50,
  'g1b.json': 70,
  'g1c.json': 44,
  'g2.json': 67,
  'g3.json': 109,
  'g4.json': 19,
}

const savedCounts: Readonly<Record<SavedStatus, number>> = {
  PASS: 206,
  FAIL: 6,
  'DIFFERENT BY DESIGN': 90,
  'NO EQUIVALENT': 13,
  'NOT A BROWSER BEHAVIOUR': 44,
}

async function readInput(root: string, path: string): Promise<Buffer> {
  const absolute = await realpath(join(root, path))
  requireCondition(
    absolute.startsWith(`${root}${sep}`),
    `Input resolves outside bundle: ${path}`,
  )
  return readFile(absolute)
}

async function readJson(root: string, path: string): Promise<unknown> {
  return JSON.parse((await readInput(root, path)).toString('utf8'))
}

/** Import callers supply host identity explicitly; committed offline checks never consult that host. */
export async function prepareImport(
  input: string,
  context: { importingHost: string },
): Promise<ReadonlyMap<string, Uint8Array>> {
  const root = await realpath(resolve(input))
  const originals = parseOriginalCases(await readJson(root, 'inventory.json'))
  const originalById = new Map(originals.map((entry) => [entry.id, entry]))
  requireCondition(
    originals.length === 359 && originalById.size === 359,
    'Original inventory must have 359 unique expanded IDs',
  )
  const statuses = parseSavedStatuses(
    await readJson(root, 'results/index.json'),
  )
  requireCondition(
    statuses.size === 359,
    'Saved result index must cover exactly 359 IDs',
  )
  for (const id of statuses.keys())
    requireCondition(originalById.has(id), `Unknown saved result ID ${id}`)
  for (const [status, count] of Object.entries(savedCounts)) {
    requireCondition(
      [...statuses.values()].filter((value) => value === status).length ===
        count,
      `Saved ${status} count differs from ${count}`,
    )
  }
  const failedIds = [...historicalFailures].sort()
  requireCondition(
    JSON.stringify(
      [...statuses]
        .filter(([, status]) => status === 'FAIL')
        .map(([id]) => id)
        .sort(),
    ) === JSON.stringify(failedIds),
    'Saved failures must equal the six known source contracts',
  )
  const allIds = new Set<string>()
  const selected: CaseSpec[] = []
  for (const [file, count] of Object.entries(manifestSizes)) {
    const rows = parseManifestCases(
      await readJson(root, `manifests/${file}`),
      originalById,
    )
    requireCondition(
      rows.length === count,
      `Manifest ${file} must have ${count} rows`,
    )
    for (const row of rows) {
      requireCondition(!allIds.has(row.id), `Duplicate manifest ID ${row.id}`)
      allIds.add(row.id)
      const status = statuses.get(row.id)
      if (row.kind === 'classified') {
        requireCondition(
          status === row.classification,
          `Saved classification differs for ${row.id}`,
        )
      } else {
        requireCondition(
          status === 'PASS' ||
            (status === 'FAIL' && failedIds.includes(row.id)),
          `Selected ${row.id} was not a measured pass or known failure`,
        )
        selected.push(row.case)
      }
    }
  }
  requireCondition(
    allIds.size === originals.length,
    'Manifests must cover every original ID exactly once',
  )
  selected.sort(
    (a, b) => a.file.localeCompare(b.file) || a.id.localeCompare(b.id),
  )
  validateCaseContract(selected)
  const output = new Map<string, Uint8Array>()
  const emit = (path: string, content: string | Uint8Array) => {
    const bytes = typeof content === 'string' ? Buffer.from(content) : content
    requirePublicText(Buffer.from(bytes).toString('utf8'), {
      privateRoot: root,
      importingHost: context.importingHost,
    })
    output.set(path, bytes)
  }
  for (const file of Object.keys(expectedFamilyCounts)) {
    emit(
      familyModule(file).path,
      serializeFamily(
        file,
        selected.filter((entry) => entry.file === file),
      ),
    )
  }
  const common = await readInput(root, 'common.js')
  emit('common.txt', common)
  const inventory: BodyInventory = { commonSha256: sha256(common), bodies: [] }
  const paths = [
    ...new Set(selected.flatMap((entry) => entry.steps.flatMap(bodyPaths))),
  ].sort()
  for (const path of paths) {
    const bytes = await readInput(root, path.replace(/\.txt$/, '.js'))
    // This compiles the same async function body sent to run(), without calling it on the host.
    Reflect.construct((async () => {}).constructor, [
      'args',
      `${common.toString('utf8')}\n${bytes.toString('utf8')}`,
    ])
    emit(path, bytes)
    inventory.bodies.push({ path, sha256: sha256(bytes) })
  }
  requireCondition(
    new Set(inventory.bodies.map((entry) => entry.sha256)).size === 155,
    'Imported bodies must have 155 distinct raw hashes',
  )
  emit('bodies.json', serializeJson(inventory))
  return output
}

async function main(args: string[]): Promise<void> {
  if (args.length === 1 && args[0] === '--check') {
    // Offline validation imports only committed modules and never consults an external directory.
    const { validateInventory } = await import('./inventory')
    await validateInventory()
    process.stdout.write(
      'Validated 212 desktop contracts and 155 function bodies.\n',
    )
    return
  }
  const [input] = args
  requireCondition(
    args.length === 1 && input && !input.startsWith('-'),
    'Usage: bun import.ts <explicit-external-bundle> | --check',
  )
  // Machine identity is import evidence, never an input to committed offline validation.
  const output = await prepareImport(input, { importingHost: hostname() })
  for (const [path, bytes] of output) {
    const destination = join(import.meta.dir, path)
    await mkdir(dirname(destination), { recursive: true })
    await writeFile(destination, bytes)
  }
  const { validateInventory } = await import('./inventory')
  await validateInventory()
  process.stdout.write(
    'Imported 212 desktop contracts and 155 function bodies.\n',
  )
}

if (import.meta.main) {
  try {
    await main(Bun.argv.slice(2))
  } catch (error) {
    process.stderr.write(
      `${error instanceof Error ? error.message : String(error)}\n`,
    )
    process.exitCode = 1
  }
}
