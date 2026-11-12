// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Loads v1's exact production sources in Node's main realm, with a private bootstrap holder. */
import { Buffer } from 'node:buffer'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

const files = [
  'neo-server/src/runtime/js/bootstrap.js',
  'neo-playwright/assets/prelude.js',
  'neo-server/src/runtime/js/neo_prelude.js',
]

export async function openClaw(cdpPort, options = {}, { onUncaught } = {}) {
  const require = createRequire(import.meta.url)
  const addon = require(
    process.env.CLAW_NODE_ADDON || './claw-addon/claw_node.node',
  )
  const stateDir =
    options.stateDir ||
    fs.mkdtempSync(path.join(os.tmpdir(), 'claw-node-state-'))
  let runtime
  let native
  let finishing = false
  let finished
  let releaseMain
  let running
  const parked = new Promise(() => {})
  const uncaught =
    onUncaught ||
    ((error) =>
      setImmediate(() => {
        throw error
      }))
  // v1's emitter reports listener throws with Promise.reject. One run owns each test worker;
  // forward its uncaught rejections while it lives, without changing Promise or SDK objects.
  const onRejection = (error) => uncaught(error)
  process.on('unhandledRejection', onRejection)
  async function close() {
    try {
      await runtime?.close()
    } finally {
      process.removeListener('unhandledRejection', onRejection)
      if (!options.stateDir)
        fs.rmSync(stateDir, { recursive: true, force: true })
    }
  }
  try {
    runtime = await addon.connect(cdpPort, {
      stateDir,
      executionDir: options.executionDir,
      browserosVersion: options.browserosVersion,
      // The browser's launch mode, as launchBrowser defaults it: hosted only for --hosted-mode.
      hostedMode:
        options.hostedMode ?? process.env.NEO_NODE_HOSTED_MODE === '1',
    })
    native = await runtime.openRun({ timeoutMs: options.timeoutMs })
    const holder = {
      __neo_invoke: (name, input, site) =>
        finishing && name === 'events.next'
          ? parked
          : native.invoke(name, input, site),
      __neo_log: (level, text) => native.log(level, text),
      __neo_sleep: (ms) => native.sleep(ms),
      // The prelude's conformance switch (src/prelude/snapshot.ts): Playwright's spec tests pin
      // its exact snapshot text, so here `ariaSnapshot()` keeps upstream's modes and output
      // instead of Neo's condensed v1 format. Production hosts never set it.
      __neo_upstream_snapshots: true,
    }
    const sources = {}
    for (const file of files) {
      const filename = fileURLToPath(new URL(`../../${file}`, import.meta.url))
      const bytes = fs.readFileSync(filename)
      sources[file] = createHash('sha256').update(bytes).digest('hex')
      // A parameter shadows globalThis only for these scripts. No vm context: their Promise,
      // Error and Uint8Array are Node's own. Source bytes are unchanged, after the strict prefix
      // the production hosts apply; the bootstrap writes __neo only onto holder.
      vm.compileFunction(
        `'use strict';\n${bytes.toString('utf8')}`,
        ['globalThis', '__neo'],
        { filename },
      )(holder, holder.__neo)
    }
    // Node libraries (including Playwright's image comparators) require Node Buffer identity.
    // Adapt bytes at this host boundary through the bootstrap's public hooks, leaving the
    // production bundles and their QuickJS Buffer implementation unchanged.
    holder.__neo.brands.register('bytes', (json) =>
      Buffer.from(String(json.base64 ?? ''), 'base64'),
    )
    holder.__neo.provide('Buffer', () => Buffer)
    const names = holder.__neo.parameters().split(/,\s*/)
    let ready
    let entered = false
    const parameters = new Promise((resolve) => {
      ready = resolve
    })
    const lifetime = new Promise((resolve) => {
      releaseMain = resolve
    })
    running = holder.__neo.run(async (...values) => {
      entered = true
      ready(
        Object.fromEntries(names.map((name, index) => [name, values[index]])),
      )
      await lifetime
    }, native.init)
    const params = await Promise.race([
      parameters,
      running.then((result) => {
        if (!entered) throw new Error(`Claw failed to initialize: ${result}`)
      }),
    ])
    return {
      ...params,
      params,
      native,
      runtime,
      sources,
      finish() {
        if (!finished) {
          finishing = true
          releaseMain()
          finished = (async () => {
            try {
              const reply = await native.finish()
              await running
              // Let already queued listener rejections reach onUncaught before removing it.
              await new Promise((resolve) => setImmediate(resolve))
              return reply
            } finally {
              await close()
            }
          })()
        }
        return finished
      },
    }
  } catch (error) {
    finishing = true
    releaseMain?.()
    // `running` rejects again when it was the setup failure, and close() can fail to flush
    // traces; either must not replace the setup error below.
    const cleanup = await (async () => {
      try {
        await native?.finish()
        await running
      } finally {
        await close()
      }
    })().then(
      () => undefined,
      (failure) => failure,
    )
    // Nothing here is a test's own SDK call: connecting to BrowserOS, opening the run or
    // evaluating the three files failed, so the report must count it as the harness's.
    const also = cleanup
      ? `; cleanup also failed: ${cleanup?.message ?? cleanup}`
      : ''
    throw new Error(
      `harness-error: Claw setup failed: ${error?.message ?? error}${also}`,
      { cause: error },
    )
  }
}
