/**
 * Wrapper around `bun test` that spawns one process per test FILE.
 *
 * Motivation. Bun's `mock.module()` writes to a process-scoped module
 * registry. In `bun test <dir>` mode all discovered test files run
 * inside a single process, so a top-level `mock.module()` in one file
 * leaks into every other file that imports the same specifier. When
 * the mock is a partial replacement (drops any real export the
 * factory did not include) the leak surfaces later as
 * `SyntaxError: Export named 'X' not found in module '...'` on files
 * whose source clearly exports X. File-load ordering is stable on
 * macOS APFS and non-deterministic on Linux ext4, so the failure
 * intermittently kills CI while local runs pass. See the 2026-07-17
 * test reliability audit for the full trace.
 *
 * Per-file isolation kills the class of failure regardless of any
 * mock-mistake a future contributor may make. Cost is roughly one
 * bun startup per test file (~50 files x ~200ms ≈ 10s added).
 *
 * The wrapper accepts either a single directory (bun will recurse
 * into it) or a list of specific test file paths. When called with a
 * directory it walks the tree, filters to `*.test.ts` / `*.test.tsx`,
 * and spawns one child per file. When called with explicit files it
 * spawns one child per argument. In either mode it aggregates
 * per-file JUnit XML into the single output path CI expects.
 */

import { spawnSync } from 'node:child_process'
import type { Stats } from 'node:fs'
import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'

const projectRoot = resolve(import.meta.dir, '..')
const junitPath = process.env.BROWSEROS_JUNIT_PATH?.trim()
const testArgs = process.argv.slice(2)

if (testArgs.length === 0) {
  console.error(
    'run-bun-test: expected at least one file or directory argument',
  )
  process.exit(2)
}

const files = collectTestFiles(testArgs)
if (files.length === 0) {
  console.error(
    `run-bun-test: no test files found under ${testArgs.join(', ')}`,
  )
  // Emit an empty junit so the workflow upload step still has a file.
  if (junitPath) writeEmptyJunit(junitPath)
  process.exit(0)
}

const perFileJunitDir = junitPath
  ? mkdtempSync(join(tmpdir(), 'browseros-junit-'))
  : null

let failed = 0

for (const [i, file] of files.entries()) {
  const rel = relative(projectRoot, file) || file
  const cmd = [process.execPath, 'test']

  if (perFileJunitDir) {
    // One XML per test file so a later parse error in one child
    // cannot corrupt a shared junit output.
    const perFilePath = join(perFileJunitDir, `${i}.xml`)
    cmd.push('--reporter=junit', `--reporter-outfile=${perFilePath}`)
  }

  cmd.push(file)

  const result = spawnSync(cmd[0], cmd.slice(1), {
    cwd: projectRoot,
    env: process.env,
    stdio: 'inherit',
  })

  if (result.error) {
    console.error(`run-bun-test: spawn failed for ${rel}:`, result.error)
    failed += 1
    continue
  }
  if ((result.status ?? 1) !== 0) failed += 1
}

if (perFileJunitDir && junitPath) {
  const outputPath = resolve(projectRoot, junitPath)
  mkdirSync(dirname(outputPath), { recursive: true })
  mergeJunitXml(perFileJunitDir, outputPath)
  rmSync(perFileJunitDir, { recursive: true, force: true })
}

if (failed > 0) {
  console.error(`run-bun-test: ${failed} of ${files.length} test files failed`)
  process.exit(1)
}

// ------------------------------------------------------------------

function collectTestFiles(args: string[]): string[] {
  const out: string[] = []
  for (const arg of args) {
    const abs = resolve(projectRoot, arg)
    let st: Stats
    try {
      st = statSync(abs)
    } catch {
      console.error(`run-bun-test: cannot stat ${arg}`)
      process.exit(2)
    }
    if (st.isDirectory()) {
      walk(abs, out)
    } else if (isTestFile(abs)) {
      out.push(abs)
    } else {
      console.error(
        `run-bun-test: ignoring ${arg} (not a *.test.ts / *.test.tsx file or directory)`,
      )
    }
  }
  // Stable ordering so log lines and per-file XML numbering are
  // deterministic across CI reruns.
  return out.sort()
}

