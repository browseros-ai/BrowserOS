import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { generateEnvExample } from '@browseros/shared/env/generate'
import { parseEnvFile } from '@browseros/shared/env/load'
import { type EnvMode, findEnvKeySpec } from '@browseros/shared/env/registry'

interface HarvestedValue {
  mode: EnvMode
  key: string
  value: string
  sourceFile: string
  rootFile: string
}

interface EnvConflict {
  mode: EnvMode
  key: string
  keptSourceFile: string
  conflictingSourceFile: string
}

interface DroppedEnvKey {
  mode: EnvMode
  key: string
  sourceFile: string
  reason: string
  warning?: string
}

interface PlannedRootFile {
  mode: EnvMode
  rootFile: string
  content: string
  exists: boolean
}

export interface LocalEnvMigrationPlan {
  rootDir: string
  files: PlannedRootFile[]
  harvested: HarvestedValue[]
  conflicts: EnvConflict[]
  dropped: DroppedEnvKey[]
  oldFiles: string[]
}

export interface LocalEnvMigrationResult extends LocalEnvMigrationPlan {
  dryRun: boolean
  force: boolean
  wrote: string[]
  refused: string[]
}

const MODE_ORDER: readonly EnvMode[] = ['development', 'production']
const RETIRED_KEYS = new Set(['R2_UPLOAD_PREFIX', 'R2_DOWNLOAD_PREFIX'])
const RETIRED_PREFIX_DEFAULTS: Record<string, readonly string[]> = {
  R2_UPLOAD_PREFIX: [
    'artifacts/server',
    'cli',
    'claw-server/prod-resources',
    'claw-onboard/prod-resources',
  ],
  R2_DOWNLOAD_PREFIX: ['artifacts/vendor'],
}
const PRIMARY_OLD_FILES: Record<EnvMode, string[]> = {
  development: ['apps/server/.env.development', 'apps/app/.env.development'],
  production: ['apps/server/.env.production', 'apps/cli/.env.production'],
}

/** Builds the root env migration plan from existing per-app env files. */
export function buildLocalEnvMigration(rootDir: string): LocalEnvMigrationPlan {
  const harvested: HarvestedValue[] = []
  const conflicts: EnvConflict[] = []
  const dropped: DroppedEnvKey[] = []
  const oldFiles = new Set<string>()
  const files: PlannedRootFile[] = []

  for (const mode of MODE_ORDER) {
    const rootFile = join(rootDir, `.env.${mode}`)
    const selected = new Map<string, HarvestedValue>()
    const oldModeFiles = findOldEnvFiles(rootDir, mode)

    for (const sourceFile of oldModeFiles) {
      oldFiles.add(sourceFile)
      harvestEnvFile({
        mode,
        rootFile,
        sourceFile,
        selected,
        harvested,
        conflicts,
        dropped,
      })
    }

    files.push({
      mode,
      rootFile,
      content: applyHarvestedValues(generateEnvExample(mode), selected),
      exists: existsSync(rootFile),
    })
  }

  return {
    rootDir,
    files,
    harvested,
    conflicts,
    dropped,
    oldFiles: [...oldFiles],
  }
}

/** Writes the migration plan unless dry-run or overwrite protection blocks a file. */
export function migrateLocalEnv(options: {
  rootDir: string
  dryRun?: boolean
  force?: boolean
}): LocalEnvMigrationResult {
  const plan = buildLocalEnvMigration(options.rootDir)
  const dryRun = options.dryRun === true
  const force = options.force === true
  const wrote: string[] = []
  const refused: string[] = []

  for (const file of plan.files) {
    if (dryRun) {
      continue
    }

    if (file.exists && !force) {
      refused.push(file.rootFile)
      continue
    }

    writeFileSync(file.rootFile, file.content)
    wrote.push(file.rootFile)
  }

  return { ...plan, dryRun, force, wrote, refused }
}

function harvestEnvFile(options: {
  mode: EnvMode
  rootFile: string
  sourceFile: string
  selected: Map<string, HarvestedValue>
  harvested: HarvestedValue[]
  conflicts: EnvConflict[]
  dropped: DroppedEnvKey[]
}): void {
  const values = parseEnvFile(readFileSync(options.sourceFile, 'utf8'))

  for (const [key, value] of Object.entries(values)) {
    if (!isKnownForMode(key, options.mode)) {
      options.dropped.push({
        mode: options.mode,
        key,
        sourceFile: options.sourceFile,
        reason: RETIRED_KEYS.has(key)
          ? 'retired to code constants'
          : `not in the ${options.mode} registry`,
        warning: retiredPrefixWarning(key, value, options.sourceFile),
      })
      continue
    }

    if (value.trim() === '') {
      continue
    }

    const existing = options.selected.get(key)
    if (existing) {
      if (existing.value !== value) {
        options.conflicts.push({
          mode: options.mode,
          key,
          keptSourceFile: existing.sourceFile,
          conflictingSourceFile: options.sourceFile,
        })
      }
      continue
    }

    const harvestedValue = {
      mode: options.mode,
      key,
      value,
      sourceFile: options.sourceFile,
      rootFile: options.rootFile,
    }
    options.selected.set(key, harvestedValue)
    options.harvested.push(harvestedValue)
  }
}

