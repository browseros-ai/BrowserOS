#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
/** Orchestrates one bounded run. Programs are generated once on the executing host, persisted,
 * then read unchanged by both runners. Reports preserve failures even when calibration aborts. */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { behavior } from './src/behavior.mjs'
import { start } from './src/environment.mjs'
import { interruption } from './src/interruption.mjs'
import { judge } from './src/judge.mjs'
import { load } from './src/loader.mjs'
import { parity } from './src/parity.mjs'
import { persistState, writeReport } from './src/run-report.mjs'
import { runNative } from './src/runner-native.mjs'
import { connectNeo, runNeo } from './src/runner-neo.mjs'
import { compareImages } from './src/screenshots.mjs'
import { materialize, sha } from './src/upstream.mjs'
import { viewport } from './src/viewport-rig.mjs'

const require = createRequire(import.meta.url),
  yaml = require('yaml')
const here = import.meta.dirname,
  server = resolve(here, '..'),
  agent = resolve(server, '../../..'),
  repo = resolve(agent, '../..')
const [command = 'help', ...args] = process.argv.slice(2)
const option = (name, fallback) => {
  const index = args.indexOf(name)
  return index < 0 ? fallback : args[index + 1]
}
const side = option('--side', 'both')
if (!['native', 'neo', 'both'].includes(side))
  throw new Error('--side must be native, neo or both')
