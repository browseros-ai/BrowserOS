/** Neo: Copies verified upstream tests; only the browser fixture seam and server port lease change. */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { corpus, sha256, source, verifySource } from '../js/prepare.mjs'

export function materialize(root, { reference, output }) {
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
  for (const name of ['packages', 'node_modules'])
    fs.symlinkSync(path.join(source, name), path.join(root, name), 'dir')
  const edits = []
  if (!reference) {
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
    edits.push({ file, reason: 'Neo SDK fixture seam' })
  }
  const serverFile = path.join(root, 'tests/config/serverFixtures.ts')
  const before = fs.readFileSync(serverFile, 'utf8')
  const seam = 'const port = 8907 + workerInfo.workerIndex * 4;'
  if (before.split(seam).length !== 2)
    throw new Error('Server port seam changed')
  fs.writeFileSync(
    serverFile,
    before.replace(seam, 'const port = Number(process.env.NEO_TESTS_PORT);'),
  )
  edits.push({
    file: 'tests/config/serverFixtures.ts',
    reason: 'nonzero port lease',
  })
  const suite = corpus.suites.find(
    (suite) => suite.file === 'tests/page/page-goto.spec.ts',
  )
  const digest = sha256(fs.readFileSync(path.join(root, suite.file)))
  if (digest !== suite.sha256) throw new Error('page-goto differs from corpus')
  fs.writeFileSync(
    path.join(output, 'overlay.json'),
    JSON.stringify(
      {
        revision: corpus.revision,
        spec: suite.file,
        sha256: digest,
        edits,
        omittedUpstreamDefaults: reference
          ? {}
          : { offline: false, acceptDownloads: true },
      },
      null,
      2,
    ),
  )
  const config = {
    testDir: path.join(root, 'tests'),
    testMatch: 'page/page-goto.spec.ts',
    workers: 1,
    retries: 0,
    repeatEach: 1,
    timeout: 30_000,
    globalTimeout: 1_800_000,
    forbidOnly: true,
    fullyParallel: false,
    outputDir: path.join(output, 'test-results'),
    build: {
      external: [
        fileURLToPath(new URL('../node/**', import.meta.url)),
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
      trace: 'off',
      screenshot: 'off',
      video: 'off',
      ...(reference
        ? {
            launchOptions: {
              executablePath: process.env.NEO_E2E_BROWSER,
              args: [
                '--disable-browseros-server',
                '--disable-browseros-extensions',
                '--lang=en-US',
                '--window-size=1280,720',
                '--force-device-scale-factor=1',
              ],
            },
          }
        : {}),
    },
    projects: [{ name: reference ? 'browseros-playwright' : 'browseros-neo' }],
  }
  const configPath = path.join(root, 'neo.config.mjs')
  fs.writeFileSync(
    configPath,
    `export default ${JSON.stringify(config, null, 2)};\n`,
  )
  return configPath
}
