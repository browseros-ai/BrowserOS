/**
 * Shared plumbing for the OpenAPI codegen scripts (`claw-api.ts`, `hosted-api.ts`).
 *
 * Both run the same pinned OpenAPI Generator image and normalize its output so a fresh
 * generation is byte-identical across machines. Their `--check` modes rely on that to
 * compare a fresh run against the committed trees.
 */

import { spawnSync } from 'node:child_process'
import { readdirSync, readFileSync } from 'node:fs'
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
