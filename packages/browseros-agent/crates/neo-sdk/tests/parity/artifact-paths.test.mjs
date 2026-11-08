// The run's artifact directory for every `path` option that writes or reads a script's file:
// context.storageState({ path }) writes the state there, context.setStorageState(path) reads one
// from there, and download.saveAs(path) copies the download there, as page.screenshot({ path })
// writes its image (neo-runtime `artifacts.rs`). Playwright writes and reads those paths on its
// host; Neo only inside the directory the host gives the run (neo_run --artifacts), at relative
// paths without `..`, and lists each file it wrote in the outcome's `artifacts`.
//
// The first case runs one script through the pinned Playwright, its paths absolute inside a
// directory of the test's, and through Neo with the same paths relative to the run's artifact
// directory; it compares what each script returned and the files each left. The second, Neo's
// alone, checks the confinement: a path that leaves the directory rejects and writes nothing.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const DOWNLOAD = Buffer.from('00ff800d0a4100c3a9', 'hex')

const pages = {
  '/artifacts.html': '<!doctype html><title>artifacts</title><p>artifacts</p>',
  '/download.bin': {
    headers: {
      'content-type': 'application/octet-stream',
      'content-disposition': 'attachment; filename="review.bin"',
    },
    body: DOWNLOAD,
  },
}

const PRELUDE = `const settle = (promise) => promise.then(() => 'ok', (error) => error.name)
const download = () => Promise.all([
  page.waitForEvent('download'),
  page.evaluate(() => {
    const link = document.createElement('a')
    link.href = '/download.bin'
    document.body.append(link)
    link.click()
  }),
]).then(([download]) => download)
`

/** The script, with `paths` as the engine names them: absolute for Playwright, relative for Neo. */
const roundTrip = (paths) => `${PRELUDE}const P = ${JSON.stringify(paths)}
// An origin counts as visited once a page loaded it after the run attached (storageState.test.mjs).
await page.goto(page.url())
await context.addCookies([{ name: 'artifactCookie', value: 'value', url: page.url() }])
await page.evaluate(() => localStorage.setItem('artifactItem', 'kept'))
const shot = await settle(page.screenshot({ path: P.shot }))
const state = await settle(context.storageState({ path: P.state }))
await context.clearCookies({ name: 'artifactCookie' })
await page.evaluate(() => localStorage.removeItem('artifactItem'))
const restored = await settle(context.setStorageState(P.state))
const after = {
  cookie: (await context.cookies(page.url())).filter((cookie) => cookie.name === 'artifactCookie').map((cookie) => cookie.value),
  item: await page.evaluate(() => localStorage.getItem('artifactItem')),
}
const file = await download()
const saved = await settle(file.saveAs(P.download))
const filename = file.suggestedFilename()
await file.delete()
await context.clearCookies({ name: 'artifactCookie' })
await page.evaluate(() => localStorage.removeItem('artifactItem'))
return { shot, state, restored, after, saved, filename }`

const PATHS = {
  shot: 'proof.png',
  state: 'state.json',
  download: 'saved/review.bin',
}

/** What `dir` holds of the case's files: which exist, the download's bytes, the state's own entries. */
function files(dir) {
  const at = (name) => join(dir, name)
  const state = existsSync(at(PATHS.state))
    ? JSON.parse(readFileSync(at(PATHS.state), 'utf8'))
    : null
  return {
    shot: existsSync(at(PATHS.shot)),
    download: existsSync(at(PATHS.download))
      ? readFileSync(at(PATHS.download)).toString('hex')
      : null,
    stateCookies: state?.cookies
      .filter((cookie) => cookie.name === 'artifactCookie')
      .map(({ name, value, path, httpOnly, secure, sameSite }) => ({
        name,
        value,
        path,
        httpOnly,
        secure,
        sameSite,
      })),
    stateItems: state?.origins.flatMap((origin) =>
      origin.localStorage.filter((item) => item.name === 'artifactItem'),
    ),
  }
}

test('storageState, setStorageState and download.saveAs write and read in the run artifact directory', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const root = mkdtempSync(join(tmpdir(), 'neo-artifact-paths-'))
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const url = `${origin}/artifacts.html`

    await t.test(
      'the files land where Playwright puts them on its host, and the outcome lists them',
      async () => {
        const playwrightDir = join(root, 'playwright')
        mkdirSync(playwrightDir)
        const absolute = Object.fromEntries(
          Object.entries(PATHS).map(([key, name]) => [
            key,
            join(playwrightDir, name),
          ]),
        )
        const connection = await chromium.connectOverCDP(
          `http://127.0.0.1:${browser.port}`,
        )
        let reference
        try {
          const [context] = connection.contexts()
          const page = await context.newPage()
          await page.goto(url)
          reference = await new (async () => {}).constructor(
            'page',
            'context',
            roundTrip(absolute),
          )(page, context)
          await page.close()
        } finally {
          await connection.close()
        }
        assert.deepEqual(reference, {
          shot: 'ok',
          state: 'ok',
          restored: 'ok',
          after: { cookie: ['value'], item: 'kept' },
          saved: 'ok',
          filename: 'review.bin',
        })

        const neoDir = join(root, 'neo')
        mkdirSync(neoDir)
        const script = join(root, 'round-trip.js')
        writeFileSync(script, roundTrip(PATHS))
        const { targetId } = await openTab(cdp, url)
        try {
          const run = startNeo(neoRun, browser.port, targetId, script, [
            '--artifacts',
            neoDir,
          ])
          await run.connected
          const outcome = await run.outcome
          assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
          assert.deepEqual(outcome.value, reference, 'Neo vs Playwright')
          assert.deepEqual(
            files(neoDir),
            files(playwrightDir),
            'the files each engine left',
          )
          assert.deepEqual(
            outcome.artifacts,
            Object.values(PATHS).map((path) => ({
              path,
              bytes: statSync(join(neoDir, path)).size,
            })),
            'the outcome lists each file the run wrote, once',
          )
        } finally {
          await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
        }
      },
    )

    await t.test(
      'Neo: a path that leaves the artifact directory rejects with Error and writes nothing',
      async () => {
        const neoDir = join(root, 'confined')
        mkdirSync(neoDir)
        const outside = join(root, 'outside.json')
        const script = join(root, 'confined.js')
        writeFileSync(
          script,
          `${PRELUDE}const file = await download()
const out = {
  up: await settle(context.storageState({ path: '../escape.json' })),
  absolute: await settle(context.storageState({ path: ${JSON.stringify(outside)} })),
  read: await settle(context.setStorageState('../escape.json')),
  saved: await settle(file.saveAs('../escape.bin')),
}
await file.delete()
return out`,
        )
        const { targetId } = await openTab(cdp, url)
        try {
          const run = startNeo(neoRun, browser.port, targetId, script, [
            '--artifacts',
            neoDir,
          ])
          await run.connected
          const outcome = await run.outcome
          assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
          assert.deepEqual(outcome.value, {
            up: 'Error',
            absolute: 'Error',
            read: 'Error',
            saved: 'Error',
          })
          assert.deepEqual(
            [
              existsSync(join(root, 'escape.json')),
              existsSync(outside),
              existsSync(join(root, 'escape.bin')),
            ],
            [false, false, false],
            'nothing written outside the directory',
          )
          assert.equal(outcome.artifacts, undefined, 'and nothing listed')
        } finally {
          await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
        }
      },
    )
  } finally {
    await runCleanups(cleanups)
  }
})
