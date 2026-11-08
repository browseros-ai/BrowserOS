// Neo: one run leaves another run’s same-named binding on a different page.
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

const probeSource = (tag) =>
  `(async()=>({type:typeof sharedBinding,result:await Promise.race([Promise.resolve().then(()=>sharedBinding(${JSON.stringify(tag)})).then(value=>({value}),error=>({error:error.name})),new Promise(resolve=>setTimeout(()=>resolve({unsettled:true}),1200))])}))()`
test('ending run A preserves run B same-named binding on another page', {
  skip,
  timeout: 180000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'binding-page-owner-review-'))
  const cleanups = [() => rmSync(root, { recursive: true, force: true })]
  let child,
    exited,
    stoppedA = false,
    stoppedB = false
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/first': '<title>first</title>',
        '/second': '<title>second</title>',
      },
    })
    const refBrowser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(
      `http://127.0.0.1:${refBrowser.port}`,
    )
    cleanups.push(() => pw.close())
    const context = pw.contexts()[0],
      p = await context.newPage(),
      q = await context.newPage()
    await p.goto(`${origin}/first`)
    await q.goto(`${origin}/second`)
    const aBinding = await p.exposeFunction('sharedBinding', (v) => `A:${v}`)
    await q.exposeFunction('sharedBinding', (v) => `B:${v}`)
    const beforeReference = await q.evaluate(probeSource('before'))
    await aBinding.dispose()
    const afterReference = await q.evaluate(probeSource('after'))
    await p.close()
    await q.close()
    await pw.close()
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const first = await openTab(cdp, `${origin}/first`),
      second = await openTab(cdp, `${origin}/second`)
    const probe = async (sessionId, expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true, awaitPromise: true },
          sessionId,
        )
      ).result.value
    const a = join(root, 'a.js'),
      b = join(root, 'b.js')
    writeFileSync(
      a,
      "await page.exposeFunction('sharedBinding',v=>'A:'+v); const second=context.pages().find(p=>p.url().endsWith('/second')); await second.title(); await page.evaluate(()=>window.aReady=true); await new Promise(()=>{});",
    )
    writeFileSync(
      b,
      "const second=context.pages().find(p=>p.url().endsWith('/second')); await second.exposeFunction('sharedBinding',v=>'B:'+v); await second.evaluate(()=>window.bReady=true); await new Promise(()=>{});",
    )
    child = spawn(bin, [String(browser.port), first.targetId, a, b])
    let stdout = '',
      stderr = ''
    child.stdout.on('data', (d) => (stdout += d))
    child.stderr.on('data', (d) => (stderr += d))
    exited = once(child, 'exit')
    await eventually(
      () => probe(first.sessionId, 'window.aReady===true'),
      'run A ready and attached to the other page',
    )
    child.stdin.write('startB\n')
    await eventually(
      () => probe(second.sessionId, 'window.bReady===true'),
      'run B ready on the other page',
    )
    const before = await probe(second.sessionId, probeSource('before'))
    child.stdin.write('stopA\n')
    stoppedA = true
    await eventually(() => stdout.includes('outcome 0'), 'run A ended')
    const after = await probe(second.sessionId, probeSource('after'))
    child.stdin.write('stopB\n')
    stoppedB = true
    const [code] = await exited
    t.diagnostic(
      JSON.stringify({
        beforeReference,
        afterReference,
        before,
        after,
        stdout,
        stderr,
      }),
    )
    assert.equal(code, 0, stderr)
    assert.deepEqual(beforeReference, {
      type: 'function',
      result: { value: 'B:before' },
    })
    assert.deepEqual(afterReference, {
      type: 'function',
      result: { value: 'B:after' },
    })
    assert.deepEqual(before, beforeReference)
    assert.deepEqual(after, afterReference)
  } finally {
    if (child && child.exitCode === null) {
      child.stdin.end((stoppedA ? '' : 'stopA\n') + (stoppedB ? '' : 'stopB\n'))
      await exited
    }
    await runCleanups(cleanups)
  }
})
