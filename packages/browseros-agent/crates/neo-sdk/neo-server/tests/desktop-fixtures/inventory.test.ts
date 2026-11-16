import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { historicalFailures } from './cases/history'
import { parseManifestCases, type SavedStatus } from './cases/import-source'
import {
  bodyPaths,
  requireCondition,
  strictFailures,
  validateCaseContract,
} from './cases/requirements'
import { requirePublicText, serializeJson } from './cases/serialization'
import { prepareImport } from './import'
import {
  baselineCases as cases,
  loadBody,
  loadCommon,
  validateInventory,
} from './inventory'
import type { CaseSpec } from './model'

const originals = new Map(
  cases.map(({ id, file, name }) => [id, { id, file, name }]),
)
const importContext = { importingHost: 'synthetic-import-host' }

function requiredCase(id: string): CaseSpec {
  const entry = cases.find((entry) => entry.id === id)
  requireCondition(entry, `Missing test case ${id}`)
  return entry
}

function externalRow(entry: CaseSpec): Record<string, unknown> {
  const last = entry.steps.at(-1)
  if (last?.kind !== 'script')
    throw new Error('Synthetic input needs a primary body')
  const originalPath = (path: string) => path.replace(/\.txt$/, '.js')
  return {
    id: entry.id,
    file: entry.file,
    name: entry.name,
    proves: entry.proves,
    script: originalPath(last.script),
    classification: null,
    settings: entry.settings ?? {},
    timeout: entry.timeoutMs,
    ...(entry.steps.length > 1
      ? {
          steps: entry.steps.map((step) =>
            step.kind === 'script'
              ? originalPath(step.script)
              : { parallel: step.scripts.map(originalPath) },
          ),
        }
      : {}),
    ...(entry.hostSetup
      ? {
          hostSetup:
            entry.hostSetup.kind === 'borrowed-page'
              ? {
                  ...entry.hostSetup,
                  mustCreateOutsideSdk: true,
                  cleanup:
                    'The owner closes its external target after the script.',
                }
              : entry.hostSetup,
        }
      : {}),
    ...(entry.hostCheck ? { hostCheck: entry.hostCheck } : {}),
    ...(entry.postconditions ? { postconditions: entry.postconditions } : {}),
    ...(entry.source ? { source: entry.source } : {}),
    ...(entry.sourceSha ? { sourceSha: entry.sourceSha } : {}),
  }
}

/** A complete synthetic source bundle tests import reproducibility without any private input. */
async function syntheticBundle(root: string): Promise<void> {
  const sizes = { g1a: 50, g1b: 70, g1c: 44, g2: 67, g3: 109, g4: 19 }
  const groups = new Map<string, Record<string, unknown>[]>(
    Object.keys(sizes).map((group) => [group, []]),
  )
  const inventory = [...originals.values()]
  const statuses: Record<string, { status: SavedStatus; evidence: string }> = {}
  for (const entry of cases) {
    const firstBody = entry.steps.flatMap(bodyPaths)[0]
    requireCondition(firstBody, 'Synthetic case needs a body')
    const group = firstBody.split('/')[1]
    requireCondition(group, 'Synthetic body needs a group')
    const rows = groups.get(group)
    requireCondition(rows, 'Unknown synthetic body group')
    rows.push(externalRow(entry))
    statuses[entry.id] = {
      status: historicalFailures.has(entry.id) ? 'FAIL' : 'PASS',
      evidence: 'results/runs/synthetic-proof.json',
    }
  }
  const excluded: SavedStatus[] = [
    ...Array.from({ length: 90 }, (): SavedStatus => 'DIFFERENT BY DESIGN'),
    ...Array.from({ length: 13 }, (): SavedStatus => 'NO EQUIVALENT'),
    ...Array.from({ length: 44 }, (): SavedStatus => 'NOT A BROWSER BEHAVIOUR'),
  ]
  for (const [group, count] of Object.entries(sizes)) {
    const rows = groups.get(group)
    requireCondition(rows, 'Missing synthetic manifest group')
    while (rows.length < count) {
      const status = excluded.shift()
      requireCondition(status, 'Missing synthetic excluded classification')
      const original = {
        id: `excluded-${group}-${String(rows.length + 1).padStart(3, '0')}`,
        file: 'cases-excluded.ts',
        name: 'Excluded source contract',
      }
      rows.push({ ...original, classification: status })
      inventory.push(original)
      statuses[original.id] = {
        status,
        evidence: 'results/runs/excluded-proof.json',
      }
    }
  }
  requireCondition(
    excluded.length === 0,
    'All synthetic excluded rows must be consumed',
  )
  const files = new Map<string, string | Uint8Array>([
    ['inventory.json', serializeJson(inventory)],
    ['results/index.json', serializeJson(statuses)],
    ['common.js', await loadCommon()],
  ])
  for (const [group, rows] of groups)
    files.set(`manifests/${group}.json`, serializeJson(rows))
  for (const path of new Set(
    cases.flatMap((entry) => entry.steps.flatMap(bodyPaths)),
  )) {
    files.set(
      path.replace(/\.txt$/, '.js'),
      await readFile(join(import.meta.dir, path)),
    )
  }
  await Promise.all(
    [...files].map(async ([path, content]) => {
      const destination = join(root, path)
      await mkdir(dirname(destination), { recursive: true })
      await writeFile(destination, content)
    }),
  )
}

