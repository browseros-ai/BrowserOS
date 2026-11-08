/**
 * Keeps `vendor/playwright/` an unchanged copy of Playwright's injected script at the pinned
 * commit. The file list is not hand-kept: it is the import closure of `ROOT_ENTRIES`,
 * `packages/injected/src/injectedScript.ts` (which pulls in `utilityScript.ts`, the
 * `@isomorphic/*` files it names, the type-only `@protocol/*` files and `highlight.css`) and
 * `bindingsController.ts` (the page half of `exposeBinding`) and `clock.ts` (the page half of
 * `page.clock`), plus Playwright's LICENSE and NOTICE. Each file keeps its path under Playwright's `packages/`, so
 * relative imports between vendored files resolve unchanged and tsconfig's `paths` map the
 * `@injected`, `@isomorphic` and `@protocol` aliases. The generated VENDOR.md names the commit
 * and lists every file with its sha256.
 *
 * Usage, from crates/neo-playwright-injected:
 *   bun run sync-vendor -- --from /Users/shadowfax/code/oss/playwright   # copy + VENDOR.md
 *   bun run test                                                         # sync-vendor --check
 *
 * `--from` refuses a checkout whose HEAD is not the pinned commit, reads every file from that
 * commit's objects (so local edits in the checkout never leak in), and fails if a written byte
 * differs from upstream. `--check` runs in neo-check, which must work without a Playwright
 * checkout: it re-hashes every vendored file against VENDOR.md and fails on any edit, addition
 * or removal.
 */

import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { dirname, join, posix, relative } from 'node:path'

const PINNED_COMMIT = '26137face0500cacddf90c767dc1364da7262559'
const ROOT_ENTRIES = [
  'packages/injected/src/injectedScript.ts',
  'packages/injected/src/bindingsController.ts',
  'packages/injected/src/clock.ts',
]
const LEGAL_FILES = ['LICENSE', 'NOTICE']
/** Playwright's tsconfig `paths` for the injected closure, as checkout paths. */
const ALIASES: Record<string, string> = {
  '@injected/': 'packages/injected/src/',
  '@isomorphic/': 'packages/isomorphic/',
  '@protocol/': 'packages/protocol/src/',
}
const RESOLVE_SUFFIXES = ['', '.ts', '.d.ts', '/index.ts']

const packageDir = join(import.meta.dir, '..')
const vendorDir = join(packageDir, 'vendor/playwright')
const vendorManifest = join(vendorDir, 'VENDOR.md')

/** A vendored file: its path under `vendor/playwright/` and the sha256 of its bytes. */
type VendoredFile = { path: string; sha256: string }

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex')
}

function git(checkout: string, args: string[]): Buffer {
  return execFileSync('git', ['-C', checkout, ...args], {
    maxBuffer: 64 * 1024 * 1024,
  })
}

