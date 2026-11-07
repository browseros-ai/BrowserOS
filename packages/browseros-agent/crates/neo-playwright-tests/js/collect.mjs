/** Runs the pinned upstream collector under the selected host. No browser starts here. */
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { materialize } from './overlay.mjs'
import { agent, crate, sha256, source } from './prepare.mjs'

const root = path.resolve(process.argv[2])
const overlay = path.join(root, 'overlay')
const harness = path.join(root, 'harness')
fs.mkdirSync(harness)
fs.cpSync(path.join(crate, 'js'), path.join(harness, 'js'), { recursive: true })
fs.copyFileSync(
  path.join(crate, 'corpus.json'),
  path.join(harness, 'corpus.json'),
)
fs.writeFileSync(
  path.join(harness, '.source.json'),
  JSON.stringify({ agent, source }),
)
const worker = path.join(root, 'native-worker')
fs.copyFileSync(process.env.NEO_TESTS_WORKER, worker)
fs.chmodSync(worker, 0o700)
const manifest = materialize(overlay, harness)
const catalog = path.join(root, 'catalog.json')
const result = spawnSync(
  process.execPath,
  [
    path.join(source, 'packages/playwright/cli.js'),
    'test',
    '--config',
    path.join(overlay, 'neo.config.mjs'),
    '--list',
  ],
  {
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
    timeout: 90_000,
    env: {
      ...process.env,
      PWTEST_UNDER_TEST: '1',
      PWPAGE_IMPL: '',
      NEO_TESTS_OVERLAY: overlay,
      NEO_TESTS_CATALOG: catalog,
      NEO_TESTS_CASE: '',
    },
  },
)
fs.writeFileSync(
  path.join(root, 'collection.log'),
  `${result.stdout || ''}\n${result.stderr || ''}`,
)
if (result.error || result.status !== 0)
  throw new Error(
    `Pinned upstream collection failed; see ${root}/collection.log`,
  )
const entries = JSON.parse(fs.readFileSync(catalog, 'utf8'))
if (!entries.length) throw new Error('Pinned upstream collection is empty')
const harnessFiles = Object.fromEntries(
  fs
    .readdirSync(path.join(harness, 'js'))
    .map((file) => [
      file,
      sha256(fs.readFileSync(path.join(harness, 'js', file))),
    ]),
)
const revision = spawnSync('git', ['rev-parse', 'HEAD'], {
  cwd: agent,
  encoding: 'utf8',
})
fs.writeFileSync(
  path.join(root, 'metadata.json'),
  JSON.stringify(
    {
      ...manifest,
      host: {
        name: process.versions.bun ? 'bun' : 'node',
        version: process.versions.bun || process.versions.node,
      },
      platform: process.platform,
      arch: process.arch,
      count: entries.length,
      neoRevision: revision.stdout?.trim(),
      nativeWorkerSha256: sha256(fs.readFileSync(worker)),
      harnessFiles,
      hostReason:
        'Bun 1.3.6 closes the upstream HTTP listener in TestServer.reset; Node 26 preserves it.',
    },
    null,
    2,
  ),
)
