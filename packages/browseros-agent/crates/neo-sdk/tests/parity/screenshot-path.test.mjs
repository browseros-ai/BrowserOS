// page.screenshot({ path }) as the MCP host runs it: neo_run --redirect-absolute-paths
// (neo-runtime `AbsolutePaths::Redirect`). REAL fly-unified-3, opendining-2 and dashdish-3 passed
// absolute paths on the agent's machine and got "this run has no artifact directory".
//
// One script runs through the pinned Playwright and through Neo. Playwright writes every path
// where it is named. Neo writes a relative path and an absolute path inside the run's artifact
// directory there too; an absolute path outside it lands at `<artifacts>/<file name>`, the
// documented deviation, so that case compares the file's content, not its location. Compared:
// what the script saw of its bytes (image type, size; never checksums across engines), each
// file against the bytes its own engine returned, the error kind of a path naming a directory,
// and the returned Buffer's shape (Uint8Array, PNG signature, base64).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { DECODERS, fileChecksum } from './capture.mjs'
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

const pages = {
  '/tall.html': `<!doctype html><title>tall</title><style>body{margin:0}
    #tall{width:700px;height:2400px;background:linear-gradient(#f80,#08f)}</style>
    <button>first</button><div id=tall></div>`,
}

/** The REAL snippet's shape, with `P` naming each path as the engine takes it. */
const script = (paths) => `${DECODERS}
const P = ${JSON.stringify(paths)}
const settle = (promise) => promise.then(() => 'ok', (error) => error.name)
const relative = describeImage(await page.screenshot({ path: P.relative }))
const outside = describeImage(await page.screenshot({ path: P.outside, fullPage: true }))
const inside = describeImage(await page.screenshot({ path: P.inside }))
const element = describeImage(await page.getByRole('button').first().screenshot({ path: P.element }))
const directory = await settle(page.screenshot({ path: P.directory }))
const returned = await page.screenshot()
return {
  relative, outside, inside, element, directory,
  returned: {
    uint8: returned instanceof Uint8Array,
    signature: [...returned.subarray(0, 8)],
    base64: returned.toString('base64').slice(0, 12),
    size: describeImage(returned).width > 0 && returned.length > 1000,
  },
  message: 'Screenshot saved to columns.png',
}`

/** `value` without the per-engine checksums, which differ across engines by design. */
function shape(value) {
  return JSON.parse(
    JSON.stringify(value, (key, field) =>
      key === 'checksum' ? undefined : field,
    ),
  )
}

test('screenshot({ path }) writes relative and absolute paths; a returned Buffer keeps its shape', {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const root = mkdtempSync(join(tmpdir(), 'neo-screenshot-path-'))
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
    const origin = await serveFixture(cleanups, { pages })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const url = `${origin}/tall.html`

    const playwrightDir = join(root, 'playwright')
    const playwrightOutside = join(root, 'playwright-results')
    mkdirSync(join(playwrightDir, 'existing'), { recursive: true })
    const playwrightPaths = {
      relative: join(playwrightDir, 'shots/a.png'),
      outside: join(playwrightOutside, 'columns.png'),
      inside: join(playwrightDir, 'inside/b.png'),
      element: join(playwrightDir, 'button.png'),
      directory: join(playwrightDir, 'existing'),
    }
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
        script(playwrightPaths),
      )(page, context)
      await page.close()
    } finally {
      await connection.close()
    }
    assert.equal(
      reference.directory,
      'Error',
      'a directory is no file to write',
    )
    assert.equal(reference.outside.type, 'png')
    assert.ok(
      reference.outside.height >= 2400,
      'fullPage captures the whole page',
    )
    for (const key of ['relative', 'outside', 'inside', 'element'])
      assert.equal(
        fileChecksum(playwrightPaths[key]),
        reference[key].checksum,
        `Playwright wrote ${key} where named`,
      )

    const neoDir = join(root, 'neo')
    const neoOutside = join(root, 'neo-results')
    mkdirSync(join(neoDir, 'existing'), { recursive: true })
    const neoPaths = {
      relative: 'shots/a.png',
      outside: join(neoOutside, 'columns.png'),
      inside: join(neoDir, 'inside/b.png'),
      element: 'button.png',
      directory: 'existing',
    }
    const scriptPath = join(root, 'neo.js')
    writeFileSync(scriptPath, script(neoPaths))
    const { targetId } = await openTab(cdp, url)
    try {
      const run = startNeo(neoRun, browser.port, targetId, scriptPath, [
        '--artifacts',
        neoDir,
        '--redirect-absolute-paths',
      ])
      await run.connected
      const outcome = await run.outcome
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
      assert.deepEqual(
        shape(outcome.value),
        shape(reference),
        'Neo vs Playwright',
      )
      const value = outcome.value
      // Inside the directory: exactly where named. Outside: under its file name, never there.
      assert.equal(
        fileChecksum(join(neoDir, 'shots/a.png')),
        value.relative.checksum,
      )
      assert.equal(
        fileChecksum(join(neoDir, 'inside/b.png')),
        value.inside.checksum,
      )
      assert.equal(
        fileChecksum(join(neoDir, 'button.png')),
        value.element.checksum,
      )
      assert.equal(
        fileChecksum(join(neoDir, 'columns.png')),
        value.outside.checksum,
      )
      assert.equal(
        existsSync(neoOutside),
        false,
        'nothing written outside the directory',
      )
      assert.deepEqual(
        outcome.artifacts.map(({ path, requested }) => ({ path, requested })),
        [
          { path: 'shots/a.png', requested: undefined },
          { path: 'columns.png', requested: neoPaths.outside },
          { path: 'inside/b.png', requested: undefined },
          { path: 'button.png', requested: undefined },
        ],
        'the outcome lists each file, with the path asked for when redirected',
      )
      const note = outcome.logs.filter((line) =>
        line.includes(neoPaths.outside),
      )
      assert.equal(note.length, 1, JSON.stringify(outcome.logs))
      assert.ok(note[0].includes(join(neoDir, 'columns.png')), note[0])
    } finally {
      await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
    }
  } finally {
    await runCleanups(cleanups)
  }
})