for (const [name, integer] of [
  ['--repeat', true],
  ['--max-minutes', false],
  ['--ports', true],
]) {
  if (option(name) !== undefined) {
    const value = Number(option(name))
    if (
      !Number.isFinite(value) ||
      value <= 0 ||
      (integer && !Number.isSafeInteger(value)) ||
      (name === '--ports' && value > 65532)
    )
      throw new Error(`invalid ${name}`)
  }
}
const cache = resolve(
  process.env.CONFORMANCE_OUTPUT || join(agent, 'target/conformance'),
)
const json = (file, value) =>
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
const settings = () => ({
  deviations:
    yaml.parse(readFileSync(join(here, 'deviations.yaml'), 'utf8')) || [],
  oracleExceptions:
    yaml.parse(readFileSync(join(here, 'oracle-exceptions.yaml'), 'utf8')) ||
    [],
  unsupported: JSON.parse(readFileSync(join(here, 'unsupported.json'))),
})
if (command === 'behavior') {
  const output = join(cache, option('--run-id', `behavior-${Date.now()}`))
  const control = interruption()
  try {
    process.exitCode = (await behavior({
      server,
      filter: option('--test'),
      output,
      browser: process.env.BROWSEROS_APP,
      neoBinary: process.env.NEO_BINARY,
      gitSha:
        process.env.NEO_GIT_SHA ||
        execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], {
          encoding: 'utf8',
        }).trim(),
      ports: Number(option('--ports', 19561)),
      signal: control.signal,
    }))
      ? 0
      : 1
  } finally {
    control.dispose()
  }
} else if (command === 'judge') {
  process.exitCode = writeReport(resolve(args[0]), settings()).ok ? 0 : 1
} else if (
  ['inventory', 'show', 'run', 'selftest', 'compile'].includes(command)
) {
  const pin = materialize({
    checkout:
      process.env.PLAYWRIGHT_SRC || '/Users/shadowfax/code/oss/playwright',
    cache,
  })
  const catalog = load(pin.dir, pin, here)
  if (command === 'compile') {
    const dir = join(
      cache,
      option('--run-id', `compile-${Date.now()}`),
      'programs',
    )
    mkdirSync(dir, { recursive: true })
    const rows = catalog.inventory.tests.filter(
      (r) => r.selected && r.class !== 'excluded',
    )
    for (const row of rows)
      writeFileSync(
        join(dir, `${sha(row.id).slice(0, 16)}.js`),
        catalog.program(row.spec, row.ordinal, {
          platform: process.platform,
          fixtures: catalog.fixtures,
          server: {},
          assetDir: join(pin.dir, 'tests/assets'),
        }),
      )
    const compiler =
      process.env.CONFORMANCE_COMPILER ||
      join(dirname(process.env.NEO_BINARY), 'examples/conformance_compile')
    execFileSync(compiler, [dir], { timeout: 120000 })
    console.log(
      `Compiled ${rows.length} programs in the production QuickJS engine`,
    )
  } else if (command === 'inventory') {
    const output = `${JSON.stringify(catalog.inventory, null, 2)}\n`,
      file = join(here, 'inventory.json')
    if (args.includes('--check')) {
      if (readFileSync(file, 'utf8') !== output)
        throw new Error('inventory changed; regenerate and review the diff')
    } else writeFileSync(file, output)
    console.log(
      `Inventory: ${catalog.inventory.tests.length} tests; ${catalog.inventory.tests.filter((r) => r.selected && r.spec.startsWith('tests/page/')).length} core`,
    )
  } else {
    let rows = catalog.inventory.tests.filter(
      (r) =>
        r.selected &&
        r.spec.startsWith(command === 'selftest' ? 'selftest/' : 'tests/page/'),
    )
    if (command === 'run')
      rows.push(
        ...catalog.inventory.tests.filter((r) =>
          r.spec.startsWith('screenshots/'),
        ),
      )
    if (option('--spec')) {
      const pattern = option('--spec')
        .split('*')
        .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
        .join('.*')
      rows = rows.filter((r) => new RegExp(pattern).test(r.spec))
    }
    if (option('--test'))
      rows = rows.filter(
        (r) => r.id === option('--test') || r.id.includes(option('--test')),
      )
    if (command === 'show')
      rows = catalog.inventory.tests.filter((r) => r.id === args.join(' '))
    if (option('--shard')) {
      const [index, total] = option('--shard').split('/').map(Number)
      if (
        !Number.isInteger(index) ||
        !Number.isInteger(total) ||
        index < 1 ||
        index > total
      )
        throw new Error('shard must be i/n, 1 <= i <= n')
      rows = rows.filter((_, i) => i % total === index - 1)
    }
    if (!rows.length) throw new Error('no tests selected')
    if (command === 'show') {
      console.log(
        catalog.program(rows[0].spec, rows[0].ordinal, {
          platform: process.platform,
          testTimeout: 30000,
          expectTimeout: 10000,
          fixtures: catalog.fixtures,
          server: {},
          assetDir: join(pin.dir, 'tests/assets'),
        }),
      )
      process.exit(0)
    }
    const ports = Number(option('--ports', 19561)),
      runId = option(
        '--run-id',
        `conformance-${new Date().toISOString().replace(/[:.]/g, '-')}`,
      ),
      dir = join(cache, runId)
    if (existsSync(dir)) throw new Error(`run directory already exists: ${dir}`)
    for (const name of [
      'programs',
      'native',
      'neo',
      'logs',
      'images',
      'calibration/native',
      'calibration/neo',
    ])
      mkdirSync(join(dir, name), { recursive: true })
    const browser = process.env.BROWSEROS_APP,
      neoBinary = process.env.NEO_BINARY
    const repeat = Number(option('--repeat', 1))
    const inventory = {
      ...catalog.inventory,
      filtered: !!(
        option('--spec') ||
        option('--test') ||
        option('--shard') ||
        command === 'selftest'
      ),
      tests: command === 'selftest' ? [] : rows,
    }
    const state = {
      schema: 1,
      mode: command,
      side,
      repeat,
      phase: 'startup',
      status: 'running',
    }
    persistState(dir, state)
    json(join(dir, 'inventory.json'), inventory)
    const { signal, dispose } = interruption()
    let env, client, failure
    const native = [],
      neo = []
    try {
      json(
        join(dir, 'registration.json'),
        parity(catalog, pin, join(cache, `registration-${pin.commit}`)),
      )
      if (!browser || !neoBinary)
        throw new Error(
          'BROWSEROS_APP and NEO_BINARY must name actual executables',
        )
      env = await start({
        ports,
        browser,
        neoBinary,
        assetsDir: join(pin.dir, 'tests/assets'),
        runDir: dir,
        keep: args.includes('--keep'),
        gitSha:
          process.env.NEO_GIT_SHA ||
          execFileSync('git', ['-C', repo, 'rev-parse', 'HEAD'], {
            encoding: 'utf8',
          }).trim(),
        signal,
      })
      client = await connectNeo(env, signal)
      const config = {
        platform: process.platform,
        testTimeout: 30000,
        expectTimeout: 10000,
        fixtures: {
          ...catalog.fixtures,
          browserVersion: env?.browserVersion,
          browserMajorVersion: Number(
            env?.browserVersion?.split('/')[1]?.split('.')[0],
          ),
        },
        server: env
          ? Object.fromEntries(
              Object.entries(env.server).filter(([key]) => key !== 'close'),
            )
          : {},
        assetDir: join(pin.dir, 'tests/assets'),
      }
      const generated = new Map()
      function generate(row) {
        const file = join(dir, 'programs', `${sha(row.id).slice(0, 16)}.js`)
        row.programFile = `programs/${file.split('/').pop()}`
        writeFileSync(file, catalog.program(row.spec, row.ordinal, config))
        generated.set(row.id, file)
        return file
      }
      json(join(dir, 'run.json'), {
        runId,
        pin,
        platform: process.platform,
        node: process.version,
        gitSha: env.gitSha,
        neoBinarySha256: env.binarySha256,
        browserSha256: env.browserSha256,
        browserVersion: env.browserVersion,
        flags: env.flags,
        viewport: { ...viewport, source: 'rig' },
        ports,
        versions: Object.fromEntries(
          ['playwright', 'playwright-core', '@playwright/test'].map((p) => [
            p,
            require(`${p}/package.json`).version,
          ]),
        ),
        browserArtifact: process.env.BROWSEROS_ARTIFACT || null,
      })
      for (const row of [
        ...rows,
        ...catalog.inventory.tests.filter((r) =>
          r.spec.startsWith('selftest/'),
        ),
      ])
        if (row.class !== 'excluded') generate(row)
      json(join(dir, 'inventory.json'), inventory)
      const compiler =
        process.env.CONFORMANCE_COMPILER ||
        join(dirname(neoBinary), 'examples/conformance_compile')
      execFileSync(compiler, [join(dir, 'programs')], { timeout: 120000 })
      json(join(dir, 'compile.json'), {
        status: 'PASS',
        compiler,
        programs: generated.size,
      })
      const deadline = Date.now() + Number(option('--max-minutes', 120)) * 60000
      const restarts = []
      const refreshIfLeaked = async () => {
        const counts = await Promise.all(
          [env.nativeCdp, env.neoCdp].map(async (url) => {
            const response = await fetch(`${url}/json/list`, {
              signal: AbortSignal.timeout(5000),
            })
            return (await response.json()).filter(
              (target) => target.type === 'page',
            ).length
          }),
        )
        if (counts.every((count) => count <= 10)) return
        await client.close()
        await env.stop()
        env = await start({
          ports,
          browser,
          neoBinary,
          assetsDir: join(pin.dir, 'tests/assets'),
          runDir: dir,
          keep: args.includes('--keep'),
          gitSha: env.gitSha,
          signal,
        })
        client = await connectNeo(env, signal)
        restarts.push({ at: Date.now(), reason: 'page leak threshold', counts })
        json(join(dir, 'restarts.json'), restarts)
      }
      const observedPages = new WeakMap()
      const perform = async (row, attempt = 1, timeout) => {
        signal.throwIfAborted()
        if (Date.now() > deadline) throw new Error('suite budget exhausted')
        const rigs = env.viewportRigs
        const ready = Object.values(rigs).every((rig) => rig.health() === 'ok')
        const file = generated.get(row.id) || generate(row),
          key = sha(row.id).slice(0, 16),
          suffix = attempt > 1 ? `-${attempt}` : ''
        const calibration = state.phase === 'calibration'
        const recordsDir = calibration ? join(dir, 'calibration') : dir
        const output = (side) => join(recordsDir, side, `${key + suffix}.json`)
        let p, n
        if (ready && (calibration || side !== 'neo')) {
          p = await runNative({
            file,
            row,
            env,
            output: output('native'),
            signal,
          })
        }
        if (
          !signal.aborted &&
          Object.values(rigs).every((rig) => rig.health() === 'ok') &&
          (calibration || side !== 'native')
        ) {
          n = await runNeo({
            file,
            row,
            env,
            client,
            output: output('neo'),
            signal,
            taskId: runId,
            requestId: `${runId}/${key}/${attempt}`,
            timeout,
          })
        }
        // The protocol may deliver the last release reply after a runner reports completion.
        if (!signal.aborted)
          await Promise.all(Object.values(rigs).map((rig) => rig.settle()))
        let lost
        for (const [name, rig] of Object.entries(rigs)) {
          const pages = rig.pages()
          const failed = pages
            .slice(observedPages.get(rig) || 0)
            .find((entry) => entry.error)
          observedPages.set(rig, pages.length)
          const disconnected = rig.health() === 'lost'
          if (disconnected) lost = `viewport rig (${name}): connection lost`
          if (!failed && !disconnected) continue
          const record = (name === 'native' ? p : n) || { id: row.id }
          record.transport = {
            ...record.transport,
            error: `viewport rig (${name}): ${failed?.error || 'connection lost'}`,
            source: 'viewport-rig',
          }
          if (name === 'native') p = record
          else n = record
        }
        if (lost) {
          p ??= {
            id: row.id,
            transport: { error: lost, source: 'viewport-rig' },
          }
          n ??= {
            id: row.id,
            transport: { error: lost, source: 'viewport-rig' },
          }
        }
        if (p) {
          p.attempt = attempt
          json(output('native'), p)
          native.push(p)
        }
        if (n) {
          n.attempt = attempt
          json(output('neo'), n)
          neo.push(n)
        }
        signal.throwIfAborted()
        if (!lost) await refreshIfLeaked()
        return { p, n, lost }
      }
      // Calibrations must prove the two paths before any upstream result is counted.
      const expected = {
        'must-pass': 'pass',
        'oracle-fails': 'oracle-fail',
        'neo-refuses': 'unsupported',
        'caught-refusal': 'unsupported',
        'neo-differs': 'fail',
        'count-differs': 'fail:assertion-count',
        'runtime-globals': 'pass',
        viewport: 'pass',
        'transport-timeout': 'harness-error',
      }
      state.phase = 'calibration'
      persistState(dir, state)
      const calibrations = []
      const viewportMeasurement = {
        width: viewport.width,
        height: viewport.height,
        dpr: viewport.deviceScaleFactor,
      }
      for (const row of catalog.inventory.tests.filter((r) =>
        r.spec.startsWith('selftest/'),
      )) {
        const { p, n, lost } = await perform(
          row,
          1,
          row.titlePath[0] === 'transport-timeout' ? 1 : undefined,
        )
        const result = judge({
          inventory: { tests: [row], filtered: true },
          native: [p],
          neo: [n],
          ...settings(),
        }).results[0]
        calibrations.push({
          ...result,
          expected: expected[row.titlePath[0]],
          correct:
            result.classification === expected[row.titlePath[0]] &&
            [p, n].every(
              (record) => record.transport?.source !== 'viewport-rig',
            ) &&
            (row.titlePath[0] !== 'viewport' ||
              [p, n].every((record) =>
                ['viewport', 'viewportAfterNavigation'].every((stage) =>
                  Object.entries(viewportMeasurement).every(
                    ([key, value]) =>
                      record.measurements?.[stage]?.[key] === value,
                  ),
                ),
              )),
        })
        if (lost) {
          json(join(dir, 'calibration.json'), {
            closedPort: null,
            results: calibrations,
          })
          throw new Error(lost)
        }
      }
      // Closed-port calibration cannot dispatch anything. It exercises the MCP client failure path.
      let closedPort = false
      try {
        const c = await connectNeo(
          { ...env, neoMcp: 'http://127.0.0.1:9/mcp' },
          signal,
        )
        await c.close()
      } catch {
        closedPort = true
      }
      json(join(dir, 'calibration.json'), { closedPort, results: calibrations })
      if (!closedPort || calibrations.some((r) => !r.correct))
        throw new Error(
          'calibration failed; see calibration.json; upstream run aborted',
        )
      state.phase = 'suite'
      persistState(dir, state)
      native.length = 0
      neo.length = 0
      for (const row of inventory.tests) {
        if (row.class === 'excluded') continue
        for (let attempt = 1; attempt <= repeat; attempt++) {
          const { p, n, lost } = await perform(row, attempt)
          if (lost) throw new Error(lost)
          if (
            row.spec.startsWith('screenshots/') &&
            !p?.transport &&
            !n?.transport &&
            p?.status === 'passed' &&
            n?.status === 'passed'
          ) {
            const imageDir = join(dir, 'images', sha(row.id).slice(0, 16))
            mkdirSync(imageDir, { recursive: true })
            const result = compareImages(
              p.images || [],
              n.images || [],
              imageDir,
            )
            json(join(imageDir, 'comparison.json'), result)
            n.screenshotComparison = result
            if (!result.ok) {
              n.status = 'failed'
              n.errors.push({
                name: 'PixelMismatch',
                message: JSON.stringify(result),
              })
            }
          }
          if (n)
            json(
              join(
                dir,
                'neo',
                `${sha(row.id).slice(0, 16)}${attempt > 1 ? `-${attempt}` : ''}.json`,
              ),
              n,
            )
          console.log(
            `${native.length}/${rows.length} ${row.id} native=${p?.status || 'unpaired/transport'} neo=${n?.status || 'unpaired/transport'}`,
          )
        }
      }
    } catch (error) {
      failure = error
    } finally {
      try {
        try {
          await client?.close()
        } finally {
          await env?.stop()
        }
      } catch (error) {
        failure ??= error
      }
      dispose()
    }
    failure = signal.reason || failure
    state.status = signal.aborted
      ? 'interrupted'
      : failure
        ? 'failed'
        : 'completed'
    if (failure) {
      state.failure = String(failure)
      json(join(dir, 'failure.json'), {
        error: state.failure,
        stack: failure.stack,
      })
      console.error(`${failure}\nEvidence: ${dir}`)
    } else console.log(dir)
    persistState(dir, state)
    process.exitCode = writeReport(dir, settings()).ok ? 0 : 1
  }
} else
  console.log(
    'conformance inventory [--check] | show <id> | run [--spec substring] [--test id] [--ports base] [--shard i/n] [--repeat n] | selftest | judge <run dir>',
  )