describe('committed desktop fixture inventory', () => {
  test('validates canonical registry, exact membership, raw bytes, and syntax offline', async () => {
    await validateInventory()
  })

  test('committed validation is independent of the executing hostname', async () => {
    for (const host of ['runner', 'localhost']) {
      // A child isolates the host mock from the other process and importer checks. These
      // names legitimately occur in committed comments and fixture URLs, not host evidence.
      const source = `import { spyOn } from 'bun:test';
import * as os from 'node:os';
spyOn(os, 'hostname').mockReturnValue(${JSON.stringify(host)});
if (os.hostname() !== ${JSON.stringify(host)}) throw new Error('Hostname fixture did not take effect');
const { validateInventory } = await import(${JSON.stringify(join(import.meta.dir, 'inventory.ts'))});
await validateInventory();`
      const child = Bun.spawn({
        cmd: [process.execPath, '--eval', source],
        stdout: 'ignore',
        stderr: 'pipe',
      })
      const [code, stderr] = await Promise.all([
        child.exited,
        new Response(child.stderr).text(),
      ])
      expect({ code, stderr }).toEqual({ code: 0, stderr: '' })
    }
  })

  test('does not allow a missing, duplicated, or renamed expanded case', () => {
    expect(() => validateCaseContract(cases.slice(1))).toThrow('212')
    expect(() =>
      validateCaseContract(
        cases.map((entry) =>
          entry.id === 'act-001' ? { ...entry, id: 'act-002' } : entry,
        ),
      ),
    ).toThrow('unique')
    expect(() =>
      validateCaseContract(
        cases.map((entry) =>
          entry.id === 'act-001' ? { ...entry, id: 'act-099' } : entry,
        ),
      ),
    ).toThrow('digest')
  })

  test('retains strict failures and the fixed iframe download as an ordinary pass', () => {
    const altered: CaseSpec[] = cases.map((entry) =>
      strictFailures.has(entry.id)
        ? { ...entry, expectation: { kind: 'pass' } }
        : entry,
    )
    if (strictFailures.size > 0)
      expect(() => validateCaseContract(altered)).toThrow('failure contract')
    else expect(() => validateCaseContract(altered)).not.toThrow()
    expect(requiredCase('capture-io-017').expectation).toEqual({ kind: 'pass' })
    expect(requiredCase('read-eval-021').expectation).toEqual({ kind: 'pass' })
  })

  test('rejects path traversal before reading a body', async () => {
    for (const path of [
      '/tmp/body.txt',
      'scripts/g2/../body.txt',
      'scripts/g2/body.js',
    ]) {
      await expect(loadBody(path)).rejects.toThrow('Invalid function-body path')
    }
  })

  test('rejects mismatched original titles, excluded metadata, and weakened parallel steps', () => {
    const row = externalRow(requiredCase('snapshot-concurrency-007'))
    expect(() =>
      parseManifestCases([{ ...row, name: 'Changed title' }], originals),
    ).toThrow('title')
    expect(() =>
      parseManifestCases([{ ...row, expectedMismatch: 'anything' }], originals),
    ).toThrow('Excluded')
    expect(() =>
      parseManifestCases(
        [
          {
            ...row,
            steps: [{ parallel: ['scripts/g3/snapshot-cursor-capture.js'] }],
          },
        ],
        originals,
      ),
    ).toThrow('four')
    expect(() =>
      parseManifestCases([{ ...row, script: '../outside.js' }], originals),
    ).toThrow('path')
    expect(() =>
      parseManifestCases(
        [
          {
            ...externalRow(requiredCase('capture-io-018')),
            settings: { trace: { valueBytes: 262144, unsupported: true } },
          },
        ],
        originals,
      ),
    ).toThrow('modeled')
  })

  test('rejects private paths and key material in public data', () => {
    expect(() =>
      requirePublicText('/Users/synthetic/private/report.json'),
    ).toThrow('private')
    expect(() => requirePublicText('-----BEGIN PRIVATE KEY-----')).toThrow(
      'private',
    )
    expect(() => requirePublicText("apiKey='secret-value'")).toThrow(
      'credential',
    )
  })

  test('explicit import context rejects importing-host identity leaks only during import', () => {
    const content = 'Recorded on synthetic-import-host'
    expect(() => requirePublicText(content)).not.toThrow()
    expect(() =>
      requirePublicText(content, { importingHost: 'synthetic-import-host' }),
    ).toThrow('importing host identity')
  })
})

