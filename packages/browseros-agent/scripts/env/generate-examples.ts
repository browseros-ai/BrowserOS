import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { generateEnvExample } from '@browseros/shared/env/generate'
import type { EnvMode } from '@browseros/shared/env/registry'

const EXAMPLES: Array<{ mode: EnvMode; file: string }> = [
  { mode: 'development', file: '.env.development.example' },
  { mode: 'production', file: '.env.production.example' },
]

/** Writes or verifies the generated root env example files. */
export function runGenerateExamples(options: {
  rootDir: string
  check: boolean
}): number {
  const diffs: string[] = []

  for (const example of EXAMPLES) {
    const path = join(options.rootDir, example.file)
    const generated = generateEnvExample(example.mode)

    if (options.check) {
      const existing = existsSync(path) ? readFileSync(path, 'utf8') : ''
      if (existing !== generated) {
        diffs.push(formatDiff(example.file, existing, generated))
      }
      continue
    }

    writeFileSync(path, generated)
    console.log(`wrote ${relative(options.rootDir, path)}`)
  }

  if (diffs.length === 0) {
    return 0
  }

  console.error(diffs.join('\n\n'))
  console.error('Examples drift from the registry. Run: bun run env:examples')
  return 1
}

function formatDiff(file: string, existing: string, generated: string): string {
  const existingLines = splitLines(existing)
  const generatedLines = splitLines(generated)
  const maxLines = Math.max(existingLines.length, generatedLines.length)
  const lines = [`--- ${file}`, `+++ ${file} (generated)`]
  let emitted = 0

  for (let index = 0; index < maxLines; index += 1) {
    const oldLine = existingLines[index]
    const newLine = generatedLines[index]
    if (oldLine === newLine) {
      continue
    }

    if (emitted >= 40) {
      lines.push('...')
      break
    }

    lines.push(`@@ line ${index + 1} @@`)
    if (oldLine !== undefined) {
      lines.push(`-${oldLine}`)
    }
    if (newLine !== undefined) {
      lines.push(`+${newLine}`)
    }
    emitted += 1
  }

  return lines.join('\n')
}

function splitLines(text: string): string[] {
  const lines = text.split('\n')
  if (lines.at(-1) === '') {
    lines.pop()
  }
  return lines
}

if (import.meta.main) {
  const scriptDir = dirname(fileURLToPath(import.meta.url))
  const rootDir = resolve(scriptDir, '../..')
  const check = process.argv.includes('--check')
  const exitCode = runGenerateExamples({ rootDir, check })
  if (exitCode !== 0) {
    process.exit(exitCode)
  }
}
