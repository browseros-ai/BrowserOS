#!/usr/bin/env node
/** Neo: Validates per-case curation against upstream collection, then edits only private copies. */
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { corpus, sha256, source } from '../js/prepare.mjs'
import {
  flattenResults,
  isGeneratedSkip,
  internalsSkipPrefix as skipPrefix,
} from './report/classify.mjs'

export const manifestPath = fileURLToPath(
  new URL('./curation.json', import.meta.url),
)
export const patchReason =
  'error diagnostics: remove wording and call logs; retain behavior and error classes'
export const entryKey = (entry) =>
  JSON.stringify([entry.file, entry.id, entry.title])
const caseEntry = (row) => ({
  file: row.suite,
  id: row.caseId,
  title: row.title,
})

export function summarize(entries) {
  const counts = Object.fromEntries(
    ['delete', 'skip', 'patch'].map((action) => [
      action,
      { total: 0, byReason: {} },
    ]),
  )
  for (const entry of entries) {
    const count = counts[entry.action]
    count.total++
    count.byReason[entry.reason] = (count.byReason[entry.reason] || 0) + 1
  }
  return counts
}

/** Collection locations identify callback bodies even when titles are computed in loops. */
export function testBodies(text, file) {
  const ts = createRequire(path.join(source, 'package.json'))('typescript')
  const ast = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true)
  if (ast.parseDiagnostics.length)
    throw new Error(`Cannot parse upstream spec: ${file}`)
  const bodies = new Map()
  function visit(node) {
    if (
      ts.isCallExpression(node) &&
      /^(it|test)(\.(skip|fixme|only))?$/.test(node.expression.getText(ast))
    ) {
      const callback = node.arguments.find(
        (arg) => ts.isArrowFunction(arg) || ts.isFunctionExpression(arg),
      )
      if (callback) {
        const position = ast.getLineAndCharacterOfPosition(node.getStart(ast))
        bodies.set(`${position.line + 1}:${position.character + 1}`, {
          start: callback.body.getStart(ast),
          end: callback.body.end,
        })
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(ast)
  return bodies
}

export function validateManifest(manifest, cases, readSource) {
  if (
    manifest.schemaVersion !== 1 ||
    manifest.pin !== corpus.revision ||
    !Array.isArray(manifest.entries)
  )
    throw new Error('Curation schema/pin differs from the prepared corpus')
  const catalog = new Map(cases.map((row) => [entryKey(caseEntry(row)), row]))
  if (catalog.size !== cases.length)
    throw new Error('Duplicate collected curation identity')
  const entries = new Map()
  const files = new Map()
  for (const entry of manifest.entries) {
    if (
      !entry ||
      !['file', 'id', 'title', 'reason'].every(
        (key) => typeof entry[key] === 'string' && entry[key].trim(),
      ) ||
      !['delete', 'skip', 'patch'].includes(entry.action)
    )
      throw new Error('Invalid curation entry')
    const key = entryKey(entry)
    if (entries.has(key)) throw new Error(`Duplicate curation entry: ${key}`)
    const row = catalog.get(key)
    if (!row) throw new Error(`Curation case drift: ${key}`)
    if (
      !corpus.suites.some((suite) => suite.include && suite.file === entry.file)
    )
      throw new Error(`Curation file outside corpus: ${entry.file}`)
    entries.set(key, entry)
    if (
      entry.patches !== undefined &&
      (!Array.isArray(entry.patches) ||
        !entry.patches.length ||
        entry.action === 'delete')
    )
      throw new Error(`Invalid curation patches: ${key}`)
    if (entry.action === 'patch' && !entry.patches?.length)
      throw new Error(`Patch entry has no patches: ${key}`)
    if (!files.has(entry.file)) {
      const text = readSource(entry.file)
      files.set(entry.file, {
        text,
        bodies: testBodies(text, entry.file),
        changes: new Map(),
      })
    }
    const file = files.get(entry.file)
    const location = `${row.location.line}:${row.location.column}`
    const body = file.bodies.get(location)
    if (!body) throw new Error(`Curation callback drift: ${key}`)
    if (!entry.patches) continue
    const content = file.text.slice(body.start, body.end)
    const changes = []
    for (const patch of entry.patches) {
      if (
        typeof patch.find !== 'string' ||
        !patch.find ||
        typeof patch.replace !== 'string'
      )
        throw new Error(`Invalid exact-text patch: ${key}`)
      const first = content.indexOf(patch.find)
      if (first === -1 || content.indexOf(patch.find, first + 1) !== -1)
        throw new Error(
          `Curation patch must occur exactly once in ${key}: ${JSON.stringify(patch.find)}`,
        )
      changes.push({
        start: body.start + first,
        end: body.start + first + patch.find.length,
        replace: patch.replace,
      })
    }
    changes.sort((a, b) => a.start - b.start)
    if (
      changes.some(
        (change, index) => index && change.start < changes[index - 1].end,
      )
    )
      throw new Error(`Overlapping curation patches: ${key}`)
    const prior = file.changes.get(location)
    if (prior && JSON.stringify(prior) !== JSON.stringify(changes))
      throw new Error(`Conflicting patches for shared callback: ${key}`)
    file.changes.set(location, changes)
  }
  // One source callback can register many tests. A patch must never silently affect an uncurated sibling.
  for (const row of cases) {
    const file = files.get(row.suite)
    const location = `${row.location.line}:${row.location.column}`
    if (file?.changes.has(location)) {
      const entry = entries.get(entryKey(caseEntry(row)))
      if (!entry || (entry.action !== 'delete' && !entry.patches))
        throw new Error(
          `Shared callback requires a patch decision: ${row.suite}: ${row.title}`,
        )
    }
  }
  return files
}

export function applyManifest(manifest, cases, root) {
  if (fs.realpathSync(root) === fs.realpathSync(source))
    throw new Error('Refusing to patch the prepared source')
  const privateFile = (file) => {
    const target = fs.realpathSync(path.join(root, file))
    const relative = path.relative(fs.realpathSync(root), target)
    if (
      relative.startsWith('..') ||
      path.isAbsolute(relative) ||
      target.startsWith(`${fs.realpathSync(source)}${path.sep}`)
    )
      throw new Error(`Curation target is not a private copy: ${file}`)
    if (fs.statSync(target).nlink !== 1)
      throw new Error(`Curation target is hard-linked: ${file}`)
    return target
  }
  const files = validateManifest(manifest, cases, (file) =>
    fs.readFileSync(privateFile(file), 'utf8'),
  )
  const writes = []
  for (const [file, plan] of files) {
    const changes = [...plan.changes.values()]
      .flat()
      .sort((a, b) => b.start - a.start)
    let patched = plan.text
    for (const change of changes)
      patched =
        patched.slice(0, change.start) +
        change.replace +
        patched.slice(change.end)
    if (changes.length) {
      testBodies(patched, file)
      writes.push({
        file,
        target: privateFile(file),
        patched,
        beforeSha256: sha256(plan.text),
        afterSha256: sha256(patched),
      })
    }
  }
  for (const write of writes) fs.writeFileSync(write.target, write.patched)
  return writes.map(({ file, beforeSha256, afterSha256 }) => ({
    file,
    beforeSha256,
    afterSha256,
  }))
}

export function collectCases(root, configPath, output) {
  const resultFile = path.join(output, 'curation-upstream.json')
  const result = spawnSync(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      configPath,
      '--list',
      '--reporter=json',
      '--grep-invert',
      '\\bheadful\\.spec\\.ts\\b(?!.*\\bHE-012\\b)',
    ],
    {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024,
      timeout: 90_000,
      env: {
        ...process.env,
        NEO_NODE_OVERLAY: root,
        NEO_NODE_OUTPUT: output,
        NEO_NODE_CURATION: '',
        PLAYWRIGHT_JSON_OUTPUT_FILE: resultFile,
        PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1',
        PWTEST_UNDER_TEST: '1',
      },
    },
  )
  if (result.error || result.status !== 0)
    throw new Error(
      `Curation upstream collection failed: ${result.error || result.stderr || result.stdout}`,
    )
  return flattenResults(
    JSON.parse(fs.readFileSync(resultFile, 'utf8')),
    corpus.suites,
  )
}

export function prepareCuration(root, configPath, output, enabled = true) {
  const cases = collectCases(root, configPath, output)
  if (!enabled)
    return {
      enabled: false,
      upstreamTotal: cases.length,
      counts: summarize([]),
    }
  const bytes = fs.readFileSync(manifestPath)
  const manifest = JSON.parse(bytes)
  const edits = applyManifest(manifest, cases, root)
  fs.writeFileSync(path.join(output, 'curation.json'), bytes)
  return {
    enabled: true,
    manifestSha256: sha256(bytes),
    upstreamTotal: cases.length,
    counts: summarize(manifest.entries),
    edits,
  }
}

/** Compose with caller exclusions; neither --grep nor --test-list can bring deletions back. */
export function curationArgs(args, output) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(output, 'curation.json'), 'utf8'),
  )
  const deleted = new Set(
    manifest.entries.filter((entry) => entry.action === 'delete').map(entryKey),
  )
  const cases = flattenResults(
    JSON.parse(
      fs.readFileSync(path.join(output, 'curation-upstream.json'), 'utf8'),
    ),
    corpus.suites,
  )
  const escapeRegex = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const deletions = cases.filter((row) => deleted.has(entryKey(caseEntry(row))))
  const needsRegex = (row) =>
    row.titles.some((title) => title.trim() !== title || /[›\r\n]/.test(title))
  const lines = deletions
    .filter((row) => !needsRegex(row))
    .map((row) => `${row.suite.replace(/^tests\//, '')} › ${row.title}`)
  // Playwright's test-list parser trims titles. Exact grep retains whitespace in those rare titles.
  const inversions = deletions
    .filter(needsRegex)
    .map(
      (row) =>
        `(?:^| )${escapeRegex(row.suite.replace(/^tests\//, ''))} ${escapeRegex(row.titles.join(' '))}(?: @\\S+)*$`,
    )
  for (const row of cases) {
    if (
      !deleted.has(entryKey(caseEntry(row))) &&
      inversions.some((pattern) =>
        new RegExp(pattern).test(
          `${row.suite.replace(/^tests\//, '')} ${row.titles.join(' ')}`,
        ),
      )
    )
      throw new Error(`Ambiguous curation selection: ${row.title}`)
  }
  const filtered = []
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--') {
      filtered.push(...args.slice(i))
      break
    }
    if (/^--test-list-invert(=|$)/.test(args[i])) {
      const file = args[i].includes('=')
        ? args[i].slice('--test-list-invert='.length)
        : args[++i]
      if (!file) throw new Error('--test-list-invert requires a file')
      lines.push(
        fs.readFileSync(path.resolve(output, 'upstream', file), 'utf8'),
      )
    } else if (args[i] === '--grep-invert') inversions.push(args[++i])
    else if (args[i].startsWith('--grep-invert='))
      inversions.push(args[i].slice('--grep-invert='.length))
    else filtered.push(args[i])
  }
  const excluded = path.join(output, 'curation-deleted.txt')
  fs.writeFileSync(excluded, `${lines.join('\n')}\n`)
  const separator = filtered.indexOf('--')
  filtered.splice(
    separator < 0 ? filtered.length : separator,
    0,
    '--test-list-invert',
    excluded,
  )
  if (inversions.length)
    filtered.splice(
      separator < 0 ? filtered.length : separator,
      0,
      '--grep-invert',
      inversions.map((value) => `(?:${value})`).join('|'),
    )
  return filtered
}