describe('deterministic explicit importer', () => {
  let root = ''
  beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), 'desktop-fixture-import-'))
    await syntheticBundle(root)
  })
  afterAll(async () => {
    if (root) await rm(root, { recursive: true, force: true })
  })

  async function rejectChangedSource(
    path: string,
    change: (text: string) => string,
    message: string,
  ): Promise<void> {
    const absolute = join(root, path)
    const original = await readFile(absolute, 'utf8')
    try {
      await writeFile(absolute, change(original))
      await expect(prepareImport(root, importContext)).rejects.toThrow(message)
    } finally {
      await writeFile(absolute, original)
    }
  }

  test('two imports reproduce every committed byte and never emit excluded bodies or proofs', async () => {
    const first = await prepareImport(root, importContext)
    const second = await prepareImport(root, importContext)
    expect(first.size).toBe(173)
    expect([...first.keys()]).toEqual([...second.keys()])
    for (const [path, bytes] of first) {
      expect(
        Buffer.from(bytes).equals(await readFile(join(import.meta.dir, path))),
      ).toBe(true)
      const repeated = second.get(path)
      requireCondition(repeated, `Second import omitted ${path}`)
      expect(Buffer.from(bytes).equals(Buffer.from(repeated))).toBe(true)
    }
    expect([...first.keys()].some((path) => path.includes('results'))).toBe(
      false,
    )
  })

  test('rejects the importing host identity before emitting imported content', async () => {
    await rejectChangedSource(
      'common.js',
      (text) => `${text}\n// Recorded on ${importContext.importingHost}\n`,
      'importing host identity',
    )
  })

  test('rejects changed saved classification counts before emitting files', async () => {
    await rejectChangedSource(
      'results/index.json',
      (text) => text.replace('"PASS"', '"FAIL"'),
      'Saved PASS count',
    )
  })

  test('rejects duplicate manifest IDs and changed original titles', async () => {
    await rejectChangedSource(
      'manifests/g1a.json',
      (text) => {
        const rows: unknown = JSON.parse(text)
        requireCondition(
          Array.isArray(rows),
          'Expected synthetic manifest rows',
        )
        rows.splice(1, 1, rows[0])
        return serializeJson(rows)
      },
      'Duplicate manifest ID',
    )
    await rejectChangedSource(
      'manifests/g1a.json',
      (text) =>
        text.replace(
          JSON.stringify(requiredCase('act-001').name),
          '"Changed title"',
        ),
      'Original title',
    )
  })
})
