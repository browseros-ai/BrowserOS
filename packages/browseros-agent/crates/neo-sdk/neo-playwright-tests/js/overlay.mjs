/** Neo: Physical fixture overlay keeps upstream spec modules and their lexical closures intact. */
import fs from 'node:fs'
import path from 'node:path'
import { corpus, crate, sha256, source, verifySource } from './prepare.mjs'

export function materialize(
  root,
  harness = crate,
  { upstream = source, pin = corpus } = {},
) {
  const stamp = verifySource({ root: upstream, pin })
  if (fs.existsSync(root)) throw new Error(`Overlay already exists: ${root}`)
  fs.mkdirSync(root, { recursive: true })
  // The upstream root is CommonJS. Without this boundary the overlay inherits this
  // workspace's type=module and Playwright compiles its untouched helpers differently.
  fs.writeFileSync(
    path.join(root, 'package.json'),
    JSON.stringify({ private: true, type: 'commonjs' }),
  )
  fs.cpSync(path.join(upstream, 'tests'), path.join(root, 'tests'), {
    recursive: true,
  })
  for (const [file, digest] of Object.entries(stamp.inputs))
    if (
      file.startsWith('tests/') &&
      sha256(fs.readFileSync(path.join(root, file))) !== digest
    )
      throw new Error(`Pinned source changed while copying overlay: ${file}`)
  for (const name of ['packages', 'node_modules'])
    fs.symlinkSync(path.join(upstream, name), path.join(root, name), 'dir')
  const edits = []
  const replace = (file, before, after) => {
    const name = path.join(root, file)
    const text = fs.readFileSync(name, 'utf8')
    if (text.split(before).length !== 2)
      throw new Error(`Overlay seam drift: ${file}`)
    fs.writeFileSync(name, text.replace(before, after))
    edits.push({
      file,
      beforeSha256: sha256(text),
      afterSha256: sha256(fs.readFileSync(name)),
      reason: after.includes('NEO_TESTS_PORT')
        ? 'nonzero port lease'
        : 'owner-approved headless guard waiver',
    })
  }
  const browserFixture = 'tests/config/browserTest.ts'
  const original = fs.readFileSync(path.join(root, browserFixture))
  // Keep upstream's complete fixture graph. Only the public import is replaced; the shim
  // extends this alias and overrides the browser ownership and auto configuration seams.
  const fixtureAlias = 'tests/config/__neoOriginalBrowserTest.ts'
  fs.writeFileSync(path.join(root, fixtureAlias), original, { flag: 'wx' })
  fs.writeFileSync(
    path.join(root, browserFixture),
    `export { test as playwrightTest, test as browserTest, test as contextTest, expect } from ${JSON.stringify(path.join(harness, 'js/fixtures.mjs'))};\n`,
  )
  edits.push({
    file: browserFixture,
    beforeSha256: sha256(original),
    afterSha256: sha256(fs.readFileSync(path.join(root, browserFixture))),
    reason: 'engine fixture seam; public calls stay native',
  })
  replace(
    'tests/config/serverFixtures.ts',
    'const port = 8907 + workerInfo.workerIndex * 4;',
    'const port = Number(process.env.NEO_TESTS_PORT);',
  )
  replace(
    'tests/library/headful.spec.ts',
    "it.skip(({ headless }) => headless, 'avoid popping windows in headless mode');",
    '// Owner policy: HE-012 runs headless. All other cases in this file are excluded by the reporter.',
  )
  fs.writeFileSync(
    path.join(root, 'neo.config.mjs'),
    `export default ${JSON.stringify(
      {
        testDir: path.join(root, 'tests'),
        testMatch: pin.suites
          .filter((suite) => suite.include)
          .map((suite) => suite.file.replace(/^tests\//, '')),
        workers: 1,
        retries: 0,
        repeatEach: 1,
        timeout: 30_000,
        // Keep this outer runner fence aligned with the native guard. Original slow and
        // setTimeout modifiers must retain their budgets inside the case supervisor.
        globalTimeout: 600_000,
        forbidOnly: true,
        fullyParallel: false,
        build: { external: [path.join(harness, 'js/**')] },
        outputDir: path.join(root, 'runner-output'),
        reporter: [[path.join(harness, 'js/reporter.mjs')]],
        use: { browserName: 'chromium', headless: true, channel: 'chromium' },
        projects: [{ name: 'chromium' }],
      },
      null,
      2,
    )};\n`,
  )
  const manifest = {
    revision: pin.revision,
    version: pin.version,
    sourceBuild: sha256(JSON.stringify(stamp)),
    fixtureAlias: { file: fixtureAlias, sha256: sha256(original) },
    edits,
    corpusSha256: sha256(fs.readFileSync(path.join(harness, 'corpus.json'))),
    // Freeze the actual transformation/fixture/reporter inputs, including frozen snapshots
    // passed by collect.mjs. Source provenance alone cannot identify the executed overlay.
    harnessFiles: Object.fromEntries(
      fs
        .readdirSync(path.join(harness, 'js'))
        .sort()
        .map((file) => [
          `js/${file}`,
          sha256(fs.readFileSync(path.join(harness, 'js', file))),
        ]),
    ),
    configSha256: sha256(fs.readFileSync(path.join(root, 'neo.config.mjs'))),
  }
  fs.writeFileSync(
    path.join(root, 'overlay.json'),
    JSON.stringify(manifest, null, 2),
  )
  return manifest
}

if (process.argv[1]?.endsWith('/overlay.mjs'))
  materialize(path.resolve(process.argv[2]))
