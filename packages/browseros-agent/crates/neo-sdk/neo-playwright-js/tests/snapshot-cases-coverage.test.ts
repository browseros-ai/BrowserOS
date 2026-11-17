/**
 * The snapshot text cases reach every line of today's renderer: the snapshot files, the
 * covered block, and the vendored printer's half the renderer calls (`RENDERER`). A line they do not reach is
 * listed with its reason in scripts/snapshot-cases/unreached.ts. Every line reached proves every
 * line ran, not every boundary; the cases name the boundaries themselves.
 *
 * The cases run in a child `bun test --coverage` (support/snapshot-cases-run.ts) so that only
 * they are measured, not this package's other suites. Bun marks lines differently from one
 * version to the next (1.4.4 finds a branch inside a line that 1.4.2 counts as run), so the
 * report is the pinned Bun's, as the built files are; another version is refused, not judged.
 */
import { expect, it } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { engines } from '../../../../package.json'
import {
  RENDERER,
  UNREACHED,
  type Unreached,
} from '../scripts/snapshot-cases/unreached'

const packageRoot = resolve(import.meta.dir, '..')

/** Each file's lines Bun counts as code, and those of them that never ran, from its lcov report. */
type Lines = { counted: Set<number>; unrun: Set<number> }

function lcovLines(lcov: string): Map<string, Lines> {
  const files = new Map<string, Lines>()
  let file: Lines | undefined
  for (const line of lcov.split('\n')) {
    if (line.startsWith('SF:')) {
      file = { counted: new Set(), unrun: new Set() }
      files.set(resolve(packageRoot, line.slice(3)), file)
    } else if (line.startsWith('DA:') && file) {
      const [number, hits] = line.slice(3).split(',').map(Number)
      file.counted.add(number)
      if (hits === 0) file.unrun.add(number)
    }
  }
  return files
}

/**
 * Bun counts every line of a block that never ran, its comments and closing brackets included;
 * only lines with code in them need a reason.
 */
function code(text: string): boolean {
  const trimmed = text.trim()
  return !(
    trimmed === '' ||
    trimmed.startsWith('//') ||
    trimmed.startsWith('/*') ||
    trimmed.startsWith('*') ||
    /^[\s{}()[\];,]*$/.test(trimmed)
  )
}

function explained(entry: Unreached, source: string[]): number[] {
  if (entry.from === undefined) return source.map((_, index) => index + 1)
  const start = source.findIndex((line) => line.trim() === entry.from)
  if (start === -1) return []
  const to = entry.to ?? entry.from
  const end = source.findIndex(
    (line, index) => index >= start && line.trim() === to,
  )
  if (end === -1) return []
  return Array.from(
    { length: end - start + 1 },
    (_, index) => start + index + 1,
  )
}

it('tests that the snapshot text cases reach every line of the renderer, or say why not', () => {
  expect(
    Bun.version,
    `the coverage report is Bun ${engines.bun}'s (package.json engines.bun)`,
  ).toBe(engines.bun)
  const dir = mkdtempSync(join(tmpdir(), 'snapshot-cases-coverage-'))
  try {
    const run = spawnSync(
      process.execPath,
      [
        'test',
        '--coverage',
        '--coverage-reporter=lcov',
        `--coverage-dir=${dir}`,
        './tests/support/snapshot-cases-run.ts',
      ],
      { cwd: packageRoot, encoding: 'utf8' },
    )
    expect(run.status, run.stderr).toBe(0)
    const report = lcovLines(readFileSync(join(dir, 'lcov.info'), 'utf8'))
    const unexplained: string[] = []
    const stale: string[] = []
    let lines = 0
    let reached = 0
    for (const file of RENDERER) {
      const path = resolve(packageRoot, file)
      const source = readFileSync(path, 'utf8').split('\n')
      const entries = UNREACHED.filter((entry) => entry.file === file)
      // A file the cases never load has every line unrun.
      const all = new Set(source.map((_, index) => index + 1))
      const { counted, unrun } = report.get(path) ?? {
        counted: all,
        unrun: all,
      }
      const missed = unrun
      const reasons = new Set<number>()
      for (const entry of entries) {
        const numbers = explained(entry, source)
        const live = numbers.filter(
          (number) => missed.has(number) && code(source[number - 1] ?? ''),
        )
        if (live.length === 0)
          stale.push(
            `${file}: ${entry.from ?? '(whole file)'} is reached now, or not found; remove its entry`,
          )
        for (const number of numbers) reasons.add(number)
      }
      for (const [index, text] of source.entries()) {
        if (!code(text) || !counted.has(index + 1)) continue
        lines += 1
        if (!missed.has(index + 1)) reached += 1
        else if (!reasons.has(index + 1))
          unexplained.push(`${file}:${index + 1}  ${text.trim()}`)
      }
    }
    console.log(
      `snapshot text cases: ${reached} of ${lines} code lines of the renderer reached; ${lines - reached} listed with a reason`,
    )
    expect(unexplained, 'lines no case reaches and no entry explains').toEqual(
      [],
    )
    expect(stale, 'entries of unreached.ts that explain nothing').toEqual([])
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