/** Where a checkout path lands under `vendor/playwright/`: `packages/` is dropped. */
function vendoredPath(checkoutPath: string): string {
  return checkoutPath.replace(/^packages\//, '')
}

/**
 * Module specifiers of every static `import`/`export ... from` statement, type-only ones
 * included: `tsc` needs the type-only files too. Statements are matched from the start of a
 * line, so a quoted `'import'` inside code or a comment never counts.
 */
function importSpecifiers(source: string): string[] {
  if (/\bimport\s*\(/.test(source) || /\brequire\s*\(/.test(source)) {
    throw new Error(
      'a dynamic import or require: this closure walker reads static imports only',
    )
  }
  const fromClause = /^\s*(?:import|export)\s[^;'"]*?\bfrom\s*'([^']+)'/gm
  const sideEffect = /^\s*import\s*'([^']+)'/gm
  return [...source.matchAll(fromClause), ...source.matchAll(sideEffect)].map(
    (match) => match[1] as string,
  )
}

/**
 * Follows imports from `ROOT_ENTRIES` through the files of `commit`, resolving Playwright's
 * aliases and dropping a `?inline` query (the CSS stays a file; build.ts inlines it). Bare
 * specifiers are npm packages: the closure has only `import type * as yamlTypes from 'yaml'`,
 * which the workspace's own `yaml` satisfies for `tsc` and the bundle never loads.
 */
function importClosure(checkout: string, commit: string): string[] {
  const tracked = new Set(
    git(checkout, ['ls-tree', '-r', '--name-only', commit, 'packages'])
      .toString('utf8')
      .split('\n')
      .filter(Boolean),
  )
  const resolve = (from: string, specifier: string): string | undefined => {
    const bare = specifier.replace(/\?inline$/, '')
    const alias = Object.keys(ALIASES).find((prefix) => bare.startsWith(prefix))
    let base: string
    if (alias) base = ALIASES[alias] + bare.slice(alias.length)
    else if (bare.startsWith('.'))
      base = posix.normalize(posix.join(posix.dirname(from), bare))
    else return undefined
    const found = RESOLVE_SUFFIXES.map((suffix) => base + suffix).find((path) =>
      tracked.has(path),
    )
    if (!found)
      throw new Error(`${from}: cannot resolve '${specifier}' at ${commit}`)
    return found
  }

  const closure = new Set<string>()
  const pending = [...ROOT_ENTRIES]
  for (let file = pending.pop(); file; file = pending.pop()) {
    if (closure.has(file)) continue
    closure.add(file)
    if (!/\.tsx?$/.test(file)) continue
    const source = git(checkout, ['cat-file', 'blob', `${commit}:${file}`])
    for (const specifier of importSpecifiers(source.toString('utf8'))) {
      const resolved = resolve(file, specifier)
      if (resolved) pending.push(resolved)
    }
  }
  return [...closure].sort()
}

function renderManifest(commit: string, files: VendoredFile[]): string {
  const rows = files.map((file) => `| \`${file.path}\` | \`${file.sha256}\` |`)
  return [
    '# Vendored Playwright',
    '',
    `Copied unchanged from https://github.com/microsoft/playwright at commit \`${commit}\` by`,
    '`bun run sync-vendor -- --from <checkout>`, which generated this file. Do not edit anything',
    'here: `bun run test` fails on any edit, addition or removal.',
    '',
    'The files are the import closure of `packages/injected/src/injectedScript.ts`, at their',
    "paths under Playwright's `packages/`, plus its LICENSE and NOTICE.",
    '',
    '| File | sha256 |',
    '| --- | --- |',
    ...rows,
    '',
  ].join('\n')
}

function sync(checkout: string): void {
  const head = git(checkout, ['rev-parse', 'HEAD']).toString('utf8').trim()
  if (head !== PINNED_COMMIT) {
    throw new Error(
      `${checkout} is at ${head}, not the pinned Playwright commit ${PINNED_COMMIT}`,
    )
  }
  const sources = [
    ...importClosure(checkout, PINNED_COMMIT),
    ...LEGAL_FILES,
  ].map((checkoutPath) => ({
    path: vendoredPath(checkoutPath),
    bytes: git(checkout, [
      'cat-file',
      'blob',
      `${PINNED_COMMIT}:${checkoutPath}`,
    ]),
  }))

  rmSync(vendorDir, { recursive: true, force: true })
  const files: VendoredFile[] = []
  for (const { path, bytes } of sources) {
    const target = join(vendorDir, path)
    mkdirSync(dirname(target), { recursive: true })
    writeFileSync(target, bytes)
    const written = readFileSync(target)
    if (!written.equals(bytes))
      throw new Error(`${path}: written bytes differ from upstream`)
    files.push({ path, sha256: sha256(written) })
  }
  files.sort((a, b) => a.path.localeCompare(b.path))
  writeFileSync(vendorManifest, renderManifest(PINNED_COMMIT, files))
  console.log(`sync-vendor: ${files.length} files at ${PINNED_COMMIT}`)
}

function listVendored(): string[] {
  return readdirSync(vendorDir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(vendorDir, join(entry.parentPath, entry.name)))
    .filter((path) => path !== 'VENDOR.md')
    .sort()
}

function check(): void {
  const manifest = readFileSync(vendorManifest, 'utf8')
  if (!manifest.includes(`commit \`${PINNED_COMMIT}\``)) {
    throw new Error(
      `VENDOR.md does not name the pinned commit ${PINNED_COMMIT}`,
    )
  }
  const listed = new Map(
    [...manifest.matchAll(/^\| `([^`]+)` \| `([0-9a-f]{64})` \|$/gm)].map(
      (match) => [match[1] as string, match[2] as string],
    ),
  )
  const present = listVendored()
  const problems = [
    ...present
      .filter((path) => !listed.has(path))
      .map((path) => `${path}: not in VENDOR.md`),
    ...[...listed.keys()]
      .filter((path) => !present.includes(path))
      .map((path) => `${path}: listed in VENDOR.md but missing`),
    ...present
      .filter(
        (path) =>
          listed.has(path) &&
          listed.get(path) !== sha256(readFileSync(join(vendorDir, path))),
      )
      .map((path) => `${path}: edited (sha256 differs from VENDOR.md)`),
  ]
  if (problems.length > 0) {
    throw new Error(
      `vendor/playwright is not Playwright's ${PINNED_COMMIT}:\n  ${problems.join('\n  ')}\n` +
        'Re-run `bun run sync-vendor -- --from <playwright checkout>`.',
    )
  }
  console.log(`sync-vendor: ${present.length} files match VENDOR.md`)
}

const args = process.argv.slice(2).filter((arg) => arg !== '--')
const fromIndex = args.indexOf('--from')
if (args.length === 1 && args[0] === '--check') check()
else if (args.length === 2 && fromIndex === 0 && args[1]) sync(args[1])
else {
  throw new Error(
    'usage: bun scripts/sync-vendor.ts --from <playwright checkout> | --check',
  )
}
