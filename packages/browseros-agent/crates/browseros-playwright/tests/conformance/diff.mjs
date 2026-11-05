/**
 * The tier-1 verdict: Runner P's outcomes (real Playwright) against Runner N's (Neo), case by
 * case, after normalizing what legitimately differs between two runs of one script:
 *
 *   refs      canonical by first appearance: frame seqs become f1, f2.. and each frame's
 *             element numbers e1, e2.., so `f7e3` and `f2e1` compare by where they occur
 *   durations `<n> ms` in strings becomes `<n>ms`, and keys named duration, elapsed, took or
 *             ending in Ms are dropped
 *   errors    every { threw: { name, message } } keeps its name and its message's first line
 *   ports     the fixture server's port, new each run, becomes `<port>` in its URLs
 *
 * Every remaining difference must be listed in deviations.yaml, matched by case and JSON
 * pointer, with both runners' values and a reason. The run fails on a difference not listed,
 * a listed one whose values changed, a listed one that no longer happens (delete it), a case
 * only one runner ran, and a runner that broke.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core> [CONFORMANCE_CASES_DIR=cases/tier2] \
 *   node diff.mjs <P outcomes dir> <N outcomes dir> <deviations.yaml> [--stubs]
 *
 * CONFORMANCE_CASES_DIR (cases by default) is the tier's case directory, where an entry's case
 * must still exist.
 *
 * --stubs also prints each unlisted difference as a deviations.yaml entry to fill in.
 */
import { readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const nodeModules = process.env.PLAYWRIGHT_NODE_MODULES
if (!nodeModules) {
  console.error('diff: set PLAYWRIGHT_NODE_MODULES')
  process.exit(2)
}
const require = createRequire(resolve(nodeModules, 'diff.cjs'))
const { yaml, diff } = require('playwright-core/lib/utilsBundle')

const args = process.argv.slice(2)
const stubs = args.includes('--stubs')
const [pDir, nDir, deviationsFile] = args.filter((arg) => arg !== '--stubs')
if (!deviationsFile) {
  console.error(
    'usage: node diff.mjs <P dir> <N dir> <deviations.yaml> [--stubs]',
  )
  process.exit(2)
}

/** A key one outcome has and the other lacks, as deviations.yaml writes it. */
const ABSENT = '(absent)'
const KINDS = new Set(['decided', 'finding'])
const REF = /\b(?:f(\d+))?e(\d+)\b/g
const DURATION_KEY = /^(?:duration|elapsed|took)$|Ms$/
const FIXTURE_PORT = /(127\.0\.0\.1|localhost|\[::1\]|hang\.localtest\.me):\d+/g

function outcomes(dir) {
  const found = new Map()
  for (const file of readdirSync(dir))
    if (file.endsWith('.json'))
      found.set(
        basename(file, '.json'),
        JSON.parse(readFileSync(resolve(dir, file), 'utf8')),
      )
  return found
}

/** One outcome with the three normalizations applied, keys visited in sorted order. */
function normalize(outcome) {
  const frames = new Map()
  const elements = new Map()
  const canonicalRef = (_, frame, element) => {
    const scope = frame ?? 'main'
    if (frame !== undefined && !frames.has(frame))
      frames.set(frame, frames.size + 1)
    const key = `${scope}:${element}`
    if (!elements.has(key)) {
      const counted = [...elements.keys()].filter((seen) =>
        seen.startsWith(`${scope}:`),
      ).length
      elements.set(key, counted + 1)
    }
    const prefix = frame === undefined ? '' : `f${frames.get(frame)}`
    return `${prefix}e${elements.get(key)}`
  }
  const walk = (value) => {
    if (typeof value === 'string')
      return value
        .replace(REF, canonicalRef)
        .replace(/\b\d+(?:\.\d+)?\s?ms\b/g, '<n>ms')
        .replace(FIXTURE_PORT, '$1:<port>')
    if (Array.isArray(value)) return value.map(walk)
    if (value === null || typeof value !== 'object') return value
    const shaped = {}
    for (const key of Object.keys(value).sort()) {
      if (DURATION_KEY.test(key)) continue
      shaped[key] = walk(value[key])
    }
    if (isThrown(shaped))
      shaped.threw.message = shaped.threw.message.split('\n')[0]
    return shaped
  }
  return walk(outcome)
}

function isThrown(value) {
  return (
    typeof value.threw === 'object' &&
    value.threw !== null &&
    typeof value.threw.message === 'string'
  )
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)
const pointer = (path, key) =>
  `${path}/${String(key).replaceAll('~', '~0').replaceAll('/', '~1')}`

/**
 * Each place the two values differ, as deep as both still have the same shape. Two objects
 * with no key in common (one `returned`, the other `threw`) differ as a whole.
 */
function differences(p, n, path = '', found = []) {
  if (same(p, n)) return found
  const bothObjects =
    p !== null &&
    n !== null &&
    typeof p === 'object' &&
    typeof n === 'object' &&
    Array.isArray(p) === Array.isArray(n)
  const sameShape = Array.isArray(p)
    ? p.length === n.length
    : bothObjects && Object.keys(p).some((key) => key in n)
  if (bothObjects && sameShape) {
    const keys = [...new Set([...Object.keys(p), ...Object.keys(n)])].sort()
    for (const key of keys) {
      const pointed = pointer(path, key)
      if (!(key in p)) found.push({ path: pointed, p: ABSENT, n: n[key] })
      else if (!(key in n)) found.push({ path: pointed, p: p[key], n: ABSENT })
      else differences(p[key], n[key], pointed, found)
    }
    return found
  }
  found.push({ path: path || '/', p, n })
  return found
}

function loadDeviations(file) {
  const listed = yaml.parse(readFileSync(file, 'utf8')) ?? []
  const problems = []
  const byKey = new Map()
  for (const [index, entry] of listed.entries()) {
    const where = `deviations.yaml entry ${index + 1} (${entry?.case} ${entry?.path})`
    if (typeof entry?.case !== 'string' || typeof entry?.path !== 'string')
      problems.push(`${where}: needs case and path`)
    if (!('p' in (entry ?? {})) || !('n' in (entry ?? {})))
      problems.push(`${where}: needs both runners' values, p and n`)
    if (typeof entry?.reason !== 'string' || entry.reason.trim() === '')
      problems.push(`${where}: needs a reason`)
    if (!KINDS.has(entry?.kind))
      problems.push(`${where}: kind is decided or finding`)
    if (entry?.kind === 'finding' && !entry.route)
      problems.push(`${where}: a finding names the unit to route it to`)
    const key = `${entry?.case} ${entry?.path}`
    if (byKey.has(key)) problems.push(`${where}: listed twice`)
    byKey.set(key, { ...entry, used: false })
  }
  return { byKey, problems }
}

function show(value) {
  return typeof value === 'string' ? value : JSON.stringify(value, null, 2)
}

function lineDiff(p, n) {
  if (
    typeof p !== 'string' ||
    typeof n !== 'string' ||
    !`${p}${n}`.includes('\n')
  )
    return `      P: ${show(p).replaceAll('\n', '\n         ')}\n      N: ${show(n).replaceAll('\n', '\n         ')}`
  return diff
    .diffLines(p, n)
    .flatMap((part) =>
      part.value
        .replace(/\n$/, '')
        .split('\n')
        .map(
          (line) =>
            `      ${part.added ? 'N+' : part.removed ? 'P-' : '  '} ${line}`,
        ),
    )
    .join('\n')
}

const ran = { p: outcomes(pDir), n: outcomes(nDir) }
const { byKey, problems } = loadDeviations(deviationsFile)
const lines = []
const unlisted = []
const counts = { ok: 0, listed: 0, skipped: 0 }
const names = [...new Set([...ran.p.keys(), ...ran.n.keys()])].sort()

for (const name of names) {
  const p = ran.p.get(name)
  const n = ran.n.get(name)
  if (p === undefined || n === undefined) {
    problems.push(`${name}: only Runner ${p === undefined ? 'N' : 'P'} ran it`)
    continue
  }
  const broke = [p.broke && `P: ${p.broke}`, n.broke && `N: ${n.broke}`]
  if (broke.some(Boolean)) {
    problems.push(
      `${name}: a runner broke\n${broke.filter(Boolean).join('\n')}`,
    )
    continue
  }
  if ('skipped' in p || 'skipped' in n) {
    if (p.skipped === n.skipped) {
      counts.skipped++
      lines.push(`skipped  ${name}: ${p.skipped}`)
    } else problems.push(`${name}: skipped by one runner only`)
    continue
  }
  const found = differences(normalize(p), normalize(n))
  if (found.length === 0) {
    counts.ok++
    lines.push(`ok       ${name}`)
    continue
  }
  for (const difference of found) {
    const listed = byKey.get(`${name} ${difference.path}`)
    if (listed === undefined) {
      unlisted.push({ case: name, ...difference })
      problems.push(
        `${name} ${difference.path}: not in deviations.yaml\n${lineDiff(difference.p, difference.n)}`,
      )
      continue
    }
    listed.used = true
    if (!same(listed.p, difference.p) || !same(listed.n, difference.n)) {
      unlisted.push({
        case: name,
        ...difference,
        kind: listed.kind,
        reason: listed.reason,
        route: listed.route,
      })
      problems.push(
        `${name} ${difference.path}: listed, but the values changed\n${lineDiff(difference.p, difference.n)}`,
      )
      continue
    }
    counts.listed++
    lines.push(
      `listed   ${name} ${difference.path} (${listed.kind}): ${listed.reason.trim().split('\n')[0]}`,
    )
  }
}
// A deviation for a case left out of this run (run.sh's case filter) is neither used nor stale.
const caseFiles = new Set(
  readdirSync(resolve(here, process.env.CONFORMANCE_CASES_DIR ?? 'cases')).map(
    (file) => basename(file, '.js'),
  ),
)
for (const entry of byKey.values())
  if (!entry.used && names.includes(entry.case))
    problems.push(
      `${entry.case} ${entry.path}: listed, but P and N no longer differ there; delete the entry`,
    )
  else if (!entry.used && !caseFiles.has(entry.case))
    problems.push(
      `${entry.case} ${entry.path}: listed for a case that does not exist`,
    )

console.log(lines.join('\n'))
for (const problem of problems) console.log(`FAIL     ${problem}`)
console.log(
  `\n${counts.ok} same, ${counts.listed} listed deviations, ${counts.skipped} skipped, ${problems.length} failures`,
)
if (stubs && unlisted.length > 0)
  console.log(
    `\n# Paste into deviations.yaml with a reason:\n${yaml.stringify(
      unlisted.map(({ case: name, path, p, n, kind, reason, route }) => ({
        case: name,
        path,
        kind: kind ?? 'finding',
        route: route ?? '',
        reason: reason ?? '',
        p,
        n,
      })),
      { lineWidth: 0 },
    )}`,
  )
process.exit(problems.length === 0 ? 0 : 1)
