/**
 * Codegen for the canonical BrowserClaw API. Reads the OpenAPI spec at
 * `contracts/claw-api/openapi.yaml` and emits language-specific DTOs:
 * TypeScript models into `packages/claw-api/src/generated` and Rust
 * models into `crates/claw-api/src/generated`. Neither tree
 * is ever hand-edited — change the spec, then regenerate with
 * `bun run codegen:claw-api`.
 *
 * The generator runs in Docker with a pinned image and the output is
 * normalized (trailing whitespace stripped, generated Rust run through
 * rustfmt) so the emitted trees are byte-identical across machines.
 * That determinism is what lets `--check` (`bun run
 * codegen:claw-api:check`, the CI drift gate) compare a fresh
 * generation against the committed trees byte for byte.
 */

import { spawnSync } from 'node:child_process'
import {
  cpSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'

const root = join(import.meta.dir, '../..')
const image = 'openapitools/openapi-generator-cli:v7.22.0'
const check = process.argv.includes('--check')

interface GeneratedTrees {
  typescript: string
  rust: string
}

function runGenerator(outputRoot: string): GeneratedTrees {
  // Run the container as the invoking user so the generated files on
  // the bind mount aren't root-owned on Linux hosts.
  const mount = `${process.getuid?.() ?? 1000}:${process.getgid?.() ?? 1000}`
  const common = [
    'run',
    '--rm',
    '--user',
    mount,
    '-v',
    `${root}:/local:ro`,
    '-v',
    `${outputRoot}:/out`,
    image,
    'generate',
    '-i',
    '/local/contracts/claw-api/openapi.yaml',
  ]

  runDocker([
    ...common,
    '-g',
    'typescript-fetch',
    '-t',
    '/local/scripts/codegen/templates/claw-api/typescript-fetch',
    '-o',
    '/out/typescript',
    '--global-property',
    'models,modelDocs=false,modelTests=false',
    '--additional-properties',
    'supportsES6=true,typescriptThreePlus=true,importFileExtension=.js,disallowAdditionalPropertiesIfNotPresent=false',
  ])
  runDocker([
    ...common,
    '-g',
    'rust',
    '-o',
    '/out/rust',
    '--global-property',
    'models,supportingFiles,modelDocs=false,modelTests=false,apis=false,apiDocs=false,apiTests=false',
    '--additional-properties',
    'packageName=claw-api,packageVersion=1.0.0',
  ])

  const typescript = join(outputRoot, 'typescript')
  const rust = join(outputRoot, 'rust/src/models')
  rmSync(join(typescript, '.openapi-generator'), {
    recursive: true,
    force: true,
  })
  rmSync(join(typescript, '.openapi-generator-ignore'), { force: true })
  for (const file of listFiles(typescript).filter((path) =>
    path.endsWith('.ts'),
  )) {
    const path = join(typescript, file)
    const source = readFileSync(path, 'utf8')
    const normalized = `${source
      .split('\n')
      .map((line) => line.trimEnd())
      .join('\n')
      .trimEnd()}\n`
    writeFileSync(path, normalized)
  }
  const typescriptModels = listFiles(join(typescript, 'models')).filter(
    (file) => file.endsWith('.ts'),
  )
  writeFileSync(
    join(typescript, 'index.ts'),
    `${typescriptModels
      .map((file) => `export * from './models/${file.replace(/\.ts$/, '.js')}'`)
      .join('\n')}\n`,
  )
  for (const file of listFiles(rust).filter((path) => path.endsWith('.rs'))) {
    const result = spawnSync(
      'rustfmt',
      ['--edition', '2024', join(rust, file)],
      {
        stdio: 'inherit',
      },
    )
    if (result.error) throw result.error
    if (result.status !== 0) {
      throw new Error(`rustfmt exited with status ${result.status}`)
    }
  }

  return {
    typescript,
    rust,
  }
}

function runDocker(args: string[]): void {
  const result = spawnSync('docker', args, { stdio: 'inherit' })
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`OpenAPI Generator exited with status ${result.status}`)
  }
}

function installGenerated(trees: GeneratedTrees): void {
  const typescriptTarget = join(root, 'packages/claw-api/src/generated')
  const rustTarget = join(root, 'crates/claw-api/src/generated')

  rmSync(typescriptTarget, { recursive: true, force: true })
  cpSync(trees.typescript, typescriptTarget, {
    recursive: true,
    filter: (source) => {
      const path = relative(trees.typescript, source)
      return !path.startsWith('.openapi-generator')
    },
  })
  rmSync(join(typescriptTarget, '.openapi-generator-ignore'), { force: true })

  rmSync(rustTarget, { recursive: true, force: true })
  cpSync(trees.rust, rustTarget, { recursive: true })
}

function listFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) => {
      const path = join(directory, entry.name)
      return entry.isDirectory()
        ? listFiles(path).map((child) => join(entry.name, child))
        : [entry.name]
    })
    .toSorted()
}

function assertTreeMatches(expected: string, actual: string): void {
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
      `Generated BrowserClaw API is stale:\n${[...differences].map((line) => `  ${line}`).join('\n')}`,
    )
  }
}

if (import.meta.main) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'claw-api-codegen-'))
  try {
    const trees = runGenerator(temporaryRoot)
    if (check) {
      assertTreeMatches(
        trees.typescript,
        join(root, 'packages/claw-api/src/generated'),
      )
      assertTreeMatches(trees.rust, join(root, 'crates/claw-api/src/generated'))
      console.log('BrowserClaw API generated output is current.')
    } else {
      installGenerated(trees)
      console.log('Generated BrowserClaw TypeScript and Rust DTOs.')
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
