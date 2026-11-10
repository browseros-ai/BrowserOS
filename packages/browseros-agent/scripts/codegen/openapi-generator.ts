/**
 * Shared plumbing for the OpenAPI codegen scripts (`claw-api.ts`, `hosted-api.ts`,
 * `neo-sdk-api.ts`). The two models-only contracts share `runRustModelsCodegen` whole.
 *
 * Both run the same pinned OpenAPI Generator image and normalize its output so a fresh
 * generation is byte-identical across machines. Their `--check` modes rely on that to
 * compare a fresh run against the committed trees.
 */

import { spawnSync } from 'node:child_process'
import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export const OPENAPI_GENERATOR_IMAGE =
  'openapitools/openapi-generator-cli:v7.22.0'

/** Runs the container as the invoking user so bind-mounted output isn't root-owned on Linux. */
export function dockerUserMount(): string {
  return `${process.getuid?.() ?? 1000}:${process.getgid?.() ?? 1000}`
}

export function runDocker(args: string[]): void {
  const result = spawnSync('docker', args, { stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`OpenAPI Generator exited with status ${result.status}`)
  }
}

export function rustfmtFiles(directory: string): void {
  for (const file of listFiles(directory).filter((path) =>
    path.endsWith('.rs'),
  )) {
    const result = spawnSync(
      'rustfmt',
      ['--edition', '2024', join(directory, file)],
      { stdio: 'inherit' },
    )
    if (result.error) throw result.error
    if (result.status !== 0) {
      throw new Error(`rustfmt exited with status ${result.status}`)
    }
  }
}

export function listFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(directory, entry.name)
      return entry.isDirectory()
        ? listFiles(path).map((child) => join(entry.name, child))
        : [entry.name]
    })
    .toSorted()
}

/** Throws, naming every missing, changed or unexpected file, unless both trees are identical. */
export function assertTreeMatches(
  expected: string,
  actual: string,
  label: string,
): void {
  const expectedFiles = listFiles(expected)
  const actualFiles = listFiles(actual)
  const differences = new Set<string>()

  for (const file of expectedFiles) {
    if (!actualFiles.includes(file)) {
      differences.add(`missing ${file}`)
      continue
    }
    if (
      !readFileSync(join(expected, file)).equals(
        readFileSync(join(actual, file)),
      )
    ) {
      differences.add(`changed ${file}`)
    }
  }
  for (const file of actualFiles) {
    if (!expectedFiles.includes(file)) differences.add(`unexpected ${file}`)
  }
  if (differences.size > 0) {
    throw new Error(
      `Generated ${label} is stale:\n${[...differences].map((line) => `  ${line}`).join('\n')}`,
    )
  }
}

/**
 * A contract whose spec generates only Rust models. Paths are relative to the agent root;
 * defaults preserve the separate contracts/<name> and crates/<name> layout for existing APIs.
 */
export interface RustModelsContract {
  name: string
  /** How messages name the API, e.g. `hosted API`. */
  label: string
  /** Spec path when the contract lives alongside its generated crate. */
  spec?: string
  /** Crate root when it lives below a grouping folder. */
  crateDir?: string
}

/**
 * Regenerates a models-only contract's crate, or with `--check` (run in CI) regenerates into a
 * temporary directory and fails if the committed tree differs by a byte. `root` is
 * `packages/browseros-agent`.
 */
export function runRustModelsCodegen(
  contract: RustModelsContract,
  root: string,
  argv: string[],
): void {
  const target = join(
    root,
    contract.crateDir ?? `crates/${contract.name}`,
    'src/generated',
  )
  const temporaryRoot = mkdtempSync(join(tmpdir(), `${contract.name}-codegen-`))
  try {
    runDocker([
      'run',
      '--rm',
      '--user',
      dockerUserMount(),
      '-v',
      `${root}:/local:ro`,
      '-v',
      `${temporaryRoot}:/out`,
      OPENAPI_GENERATOR_IMAGE,
      'generate',
      '-i',
      `/local/${contract.spec ?? `contracts/${contract.name}/openapi.yaml`}`,
      '-g',
      'rust',
      '-o',
      '/out/rust',
      '--global-property',
      'models,supportingFiles,modelDocs=false,modelTests=false,apis=false,apiDocs=false,apiTests=false',
      '--additional-properties',
      `packageName=${contract.name},packageVersion=1.0.0`,
    ])
    const models = join(temporaryRoot, 'rust/src/models')
    rustfmtFiles(models)
    if (argv.includes('--check')) {
      assertTreeMatches(models, target, contract.label)
      console.log(`The ${contract.label} generated output is current.`)
    } else {
      rmSync(target, { recursive: true, force: true })
      cpSync(models, target, { recursive: true })
      console.log(`Generated the ${contract.label} Rust models.`)
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
