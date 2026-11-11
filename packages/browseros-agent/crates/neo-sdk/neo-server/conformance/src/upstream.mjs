/** Materializes test-only sources by object id. The owner's checkout and the shipped engine pin
 * are independent inputs: neither HEAD movement nor an npm upgrade may silently change the suite. */
import { execFileSync } from 'node:child_process'
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  rmSync,
} from 'node:fs'
import { createRequire } from 'node:module'
import { join, resolve } from 'node:path'

const require = createRequire(import.meta.url)
export const thirdParty = resolve(
  import.meta.dirname,
  '../../../neo-playwright/third_party/playwright',
)
export const sha = (bytes) =>
  require('node:crypto').createHash('sha256').update(bytes).digest('hex')
const git = (checkout, ...args) =>
  execFileSync('git', ['-C', checkout, ...args], {
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, GIT_NO_LAZY_FETCH: '1', GIT_OPTIONAL_LOCKS: '0' },
  })
export function materialize({ checkout, cache }) {
  const [tag, commit] = readFileSync(join(thirdParty, 'VERSION'), 'utf8')
    .trim()
    .split('\n')
  const before = Buffer.concat([
    git(checkout, 'rev-parse', 'HEAD'),
    git(checkout, 'status', '--porcelain=v1', '-z'),
  ])
  if (
    git(checkout, 'rev-parse', `${tag}^{commit}`).toString().trim() !== commit
  )
    throw new Error('VERSION tag/commit mismatch')
  const dir = join(cache, 'upstream', commit)
  let result, failure
  try {
    if (!existsSync(join(dir, 'package.json'))) {
      mkdirSync(dir, { recursive: true })
      try {
        const archive = git(
          checkout,
          'archive',
          commit,
          'package.json',
          'tests/page',
          'tests/config',
          'tests/assets',
        )
        execFileSync('tar', ['-x', '-C', dir], {
          input: archive,
          maxBuffer: 64 * 1024 * 1024,
        })
      } catch (error) {
        rmSync(dir, { recursive: true, force: true })
        throw error
      }
    }
    // A directory named by a commit is not proof that its cached files still have that
    // commit's bytes. Verify Git blob identities and reject extra files before loading code.
    const records = git(
      checkout,
      'ls-tree',
      '-r',
      '-z',
      commit,
      '--',
      'package.json',
      'tests/page',
      'tests/config',
      'tests/assets',
    )
      .toString()
      .split('\0')
      .filter(Boolean)
    const expected = new Set()
    for (const record of records) {
      const split = record.indexOf('\t'),
        metadata = record.slice(0, split),
        rel = record.slice(split + 1),
        [mode, type, object] = metadata.split(' ')
      if (type !== 'blob')
        throw new Error(`unsupported upstream tree entry: ${rel}`)
      expected.add(rel)
      const file = join(dir, rel)
      const bytes =
        mode === '120000' ? Buffer.from(readlinkSync(file)) : readFileSync(file)
      if (mode !== '120000' && !lstatSync(file).isFile())
        throw new Error(`upstream file type drift: ${rel}`)
      const blob = require('node:crypto')
        .createHash('sha1')
        .update(`blob ${bytes.length}\0`)
        .update(bytes)
        .digest('hex')
      if (blob !== object)
        throw new Error(`cached upstream source drift: ${rel}`)
    }
    const files = (root, rel = '') =>
      readdirSync(join(root, rel), { withFileTypes: true }).flatMap((entry) =>
        entry.isDirectory()
          ? files(root, join(rel, entry.name))
          : [join(rel, entry.name)],
      )
    for (const rel of files(dir))
      if (!expected.has(rel))
        throw new Error(`untracked upstream input: ${rel}`)
    const version = tag.replace(/^v/, '')
    if (JSON.parse(readFileSync(join(dir, 'package.json'))).version !== version)
      throw new Error('upstream package/version mismatch')
    for (const name of ['playwright', 'playwright-core', '@playwright/test']) {
      if (require(`${name}/package.json`).version !== version)
        throw new Error(`${name} does not match ${tag}`)
    }
    const walk = (root, rel = '') =>
      readdirSync(join(root, rel), { withFileTypes: true }).flatMap((e) =>
        e.isDirectory() ? walk(root, join(rel, e.name)) : [join(rel, e.name)],
      )
    for (const file of walk(join(thirdParty, 'packages'))) {
      const rel = `packages/${file}`
      if (
        !readFileSync(join(thirdParty, rel)).equals(
          git(checkout, 'show', `${commit}:${rel}`),
        )
      )
        throw new Error(`injected source drift: ${rel}`)
    }
    result = { dir, commit, tag, version }
  } catch (error) {
    failure = error
  }
  {
    const after = Buffer.concat([
      git(checkout, 'rev-parse', 'HEAD'),
      git(checkout, 'status', '--porcelain=v1', '-z'),
    ])
    if (!before.equals(after))
      throw new Error('Playwright checkout changed during materialization')
  }
  if (failure) throw failure
  return result
}
