/** Materializes and builds the pinned upstream tree without changing the developer's checkout. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const crate = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
)
export const corpus = JSON.parse(
  fs.readFileSync(path.join(crate, 'corpus.json'), 'utf8'),
)
const frozen = fs.existsSync(path.join(crate, '.source.json'))
  ? JSON.parse(fs.readFileSync(path.join(crate, '.source.json'), 'utf8'))
  : undefined
export const agent = frozen?.agent || path.resolve(crate, '../..')
export const source = path.resolve(
  process.env.NEO_TESTS_PLAYWRIGHT_SOURCE ||
    frozen?.source ||
    path.join(agent, 'target/neo-playwright-tests/source'),
)
export const sha256 = (bytes) =>
  createHash('sha256').update(bytes).digest('hex')

const buildDirs = ['packages/playwright/lib', 'packages/playwright-core/lib']
const inputTrees = ['tests', 'packages/isomorphic']
const inputPackages = [
  'packages/playwright',
  'packages/playwright-core',
  'packages/playwright-test',
]
const inputFiles = ['package.json', 'package-lock.json', 'tsconfig.json']

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: 'inherit',
    env: { ...process.env, PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1' },
  })
  if (result.error || result.status !== 0)
    throw new Error(
      `${command} failed (${result.status}): ${result.error || ''}`,
    )
}

export function verifySource({
  root = source,
  pin = corpus,
  allowLegacy = false,
} = {}) {
  const stamp = JSON.parse(
    fs.readFileSync(path.join(root, '.neo-source.json'), 'utf8'),
  )
  if (stamp.revision !== pin.revision || stamp.version !== pin.version)
    throw new Error(
      'Prepared Playwright revision differs; run js/prepare.mjs again',
    )
  for (const suite of pin.suites) {
    const actual = sha256(fs.readFileSync(path.join(root, suite.file)))
    if (actual !== suite.sha256)
      throw new Error(`Pinned upstream source changed: ${suite.file}`)
  }
  // The stamp binds executable build products as well as source. An old lib/ directory
  // would otherwise execute a different reference while the original tests still hash correctly.
  equalFiles(
    stamp.build,
    hashes(
      root,
      buildDirs.flatMap((dir) => filesBelow(path.join(root, dir))),
    ),
    'build',
  )
  if (stamp.schemaVersion !== 2) {
    if (allowLegacy && stamp.schemaVersion === undefined) return stamp
    throw new Error(
      'Prepared Playwright stamp lacks fixtures/assets; run js/prepare.mjs --upgrade-stamp /path/to/playwright-source',
    )
  }
  equalFiles(stamp.inputs, sourceInputs(root), 'input')
  return stamp
}

function filesBelow(dir, ignored = new Set()) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const name = path.join(dir, entry.name)
    if (ignored.has(name)) return []
    if (entry.isDirectory()) return filesBelow(name, ignored)
    if (!entry.isFile())
      throw new Error(`Pinned source must contain ordinary files: ${name}`)
    return [name]
  })
}

function hashes(root, files) {
  return Object.fromEntries(
    files
      .sort()
      .map((file) => [
        path.relative(root, file).split(path.sep).join('/'),
        fileDigest(file),
      ]),
  )
}

function fileDigest(file) {
  if (!fs.lstatSync(file).isFile())
    throw new Error(`Pinned source must contain ordinary files: ${file}`)
  return sha256(fs.readFileSync(file))
}

/** Tests include executable fixtures, TestServer and every served asset. Package entrypoints
 * and source helpers are separate from lib builds, so changing either invalidates the pin. */
