// Two runs on one tab, each exposing a function (med-pw-int's batch-3 delta review F2): the
// page's binding controller and CDP binding are shared, and Chromium reports each call to both
// runs' sessions. Each run answers its own name and leaves the other's alone, and the end of one
// run takes off only its own name: the other run's function keeps working. Playwright's control
// is one page exposing both names and disposing the first.
//
// Neo runs through `binding-concurrent-host.rs` (two `Runtime::run`s of one Runtime); the driver
// probes the tab over raw CDP between their steps.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  chromium,
  connectCdp,
  eventually,
  launchBrowser,
  openTab,
  runCleanups,
  serveFixture,
  skip,
} from './harness.mjs'

/** Builds `binding-concurrent-host.rs` against the crates `neo_run` is built from. */
function buildHost(root) {
  const built = spawnSync(
    'cargo',
    [
      'build',
      '-p',
      'neo-runtime',
      '--example',
      'neo_run',
      '--locked',
      '--message-format=json',
    ],
    { encoding: 'utf8' },
  )
  assert.equal(built.status, 0, built.stderr)
  const libs = new Map()
  for (const line of built.stdout.split('\n')) {
    if (!line.startsWith('{')) continue
    const row = JSON.parse(line)
    if (row.reason !== 'compiler-artifact') continue
    for (const path of row.filenames ?? [])
      if (path.endsWith('.rlib')) libs.set(row.target.name, path)
  }
  const args = [
    '--edition=2024',
    '-L',
    `dependency=${dirname(libs.get('neo_runtime'))}`,
  ]
  for (const name of ['browseros_cdp', 'neo_runtime', 'tokio', 'tokio_util']) {
    assert.ok(libs.has(name), `missing ${name} library`)
    args.push('--extern', `${name}=${libs.get(name)}`)
  }
  const bin = join(root, 'binding-concurrent-host')
  args.push(
    fileURLToPath(new URL('./binding-concurrent-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

/** What the page sees: both functions' types and a call of `runB`, settled or not in 1.2 s. */
const PROBE = (tag) => `(async () => ({
  a: typeof runA,
  b: typeof runB,
  result: await Promise.race([
    Promise.resolve().then(() => runB(${JSON.stringify(tag)})).then(
      (value) => ({ value }),
      (error) => ({ error: error.name }),
    ),
    new Promise((resolve) => setTimeout(() => resolve({ unsettled: true }), 1200)),
  ]),
}))()`

test('two runs on one tab answer their own bindings, and one run’s end leaves the other’s', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'neo-binding-concurrent-'))
  const cleanups = [() => rmSync(root, { recursive: true, force: true })]
  let child
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><title>bindings</title><p>top</p>',
      },
    })

    const reference = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${reference.port}`,
    )
    cleanups.push(() => playwright.close())
    const page = await playwright.contexts()[0].newPage()
    await page.goto(`${origin}/top.html`)
    const first = await page.exposeFunction('runA', (v) => `A:${v}`)
    await page.exposeFunction('runB', (v) => `B:${v}`)
    const beforeReference = await page.evaluate(PROBE('before'))
    await first.dispose()
    const afterReference = await page.evaluate(PROBE('check'))
    await page.close()

    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const tab = await openTab(cdp, `${origin}/top.html`)
    const probe = async (expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true, awaitPromise: true },
          tab.sessionId,
        )
      ).result.value
    const a = join(root, 'a.js')
    const b = join(root, 'b.js')
    writeFileSync(
      a,
      "await page.exposeFunction('runA', (v) => 'A:' + v); await page.evaluate(() => { window.aReady = true }); await new Promise(() => {})",
    )
    writeFileSync(
      b,
      "await page.exposeFunction('runB', (v) => 'B:' + v); await page.evaluate(() => { window.bReady = true }); await new Promise(() => {})",
    )
    child = spawn(bin, [String(port), tab.targetId, a, b])
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    const exited = once(child, 'exit')
    await eventually(
      () => probe('window.aReady === true'),
      'run A exposed runA',
    )
    child.stdin.write('startB\n')
    await eventually(
      () => probe('window.bReady === true'),
      'run B exposed runB',
    )
    const before = await probe(PROBE('before'))
    child.stdin.write('stopA\n')
    await eventually(() => stdout.includes('outcome 0'), 'run A ended')
    const after = await probe(PROBE('check'))
    child.stdin.write('stopB\n')
    const [code] = await exited
    t.diagnostic(
      JSON.stringify({
        beforeReference,
        afterReference,
        before,
        after,
        stdout,
      }),
    )
    assert.equal(code, 0, stderr)
    assert.deepEqual(beforeReference, {
      a: 'function',
      b: 'function',
      result: { value: 'B:before' },
    })
    assert.deepEqual(
      before,
      beforeReference,
      'run A leaves run B’s call to run B',
    )
    assert.deepEqual(
      after,
      afterReference,
      'run A’s end leaves run B’s binding',
    )
  } finally {
    if (child && child.exitCode === null) {
      child.stdin.end('stopA\nstopB\n')
      await once(child, 'exit')
    }
    await runCleanups(cleanups)
  }
})