export function skipReason(entries, file, titles) {
  return entries.find(
    (entry) =>
      entry.action === 'skip' &&
      entry.file === file &&
      entry.title === titles.join(' › '),
  )?.reason
}

/** The skip the owner's order 2 adds for a case, or `null`: needs-internals, or an uncovered expectLoop check. */
function generatedSkipReason(row, key) {
  if (row.verdict === 'needs-internals') {
    if (typeof row.gapKey !== 'string' || !row.gapKey.trim())
      throw new Error(`Missing internals gap: ${key}`)
    return `${skipPrefix}${row.gapKey}`
  }
  if (row.verdict === 'skip')
    return (
      String(row.reason || '')
        .split('; ')
        // Also a skip this manifest already applied, so a curated run refreshes idempotently.
        .find((part) => isGeneratedSkip(part)) || null
    )
  return null
}

export function refreshSkips(manifest, verdictRows) {
  const rows = new Map()
  for (const row of verdictRows) {
    const key = entryKey(caseEntry(row))
    if (rows.has(key)) throw new Error(`Duplicate verdict identity: ${key}`)
    rows.set(key, row)
  }
  const entries = new Map(
    manifest.entries.map((entry) => [entryKey(entry), entry]),
  )
  if (entries.size !== manifest.entries.length)
    throw new Error('Duplicate curation entry')
  for (const [key, row] of rows) {
    const prior = entries.get(key)
    if (prior?.action === 'delete') continue
    const reason = generatedSkipReason(row, key)
    // A generated skip keeps the patch it replaced, and that patch's own reason, so clearing
    // the skip later restores the entry exactly.
    const patched = prior?.patches
      ? {
          patches: prior.patches,
          patchReason:
            prior.action === 'patch' ? prior.reason : prior.patchReason,
        }
      : {}
    if (reason) {
      entries.set(key, {
        ...caseEntry(row),
        action: 'skip',
        reason,
        ...patched,
      })
    } else if (prior?.action === 'skip' && isGeneratedSkip(prior.reason)) {
      if (prior.patches) {
        const { patchReason: restored, ...rest } = prior
        entries.set(key, {
          ...rest,
          action: 'patch',
          reason: restored ?? patchReason,
        })
      } else entries.delete(key)
    }
  }
  return {
    ...manifest,
    entries: [...entries.values()].sort((a, b) =>
      entryKey(a).localeCompare(entryKey(b)),
    ),
  }
}