function sourceInputs(root) {
  // npm ci installs a nested stable runner under tests. Its dependency tree is lockfile
  // managed; hashing it as original source would reject every correctly prepared archive.
  const ignored = new Set([
    path.join(root, 'tests/playwright-test/stable-test-runner/node_modules'),
  ])
  const files = inputTrees.flatMap((dir) =>
    filesBelow(path.join(root, dir), ignored),
  )
  for (const dir of inputPackages)
    for (const entry of fs.readdirSync(path.join(root, dir), {
      withFileTypes: true,
    }))
      if (!entry.isDirectory()) files.push(path.join(root, dir, entry.name))
  files.push(...inputFiles.map((file) => path.join(root, file)))
  return hashes(root, files)
}

function equalFiles(expected, actual, kind) {
  if (
    !expected ||
    typeof expected !== 'object' ||
    !Object.keys(expected).length
  )
    throw new Error(`Prepared Playwright ${kind} inventory is missing`)
  for (const file of new Set([
    ...Object.keys(expected),
    ...Object.keys(actual),
  ]))
    if (expected[file] !== actual[file])
      throw new Error(`Prepared Playwright ${kind} changed: ${file}`)
}

/** Adds provenance to a previously verified build without replacing bytes used by an active
 * sweep. The only write is an atomic stamp replacement after comparison with git archive. */
export function upgradeStamp(checkout, { root = source, pin = corpus } = {}) {
  if (!checkout)
    throw new Error(
      'Usage: node js/prepare.mjs --upgrade-stamp /path/to/playwright-source',
    )
  const stamp = verifySource({ root, pin, allowLegacy: true })
  if (stamp.schemaVersion === 2) return stamp
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-source-stamp-'))
  try {
    const archive = path.join(scratch, 'source.tar')
    const expectedRoot = path.join(scratch, 'source')
    fs.mkdirSync(expectedRoot)
    run(
      'git',
      [
        'archive',
        '--format=tar',
        `--output=${archive}`,
        pin.revision,
        '--',
        ...inputTrees,
        ...inputPackages,
        ...inputFiles,
      ],
      checkout,
    )
    run('tar', ['-xf', archive, '-C', expectedRoot], checkout)
    const inputs = sourceInputs(expectedRoot)
    equalFiles(inputs, sourceInputs(root), 'input')
    const upgraded = { ...stamp, schemaVersion: 2, inputs }
    const temporary = path.join(root, `.neo-source.${process.pid}.json`)
    try {
      fs.writeFileSync(temporary, JSON.stringify(upgraded, null, 2), {
        flag: 'wx',
      })
      fs.renameSync(temporary, path.join(root, '.neo-source.json'))
    } finally {
      fs.rmSync(temporary, { force: true })
    }
    return verifySource({ root, pin })
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true })
  }
}

export function prepare(checkout) {
  if (!checkout)
    throw new Error('Usage: bun js/prepare.mjs /path/to/playwright-source')
  if (fs.existsSync(source))
    throw new Error(`Refusing to replace existing source directory ${source}`)
  fs.mkdirSync(source, { recursive: true })
  const archive = `${source}.tar`
  run(
    'git',
    ['archive', '--format=tar', `--output=${archive}`, corpus.revision],
    checkout,
  )
  run('tar', ['-xf', archive, '-C', source], checkout)
  fs.unlinkSync(archive)
  const inputs = sourceInputs(source)
  run('npm', ['ci', '--no-audit', '--no-fund'], source)
  run('npm', ['run', 'build'], source)
  const build = hashes(
    source,
    buildDirs.flatMap((dir) => filesBelow(path.join(source, dir))),
  )
  fs.writeFileSync(
    path.join(source, '.neo-source.json'),
    JSON.stringify(
      {
        schemaVersion: 2,
        revision: corpus.revision,
        version: corpus.version,
        build,
        inputs,
      },
      null,
      2,
    ),
  )
  verifySource()
  console.log(`Prepared Playwright ${corpus.revision} at ${source}`)
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  if (process.argv[2] === '--upgrade-stamp') {
    upgradeStamp(process.argv[3])
    console.log(`Verified fixture/asset stamp at ${source}`)
  } else prepare(process.argv[2])
