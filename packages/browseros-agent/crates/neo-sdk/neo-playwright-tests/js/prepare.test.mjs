/** Neo: Offline provenance regression tests use a tiny real git archive, never the shared build. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { materialize } from './overlay.mjs'
import { sha256, upgradeStamp, verifySource } from './prepare.mjs'

function fixture(t) {
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-prepare-test-'))
  t.after(() => fs.rmSync(scratch, { recursive: true, force: true }))
  const checkout = path.join(scratch, 'checkout')
  const root = path.join(scratch, 'prepared')
  const files = {
    'package.json': '{"private":true}',
    'package-lock.json': '{}',
    'tsconfig.json': '{}',
    'packages/playwright/package.json': '{}',
    'packages/playwright/cli.js': 'export const cli = true;',
    'packages/playwright-core/package.json': '{}',
    'packages/playwright-core/index.js': 'export const core = true;',
    'packages/playwright-test/package.json': '{}',
    'packages/playwright-test/index.js': 'export const runner = true;',
    'packages/isomorphic/helper.ts': 'export const helper = true;',
    'tests/config/browserTest.ts':
      'export const browserTest = originalFixtureGraph;',
    'tests/config/serverFixtures.ts':
      'const port = 8907 + workerInfo.workerIndex * 4;',
    'tests/config/testserver/index.ts': 'export class TestServer {}',
    'tests/config/helper.ts': 'export const helper = true;',
    'tests/playwright-test/stable-test-runner/package.json': '{}',
    'tests/playwright-test/stable-test-runner/package-lock.json': '{}',
    'tests/assets/empty.html': '<html><body></body></html>',
    'tests/library/headful.spec.ts':
      "it.skip(({ headless }) => headless, 'avoid popping windows in headless mode');",
    'tests/page/example.spec.ts':
      'test("original", async ({ page }) => page.title());',
    'tests/page/elementhandle-misc.spec.ts':
      'test("handle", async ({ page }) => page.$("div"));',
    'tests/page/page-evaluate.spec.ts':
      'test("evaluate", async ({ page }) => page.evaluate(() => 1));',
  }
  for (const [file, text] of Object.entries(files)) {
    const full = path.join(checkout, file)
    fs.mkdirSync(path.dirname(full), { recursive: true })
    fs.writeFileSync(full, text)
  }
  const git = (args) => {
    const result = spawnSync(
      'git',
      ['-c', 'core.hooksPath=/dev/null', ...args],
      {
        cwd: checkout,
        encoding: 'utf8',
      },
    )
    assert.equal(result.status, 0, result.stderr)
    return result.stdout.trim()
  }
  git(['init', '-q'])
  git(['add', '.'])
  git([
    '-c',
    'user.name=Harness Test',
    '-c',
    'user.email=harness@example.invalid',
    'commit',
    '-qm',
    'fixture',
  ])
  const pin = {
    revision: git(['rev-parse', 'HEAD']),
    version: 'fixture',
    suites: Object.keys(files)
      .filter((file) => file.startsWith('tests/page/'))
      .map((file) => ({
        file,
        include: true,
        sha256: sha256(files[file]),
      })),
  }
  fs.cpSync(checkout, root, {
    recursive: true,
    filter: (file) => path.basename(file) !== '.git',
  })
  fs.mkdirSync(path.join(root, 'node_modules'))
  const stableBin = path.join(
    root,
    'tests/playwright-test/stable-test-runner/node_modules/.bin',
  )
  fs.mkdirSync(stableBin, { recursive: true })
  fs.symlinkSync(
    path.join(root, 'packages/playwright/cli.js'),
    path.join(stableBin, 'playwright'),
  )
  const build = {}
  for (const dir of [
    'packages/playwright/lib',
    'packages/playwright-core/lib',
  ]) {
    fs.mkdirSync(path.join(root, dir), { recursive: true })
    const file = `${dir}/bundle.js`
    fs.writeFileSync(path.join(root, file), 'verified build bytes')
    build[file] = sha256('verified build bytes')
  }
  fs.writeFileSync(
    path.join(root, '.neo-source.json'),
    JSON.stringify({ ...pin, suites: undefined, build }),
  )
  return { scratch, checkout, root, pin, files }
}

test('legacy stamp upgrade verifies git archive without rebuilding or replacing source bytes', (t) => {
  const { checkout, root, pin, files } = fixture(t)
  assert.throws(() => verifySource({ root, pin }), /lacks fixtures\/assets/)
  const built = path.join(root, 'packages/playwright-core/lib/bundle.js')
  const before = fs.statSync(built)
  const stamp = upgradeStamp(checkout, { root, pin })
  assert.equal(stamp.schemaVersion, 2)
  assert.equal(
    stamp.inputs['tests/config/testserver/index.ts'],
    sha256(files['tests/config/testserver/index.ts']),
  )
  assert.equal(
    stamp.inputs['tests/assets/empty.html'],
    sha256(files['tests/assets/empty.html']),
  )
  for (const [file, text] of Object.entries(files))
    assert.equal(fs.readFileSync(path.join(root, file), 'utf8'), text)
  assert.equal(fs.statSync(built).mtimeMs, before.mtimeMs)
  assert.deepEqual(verifySource({ root, pin }), stamp)
  assert.deepEqual(upgradeStamp(checkout, { root, pin }), stamp)
})

test('upgrade rejects preexisting helper or asset drift without changing the legacy stamp', (t) => {
  const { checkout, root, pin, files } = fixture(t)
  const stampFile = path.join(root, '.neo-source.json')
  const before = fs.readFileSync(stampFile, 'utf8')
  for (const file of [
    'tests/config/helper.ts',
    'tests/config/testserver/index.ts',
    'tests/assets/empty.html',
  ]) {
    fs.appendFileSync(path.join(root, file), 'drift')
    assert.throws(() => upgradeStamp(checkout, { root, pin }), /input changed/)
    assert.equal(fs.readFileSync(stampFile, 'utf8'), before)
    fs.writeFileSync(path.join(root, file), files[file])
  }
})

test('verified inventories reject executable, asset, entrypoint, added-file and build drift', (t) => {
  const { checkout, root, pin, files } = fixture(t)
  upgradeStamp(checkout, { root, pin })
  for (const file of [
    'tests/config/helper.ts',
    'tests/config/testserver/index.ts',
    'tests/assets/empty.html',
    'packages/playwright-core/index.js',
    'packages/isomorphic/helper.ts',
  ]) {
    fs.appendFileSync(path.join(root, file), 'drift')
    assert.throws(() => verifySource({ root, pin }), /input changed/)
    fs.writeFileSync(path.join(root, file), files[file])
  }
  const added = path.join(root, 'tests/config/injected.ts')
  fs.writeFileSync(added, 'added executable')
  assert.throws(() => verifySource({ root, pin }), /input changed/)
  fs.unlinkSync(added)
  const hiddenAsset = path.join(root, 'tests/assets/node_modules/untracked.js')
  fs.mkdirSync(path.dirname(hiddenAsset), { recursive: true })
  fs.writeFileSync(hiddenAsset, 'not a managed dependency')
  assert.throws(() => verifySource({ root, pin }), /input changed/)
  fs.rmSync(path.dirname(hiddenAsset), { recursive: true })
  fs.writeFileSync(
    path.join(root, 'packages/playwright/lib/extra.js'),
    'extra executable',
  )
  assert.throws(() => verifySource({ root, pin }), /build changed/)
})

test('overlay preserves the full original fixture alias and fingerprints its executed inputs', (t) => {
  const { checkout, root, pin, scratch, files } = fixture(t)
  upgradeStamp(checkout, { root, pin })
  const harness = path.join(scratch, 'harness')
  fs.mkdirSync(path.join(harness, 'js'), { recursive: true })
  fs.writeFileSync(
    path.join(harness, 'js/fixtures.mjs'),
    'export const fixture = true;',
  )
  fs.writeFileSync(path.join(harness, 'corpus.json'), JSON.stringify(pin))
  const overlay = path.join(scratch, 'overlay')
  const manifest = materialize(overlay, harness, { upstream: root, pin })
  const config = fs.readFileSync(path.join(overlay, 'neo.config.mjs'), 'utf8')
  for (const suite of pin.suites) {
    assert.ok(config.includes(suite.file.replace(/^tests\//, '')))
    assert.equal(
      fs.readFileSync(path.join(overlay, suite.file), 'utf8'),
      files[suite.file],
    )
  }
  assert.ok(
    fs
      .readFileSync(path.join(overlay, 'tests/config/browserTest.ts'), 'utf8')
      .includes('js/fixtures.mjs'),
  )
  assert.ok(
    fs
      .readFileSync(
        path.join(overlay, 'tests/config/serverFixtures.ts'),
        'utf8',
      )
      .includes('Number(process.env.NEO_TESTS_PORT)'),
  )
  assert.equal(
    fs.readFileSync(path.join(overlay, manifest.fixtureAlias.file), 'utf8'),
    files['tests/config/browserTest.ts'],
  )
  assert.equal(
    fs.readFileSync(path.join(overlay, 'tests/page/example.spec.ts'), 'utf8'),
    files['tests/page/example.spec.ts'],
  )
  assert.equal(
    manifest.harnessFiles['js/fixtures.mjs'],
    sha256('export const fixture = true;'),
  )
  assert.equal(
    manifest.configSha256,
    sha256(fs.readFileSync(path.join(overlay, 'neo.config.mjs'))),
  )
  fs.appendFileSync(
    path.join(root, 'tests/config/testserver/index.ts'),
    'drift',
  )
  assert.throws(
    () =>
      materialize(path.join(scratch, 'rejected'), harness, {
        upstream: root,
        pin,
      }),
    /input changed/,
  )
  assert.equal(fs.existsSync(path.join(scratch, 'rejected')), false)
})

test('every included suite digest is verified before creating an overlay', (t) => {
  const { checkout, root, pin, scratch, files } = fixture(t)
  upgradeStamp(checkout, { root, pin })
  for (const suite of pin.suites) {
    fs.appendFileSync(path.join(root, suite.file), 'drift')
    const overlay = path.join(scratch, 'rejected')
    assert.throws(
      () => materialize(overlay, undefined, { upstream: root, pin }),
      /Pinned upstream source changed/,
    )
    assert.equal(fs.existsSync(overlay), false)
    fs.writeFileSync(path.join(root, suite.file), files[suite.file])
  }
})
