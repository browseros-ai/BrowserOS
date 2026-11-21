#!/usr/bin/env bun
/**
 * Builds a default-off server once and tests each MCP surface in its own process.
 * Without BROWSEROS_BINARY, the suite skips before building.
 */

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { buildRustServer } from './rust-server'

const surfaces = { catalog: '0', playwright: '1' }
type Surface = keyof typeof surfaces

const args = process.argv.slice(2)
const surfaceArgs = args.filter(
  (arg) => arg === '--surface' || arg.startsWith('--surface='),
)
const requestedSurface = surfaceArgs[0]?.slice('--surface='.length)
if (
  surfaceArgs.length > 1 ||
  (requestedSurface !== undefined &&
    requestedSurface !== 'catalog' &&
    requestedSurface !== 'playwright')
) {
  console.error(
    'claw-mcp contract suite expects --surface=catalog or --surface=playwright',
  )
  process.exit(2)
}
const passes: Surface[] = requestedSurface
  ? [requestedSurface]
  : ['catalog', 'playwright']

if (!process.env.BROWSEROS_BINARY) {
  console.log(
    'claw-mcp contract suite skipped: BROWSEROS_BINARY is not set (point it at a BrowserOS/BrowserClaw executable to run)',
  )
  process.exit(0)
}

buildRustServer()

const junitPath = process.env.BROWSEROS_JUNIT_PATH?.trim()
const reportsDir = junitPath
  ? mkdtempSync(join(tmpdir(), 'claw-mcp-junit-'))
  : undefined
const failedPasses: Surface[] = []

try {
  for (const surface of passes) {
    console.log(
      `\nclaw-mcp contract suite: ${surface} (CLAW_PLAYWRIGHT_RUN=${surfaces[surface]})`,
    )
    const junitArgs = reportsDir
      ? [
          '--reporter=junit',
          `--reporter-outfile=${join(reportsDir, `${surface}.xml`)}`,
        ]
      : []
    const result = Bun.spawnSync({
      cmd: [
        process.execPath,
        'test',
        resolve(import.meta.dir, 'rust-conformance.test.ts'),
        ...junitArgs,
      ],
      env: {
        ...process.env,
        ...(args.includes('--smoke') ? { CLAW_MCP_SMOKE: '1' } : {}),
        CLAW_PLAYWRIGHT_RUN: surfaces[surface],
      },
      stdout: 'inherit',
      stderr: 'inherit',
    })
    if (result.exitCode !== 0) failedPasses.push(surface)
  }

  if (junitPath && reportsDir) {
    const outputPath = resolve(junitPath)
    mkdirSync(dirname(outputPath), { recursive: true })
    writeFileSync(outputPath, mergeReports(reportsDir))
  }
} finally {
  if (reportsDir) rmSync(reportsDir, { recursive: true, force: true })
}

if (failedPasses.length) process.exit(1)

/** Preserve Bun's nested suite elements by replacing only the reporter's XML envelope. */
function mergeReports(reportsDir: string): string {
  const suites: string[] = []
  const attributesToSum = [
    'tests',
    'assertions',
    'failures',
    'errors',
    'skipped',
    'time',
  ]
  const totals = new Map(attributesToSum.map((name) => [name, 0]))
  for (const surface of passes) {
    const path = join(reportsDir, `${surface}.xml`)
    if (!existsSync(path)) {
      // Bun emits no report when a name filter matches zero cases. A failed
      // child with no report still needs to appear in CI's summary.
      if (failedPasses.includes(surface)) {
        totals.set('tests', (totals.get('tests') ?? 0) + 1)
        totals.set('errors', (totals.get('errors') ?? 0) + 1)
        suites.push(
          `<testsuite name="${surface}" tests="1" errors="1"><testcase name="Bun test process"><error message="Test process failed before writing JUnit"/></testcase></testsuite>`,
        )
      }
      continue
    }
    const xml = readFileSync(path, 'utf8')
    const root = xml.match(
      /^<\?xml[^>]*\?>\s*<testsuites\b([^>]*)>([\s\S]*)<\/testsuites>\s*$/,
    )
    if (!root) throw new Error(`Unexpected Bun JUnit report for ${surface}`)
    const [, attributes = '', children = ''] = root
    suites.push(children)
    for (const [key, value] of totals) {
      const count = attributes.match(new RegExp(`\\b${key}="([^"]*)"`))?.[1]
      totals.set(key, value + Number(count ?? 0))
    }
  }
  const attributes = Array.from(totals)
    .map(([key, value]) => `${key}="${value}"`)
    .join(' ')
  return `<?xml version="1.0" encoding="UTF-8"?>\n<testsuites name="claw-mcp" ${attributes}>\n${suites.join('\n')}\n</testsuites>\n`
}
