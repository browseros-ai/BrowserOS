/** Materializes and builds the pinned upstream tree without changing the developer's checkout. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
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

export function verifySource() {
  const stamp = JSON.parse(
    fs.readFileSync(path.join(source, '.neo-source.json'), 'utf8'),
  )
  if (stamp.revision !== corpus.revision || stamp.version !== corpus.version)
    throw new Error(
      'Prepared Playwright revision differs; run js/prepare.mjs again',
    )
  for (const suite of corpus.suites) {
    const actual = sha256(fs.readFileSync(path.join(source, suite.file)))
    if (actual !== suite.sha256)
      throw new Error(`Pinned upstream source changed: ${suite.file}`)
  }
  // The stamp binds executable build products as well as source. An old lib/ directory
  // would otherwise execute a different reference while the original tests still hash correctly.
  for (const [file, digest] of Object.entries(stamp.build)) {
    if (sha256(fs.readFileSync(path.join(source, file))) !== digest)
      throw new Error(`Prepared Playwright build changed: ${file}`)
  }
  return stamp
}

function filesBelow(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const name = path.join(dir, entry.name)
    return entry.isDirectory() ? filesBelow(name) : [name]
  })
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
  run('npm', ['ci', '--no-audit', '--no-fund'], source)
  run('npm', ['run', 'build'], source)
  const built = [
    'packages/playwright/lib',
    'packages/playwright-core/lib',
  ].flatMap((dir) => filesBelow(path.join(source, dir)))
  const build = Object.fromEntries(
    built.map((file) => [
      path.relative(source, file),
      sha256(fs.readFileSync(file)),
    ]),
  )
  fs.writeFileSync(
    path.join(source, '.neo-source.json'),
    JSON.stringify(
      { revision: corpus.revision, version: corpus.version, build },
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
  prepare(process.argv[2])