export async function main(args = process.argv.slice(2)) {
  if (args.length !== 2 || args[0] !== 'skip-from-verdicts')
    throw new Error(
      'Usage: node curation.mjs skip-from-verdicts <verdicts.jsonl>',
    )
  const rows = fs
    .readFileSync(args[1], 'utf8')
    .split('\n')
    .filter((line) => line.trim())
    .map(JSON.parse)
  const manifest = refreshSkips(
    JSON.parse(fs.readFileSync(manifestPath, 'utf8')),
    rows,
  )
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-curation-'))
  try {
    const { materialize } = await import('./overlay.mjs')
    const root = path.join(scratch, 'upstream')
    materialize(root, {
      reference: true,
      output: scratch,
      curation: false,
    })
    validateManifest(
      manifest,
      flattenResults(
        JSON.parse(
          fs.readFileSync(path.join(scratch, 'curation-upstream.json'), 'utf8'),
        ),
        corpus.suites,
      ),
      (file) => fs.readFileSync(path.join(root, file), 'utf8'),
    )
    const temporary = `${manifestPath}.${process.pid}.tmp`
    try {
      fs.writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`, {
        flag: 'wx',
      })
      fs.renameSync(temporary, manifestPath)
    } finally {
      fs.rmSync(temporary, { force: true })
    }
    console.log(JSON.stringify(summarize(manifest.entries)))
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true })
  }
}

if (
  process.argv[1] &&
  fs.existsSync(process.argv[1]) &&
  fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`curation: ${error.message}`)
    process.exitCode = 1
  })
}