function findOldEnvFiles(rootDir: string, mode: EnvMode): string[] {
  const primaryFiles = PRIMARY_OLD_FILES[mode].map((path) =>
    join(rootDir, path),
  )
  const found = findAppEnvFiles(rootDir, `.env.${mode}`)
  const legacyRootFiles =
    mode === 'development'
      ? [join(rootDir, '.env'), join(rootDir, '.env.local')]
      : []
  const ordered = [...primaryFiles, ...found.sort(), ...legacyRootFiles]
  const seen = new Set<string>()

  return ordered.filter((path) => {
    if (seen.has(path) || !existsSync(path)) {
      return false
    }

    seen.add(path)
    return true
  })
}

function retiredPrefixWarning(
  key: string,
  value: string,
  sourceFile: string,
): string | undefined {
  const defaults = RETIRED_PREFIX_DEFAULTS[key]
  const trimmedValue = value.trim()
  if (!defaults || trimmedValue === '' || defaults.includes(trimmedValue)) {
    return undefined
  }

  return `RETIRED PREFIX WARNING: ${key}=${trimmedValue} from ${sourceFile} is retired; builds now default to code constants (${defaults.join(
    ', ',
  )}). Export ${key} in the environment when running upload scripts to use a custom prefix.`
}

function findAppEnvFiles(rootDir: string, filename: string): string[] {
  const appsDir = join(rootDir, 'apps')
  if (!existsSync(appsDir)) {
    return []
  }

  return readdirSync(appsDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(appsDir, entry.name, filename))
    .filter((path) => existsSync(path))
}

function isKnownForMode(key: string, mode: EnvMode): boolean {
  const spec = findEnvKeySpec(key)
  return spec?.modes[mode] !== undefined
}

function applyHarvestedValues(
  example: string,
  selected: Map<string, HarvestedValue>,
): string {
  const used = new Set<string>()
  const lines = example.split('\n').map((line) => {
    const match = /^#?\s*([A-Za-z_][A-Za-z0-9_]*)=.*/.exec(line)
    if (!match) {
      return line
    }

    const harvested = selected.get(match[1])
    if (!harvested) {
      return line
    }

    used.add(harvested.key)
    return `${harvested.key}=${formatEnvValue(harvested.value)}`
  })

  for (const harvested of selected.values()) {
    if (!used.has(harvested.key)) {
      lines.push(`${harvested.key}=${formatEnvValue(harvested.value)}`)
    }
  }

  return lines.join('\n')
}

function formatEnvValue(value: string): string {
  if (value === '') {
    return ''
  }

  if (/^[A-Za-z0-9_./:@~+=,-]+$/.test(value)) {
    return value
  }

  return `"${value
    .replaceAll('\\', '\\\\')
    .replaceAll('"', '\\"')
    .replaceAll('\n', '\\n')
    .replaceAll('\r', '\\r')
    .replaceAll('\t', '\\t')}"`
}

function printResult(result: LocalEnvMigrationResult): void {
  for (const dropped of result.dropped) {
    if (dropped.warning) {
      console.warn(dropped.warning)
    } else {
      console.warn(
        `dropped ${relative(result.rootDir, dropped.sourceFile)} ${dropped.key}: ${dropped.reason}`,
      )
    }
  }

  for (const conflict of result.conflicts) {
    console.warn(
      `CONFLICT key=${conflict.key}: kept ${relative(
        result.rootDir,
        conflict.keptSourceFile,
      )}'s value, also found in ${relative(
        result.rootDir,
        conflict.conflictingSourceFile,
      )}`,
    )
  }

  for (const harvested of result.harvested) {
    console.log(
      `${harvested.key}: ${relative(result.rootDir, harvested.sourceFile)} -> ${relative(
        result.rootDir,
        harvested.rootFile,
      )}`,
    )
  }

  for (const file of result.files) {
    const rootPath = relative(result.rootDir, file.rootFile)
    if (result.dryRun) {
      console.log(`dry run: would write ${rootPath}`)
    } else if (result.wrote.includes(file.rootFile)) {
      console.log(`wrote ${rootPath}`)
    }
  }

  for (const path of result.refused) {
    console.error(
      `refused to overwrite ${relative(result.rootDir, path)}; pass --force`,
    )
  }

  if (result.oldFiles.length > 0) {
    console.log('after verifying the root env files, old files can be deleted:')
    for (const path of result.oldFiles) {
      console.log(`  rm ${relative(result.rootDir, path)}`)
    }
  }
}

if (import.meta.main) {
  const scriptDir = dirname(fileURLToPath(import.meta.url))
  const rootDir = resolve(scriptDir, '../..')
  const dryRun = process.argv.includes('--dry-run')
  const force = process.argv.includes('--force')
  const result = migrateLocalEnv({ rootDir, dryRun, force })
  printResult(result)

  if (result.refused.length > 0) {
    process.exit(1)
  }
}
