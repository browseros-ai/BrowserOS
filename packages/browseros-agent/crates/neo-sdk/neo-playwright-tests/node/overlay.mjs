/** Neo: Verifies the whole corpus and changes only documented fixture and headless seams. */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { corpus, sha256, source, verifySource } from '../js/prepare.mjs'

export function materialize(
  root,
  { reference, output, workers = 4, expectLoop = true },
) {
  const stamp = verifySource()
  fs.mkdirSync(root, { recursive: true })
  fs.writeFileSync(
    path.join(root, 'package.json'),
    JSON.stringify({ private: true, type: 'commonjs' }),
  )
  fs.cpSync(path.join(source, 'tests'), path.join(root, 'tests'), {
    recursive: true,
  })
  for (const [file, digest] of Object.entries(stamp.inputs)) {
    if (
      file.startsWith('tests/') &&
      sha256(fs.readFileSync(path.join(root, file))) !== digest
    )
      throw new Error(`Upstream file changed during copy: ${file}`)
  }
  const specs = corpus.suites
    .filter((suite) => suite.include)
    .map((suite) => {
      if (sha256(fs.readFileSync(path.join(root, suite.file))) !== suite.sha256)
        throw new Error(`Spec differs from corpus: ${suite.file}`)
      return { file: suite.file, sha256: suite.sha256, title: suite.title }
    })
  for (const name of ['packages', 'node_modules'])
    fs.symlinkSync(path.join(source, name), path.join(root, name), 'dir')
  const edits = []
  const replace = (file, before, after, reason) => {
    const target = path.join(root, file)
    const text = fs.readFileSync(target, 'utf8')
    if (text.split(before).length !== 2)
      throw new Error(`Overlay seam changed: ${file}`)
    fs.writeFileSync(target, text.replace(before, after))
    edits.push({
      file,
      reason,
      beforeSha256: sha256(text),
      afterSha256: sha256(fs.readFileSync(target)),
    })
  }
  const file = 'tests/config/browserTest.ts'
  const original = fs.readFileSync(path.join(root, file))
  fs.writeFileSync(
    path.join(root, 'tests/config/__neoOriginalBrowserTest.ts'),
    original,
  )
  const fixture = fileURLToPath(new URL('./fixtures.mjs', import.meta.url))
  fs.writeFileSync(
    path.join(root, file),
    `export { test as playwrightTest, test as browserTest, test as contextTest, expect } from ${JSON.stringify(fixture)};\n`,
  )
  edits.push({
    file,
    reason: reference
      ? 'owned reference browser fixture'
      : 'Neo SDK fixture seam',
    beforeSha256: sha256(original),
    afterSha256: sha256(fs.readFileSync(path.join(root, file))),
  })
  if (!reference && expectLoop) {
    const pageFile = 'tests/page/pageTest.ts'
    const pageOriginal = fs.readFileSync(path.join(root, pageFile))
    fs.writeFileSync(
      path.join(root, 'tests/page/__neoOriginalPageTest.ts'),
      pageOriginal,
    )
    fs.writeFileSync(
      path.join(root, pageFile),
      [
        "export * from './__neoOriginalPageTest';",
        "import { expect as originalExpect } from './__neoOriginalPageTest';",
        `import { wrapExpect } from ${JSON.stringify(fixture)};`,
        'export const expect = wrapExpect(originalExpect);',
        '',
      ].join('\n'),
    )
    edits.push({
      file: pageFile,
      reason:
        'test-only expectLoop import seam; preserve extended expect and all other exports',
      beforeSha256: sha256(pageOriginal),
      afterSha256: sha256(fs.readFileSync(path.join(root, pageFile))),
    })
  }
  replace(
    'tests/config/serverFixtures.ts',
    'async ({ loopback }, run, workerInfo)',
    'async ({ loopback, __neoWorker }, run, workerInfo)',
    'worker-owned server lease dependency',
  )
  replace(
    'tests/config/serverFixtures.ts',
    'const port = 8907 + workerInfo.workerIndex * 4;',
    'const port = __neoWorker.serverPort;',
    'per-worker nonzero port lease',
  )
  replace(
    'tests/library/headful.spec.ts',
    "it.skip(({ headless }) => headless, 'avoid popping windows in headless mode');",
    '// Owner policy: HE-012 runs headless. All other cases in this file are excluded by the reporter.',
    'owner-approved headless guard waiver; selection enforced by launcher',
  )
  const manifest = {
    revision: corpus.revision,
    specs,
    edits,
    contextOptionOmissions: reference
      ? []
      : [
          {
            option: 'offline',
            value: false,
            when: 'implicit upstream default only',
          },
          {
            option: 'acceptDownloads',
            value: true,
            when: 'implicit upstream default only',
          },
        ],
  }
  fs.writeFileSync(
    path.join(output, 'overlay.json'),
    JSON.stringify(manifest, null, 2),
  )
  const config = {
    testDir: path.join(root, 'tests'),
    testMatch: specs.map((suite) => suite.file.replace(/^tests\//, '')),
    workers,
    retries: 0,
    repeatEach: 1,
    timeout:
      process.platform === 'darwin' && process.arch === 'x64' ? 60_000 : 30_000,
    expect: { timeout: 10_000 },
    forbidOnly: true,
    fullyParallel: false,
    updateSnapshots: 'none',
    snapshotPathTemplate:
      '{testDir}/{testFileDir}/{testFileName}-snapshots/{arg}-chromium{ext}',
    // Relative SDK file paths and testInfo.outputPath must share the copied upstream cwd.
    outputDir: path.join(root, 'test-results'),
    build: {
      external: [
        fileURLToPath(new URL('./**', import.meta.url)),
        fileURLToPath(new URL('../js/**', import.meta.url)),
      ],
    },
    reporter: [
      ['line'],
      ['json', { outputFile: path.join(output, 'results.json') }],
    ],
    use: {
      browserName: 'chromium',
      headless: true,
      channel: 'chromium',
      mode: 'default',
      trace: 'off',
      screenshot: 'off',
      video: 'off',
    },
    projects: [{ name: 'browseros-neo' }],
  }
  const configPath = path.join(root, 'neo.config.mjs')
  fs.writeFileSync(
    configPath,
    `export default ${JSON.stringify(config, null, 2)};\n`,
  )
  return { configPath, ...manifest }
}