function walk(dir: string, acc: string[]): void {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name.startsWith('.')) continue
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      walk(full, acc)
      continue
    }
    if (isTestFile(full)) acc.push(full)
  }
}

function isTestFile(path: string): boolean {
  return path.endsWith('.test.ts') || path.endsWith('.test.tsx')
}

function mergeJunitXml(perFileDir: string, outputPath: string): void {
  const suites: string[] = []
  let totalTests = 0
  let totalFailures = 0
  let totalSkipped = 0
  const entries = readdirSync(perFileDir)
    .filter((f) => f.endsWith('.xml'))
    .sort()

  for (const entry of entries) {
    let xml: string
    try {
      xml = readFileSync(join(perFileDir, entry), 'utf8')
    } catch {
      continue
    }
    // Bun's junit output nests <testsuite> inside <testsuite> (a
    // describe group becomes a nested suite), so a lazy regex would
    // close on the inner tag and produce mismatched XML. Extract only
    // the OUTERMOST <testsuite> elements directly under the root
    // <testsuites> by tracking depth as we scan.
    for (const suiteXml of extractTopLevelTestsuites(xml)) {
      suites.push(suiteXml)
      const openTag = suiteXml.match(/^<testsuite\b[^>]*>/)?.[0] ?? ''
      totalTests += extractNumericAttr(openTag, 'tests')
      totalFailures += extractNumericAttr(openTag, 'failures')
      totalSkipped += extractNumericAttr(openTag, 'skipped')
    }
  }

  const wrapper = `<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="bun test" tests="${totalTests}" failures="${totalFailures}" skipped="${totalSkipped}">
${suites.join('\n')}
</testsuites>
`
  writeFileSync(outputPath, wrapper, 'utf8')
}

/**
 * Depth-aware scan for `<testsuite>...</testsuite>` blocks at depth 1
 * within the root `<testsuites>`. Preserves nested <testsuite> content
 * verbatim (a naive lazy regex would incorrectly close on an inner
 * tag and produce mismatched XML).
 */
function extractTopLevelTestsuites(xml: string): string[] {
  const results: string[] = []
  // `[^/>]` on the last char excludes self-closing `<testsuite ... />`
  // so we never increment depth on an element that has no matching
  // close tag (Bun does not emit self-closing today, but the guard
  // stays cheap and prevents a silent malformed-XML regression if
  // that ever changes).
  const openRe = /<testsuite\b[^>]*[^/>]>/g
  const closeRe = /<\/testsuite>/g
  let depth = 0
  let startOfCurrentTop = -1
  let cursor = 0

  while (cursor < xml.length) {
    openRe.lastIndex = cursor
    closeRe.lastIndex = cursor
    const nextOpen = openRe.exec(xml)
    const nextClose = closeRe.exec(xml)
    if (!nextOpen && !nextClose) break

    const takeOpen =
      nextOpen && (!nextClose || nextOpen.index < nextClose.index)
    if (takeOpen && nextOpen) {
      if (depth === 0) startOfCurrentTop = nextOpen.index
      depth += 1
      cursor = nextOpen.index + nextOpen[0].length
      continue
    }
    if (nextClose) {
      depth -= 1
      cursor = nextClose.index + nextClose[0].length
      if (depth === 0 && startOfCurrentTop >= 0) {
        results.push(xml.slice(startOfCurrentTop, cursor))
        startOfCurrentTop = -1
      }
      continue
    }
    break
  }
  return results
}

function extractNumericAttr(tag: string, name: string): number {
  const m = tag.match(new RegExp(`\\b${name}="(\\d+)"`))
  if (!m) return 0
  return Number.parseInt(m[1] ?? '0', 10)
}

function writeEmptyJunit(pathRelative: string): void {
  const outputPath = resolve(projectRoot, pathRelative)
  mkdirSync(dirname(outputPath), { recursive: true })
  writeFileSync(
    outputPath,
    `<?xml version="1.0" encoding="UTF-8"?>
<testsuites tests="0" failures="0"></testsuites>
`,
    'utf8',
  )
}
