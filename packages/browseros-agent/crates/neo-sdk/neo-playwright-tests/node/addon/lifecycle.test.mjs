/** Neo: Real native errors, abandoned owners, worker unload and process exit on a private browser. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
import { launchBrowser } from '../browser.mjs'
import { openSdk } from '../loader.mjs'

/** Neo: Tests use the same native library path override as the production loader. */
const addonPath =
  process.env.NEO_NODE_ADDON ||
  fileURLToPath(new URL('./neo_node.node', import.meta.url))
/** Neo: Load N-API through Node's CommonJS loader. */
const addon = createRequire(import.meta.url)(addonPath)
/** Neo: The run's terminal error is Rust's existing target-closed error, including punctuation. */
const closed = {
  name: 'TargetClosedError',
  message: 'Target page, context or browser has been closed',
}

/** Neo: Assert real Error instances as well as their native fields. */
function named(expected) {
  return (error) => {
    assert.ok(error instanceof Error)
    assert.equal(error.name, expected.name)
    assert.equal(error.message, expected.message)
    return true
  }
}

test('native hosted lifecycle', { timeout: 120_000 }, async (t) => {
  assert.ok(process.env.NEO_E2E_BROWSER, 'Set NEO_E2E_BROWSER')
  assert.ok(process.env.NEO_NODE_OUTPUT_DIR, 'Set NEO_NODE_OUTPUT_DIR')
  await fs.mkdir(process.env.NEO_NODE_OUTPUT_DIR, { recursive: true })
  const output = await fs.mkdtemp(
    path.join(process.env.NEO_NODE_OUTPUT_DIR, 'lifecycle-'),
  )
  const owned = await launchBrowser(process.env.NEO_E2E_BROWSER, output)
  try {
    await t.test(
      'native synchronous errors and promise rejections keep exact names and messages',
      async () => {
        const runtime = await addon.connect(owned.port)
        const run = await runtime.openRun()
        try {
          assert.throws(
            () => run.get('{"guid":"missing","field":"url"}'),
            named({ name: 'Error', message: 'unknown guid "missing"' }),
          )
          assert.throws(
            () =>
              run.subscribe(
                '{"guid":"page@missing","event":"unsupported","enabled":true}',
              ),
            named({
              name: 'Error',
              message:
                'Neo does not produce "unsupported" events on "page@missing"',
            }),
          )
          assert.equal(run.drain(-1), '[]')
          const first = run.finish()
          assert.equal(await first, await run.finish())
          for (const call of [
            () => run.get('{}'),
            () => run.subscribe('{}'),
            () => run.drain(-1),
          ]) {
            assert.throws(call, named(closed))
          }
          await assert.rejects(run.callWithEvents('{}'), named(closed))
          await assert.rejects(run.nextEvents(), named(closed))
        } finally {
          await runtime.close()
        }
      },
    )

    await t.test(
      'the host deadline is attributed identically by every door',
      async () => {
        const runtime = await addon.connect(owned.port)
        try {
          const run = await runtime.openRun({ timeoutMs: 1000 })
          const timeout = {
            name: 'TimeoutError',
            message: 'Timeout 1000ms exceeded.',
          }
          await assert.rejects(run.nextEvents(), named(timeout))
          for (const call of [
            () => run.get('{}'),
            () => run.subscribe('{}'),
            () => run.drain(-1),
          ]) {
            assert.throws(call, named(timeout))
          }
          await assert.rejects(run.callWithEvents('{}'), named(timeout))
          assert.equal(JSON.parse(await run.finish()).status, 'timeout')
        } finally {
          await runtime.close()
        }
      },
    )

    await t.test(
      'runtime close cancels open runs, calls and event waits and can repeat',
      async () => {
        const runtime = await addon.connect(owned.port)
        const run = await runtime.openRun()
        const other = await runtime.openRun()
        const init = JSON.parse(run.init)
        const message = {
          id: 1,
          guid: init.mainFrame,
          method: 'waitForSelector',
          params: {
            selector: '#never-present',
            state: 'attached',
            timeout: 30000,
          },
          metadata: {},
        }
        const events = run.nextEvents()
        const call = run.callWithEvents(JSON.stringify(message))
        const settled = Promise.allSettled([events, call])
        await runtime.close()
        const results = await settled
        assert.ok(
          results.every((result) => result.status === 'rejected'),
          JSON.stringify(results),
        )
        for (const result of results)
          named({ name: 'Error', message: 'Run cancelled' })(result.reason)
        assert.equal(JSON.parse(await run.finish()).status, 'cancelled')
        assert.equal(JSON.parse(await other.finish()).status, 'cancelled')
        await assert.rejects(runtime.openRun(), named(closed))
        await runtime.close()
      },
    )

    await t.test(
      'dropping a run settles its pending promise and Node exits',
      () => {
        const code = `
        const addon = require(${JSON.stringify(addonPath)});
        (async () => {
          const runtime = await addon.connect(${owned.port});
          let run = await runtime.openRun();
          let result;
          const events = run.nextEvents().catch(error => { result = error.message });
          run = null;
          for (let i = 0; i < 40 && !result; i++) { global.gc(); await new Promise(r => setTimeout(r, 25)); }
          if (result !== 'Run cancelled') throw new Error('Run owner retained: ' + result);
          await events;
          await runtime.close();
        })().catch(error => { console.error(error); process.exitCode = 1 });`
        const child = spawnSync(process.execPath, ['--expose-gc', '-e', code], {
          encoding: 'utf8',
          timeout: 10_000,
        })
        assert.ifError(child.error)
        assert.equal(child.status, 0, child.stderr)
      },
    )

    await t.test(
      'worker environment unload with a pending event promise does not panic',
      async () => {
        const before = new Set(
          (
            await (
              await fetch(`http://127.0.0.1:${owned.port}/json/list`)
            ).json()
          ).map((target) => target.id),
        )
        const worker = new Worker(
          `
        const { parentPort } = require('node:worker_threads');
        const addon = require(${JSON.stringify(addonPath)});
        (async () => {
          const runtime = await addon.connect(${owned.port});
          const run = await runtime.openRun();
          run.nextEvents().catch(() => {});
          global.runtime = runtime; global.run = run;
          parentPort.postMessage('ready');
        })().catch(error => { throw error });
      `,
          { eval: true },
        )
        try {
          await new Promise((resolve, reject) => {
            worker.once('message', resolve)
            worker.once('error', reject)
          })
        } finally {
          await worker.terminate()
        }
        let remaining
        for (let attempt = 0; attempt < 40; attempt++) {
          remaining = (
            await (
              await fetch(`http://127.0.0.1:${owned.port}/json/list`)
            ).json()
          ).filter((target) => !before.has(target.id))
          if (!remaining.length) break
          await sleep(50)
        }
        assert.deepEqual(remaining, [], 'Worker unload left a tab behind')
      },
    )

    await t.test(
      'handles are capped and disposing one frees a slot',
      async () => {
        const sdk = await openSdk(owned.port)
        try {
          let first
          for (let i = 0; i < 10_000; i++) {
            const handle = await sdk.page.evaluateHandle(() => ({ value: 1 }))
            if (i === 0) first = handle
          }
          await assert.rejects(
            sdk.page.evaluateHandle(() => ({})),
            /Neo hosted runs keep at most 10000 JS handles/,
          )
          assert.deepEqual(await first.jsonValue(), { value: 1 })
          await first.dispose()
          const replacement = await sdk.page.evaluateHandle(() => 42)
          assert.equal(await replacement.jsonValue(), 42)
          await replacement.dispose()
        } finally {
          await sdk.finish()
        }
      },
    )

    await t.test(
      'four independent runtimes can share the native executor',
      async () => {
        await Promise.all(
          Array.from({ length: 4 }, async () => {
            const sdk = await openSdk(owned.port)
            try {
              for (let i = 0; i < 25; i++)
                assert.equal(await sdk.page.evaluate(() => 42), 42)
            } finally {
              await sdk.finish()
            }
          }),
        )
      },
    )

    await t.test(
      'an open deadline rejects and close drains the failed startup',
      async () => {
        const runtime = await addon.connect(owned.port)
        try {
          await assert.rejects(
            runtime.openRun({ timeoutMs: 0 }),
            named({ name: 'TimeoutError', message: 'Timeout 0ms exceeded.' }),
          )
        } finally {
          await runtime.close()
        }
      },
    )

    await t.test('loader finish is shared by concurrent callers', async () => {
      const sdk = await openSdk(owned.port)
      assert.equal(sdk.page.url(), 'about:blank')
      const first = sdk.finish()
      assert.equal(sdk.finish(), first)
      assert.equal(JSON.parse(await first).status, 'ok')
    })
  } finally {
    await owned.close()
  }
})
