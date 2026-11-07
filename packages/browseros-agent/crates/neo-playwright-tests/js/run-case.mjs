/** Owns both sequential engine phases, upstream runner groups and one leased server origin. */
import { spawn } from 'node:child_process'
import { randomInt, randomUUID } from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { OwnedBrowser } from './browser.mjs'
import { codecFactory } from './codec.mjs'
import { nativeScript } from './native.mjs'
import { source } from './prepare.mjs'

const evidenceCodec = codecFactory({
  identify: () => undefined,
  resolve: () => {
    throw new Error('Evidence cannot contain native objects')
  },
})

export async function leasePorts() {
  const root = path.join(os.tmpdir(), 'neo-playwright-tests-ports')
  fs.mkdirSync(root, { recursive: true })
  for (let attempt = 0; attempt < 100; attempt++) {
    const port = 16000 + randomInt(8000) * 4
    const lock = path.join(root, String(port))
    try {
      fs.mkdirSync(lock)
    } catch (error) {
      if (error.code === 'EEXIST') continue
      throw error
    }
    const sockets = []
    try {
      for (let i = 0; i < 4; i++) {
        const socket = net.createServer()
        sockets.push(socket)
        await new Promise((resolve, reject) => {
          socket.once('error', reject)
          socket.listen(port + i, '127.0.0.1', resolve)
        })
      }
      return {
        port,
        release() {
          fs.rmdirSync(lock)
        },
      }
    } catch (error) {
      fs.rmdirSync(lock)
      if (error.code !== 'EADDRINUSE') throw error
    } finally {
      await Promise.all(
        sockets.map(
          (socket) => new Promise((resolve) => socket.close(resolve)),
        ),
      )
    }
  }
  throw new Error('Could not lease four nonzero server ports')
}

export async function runner({
  overlay,
  outputDir,
  entry,
  port,
  kind,
  engine,
}) {
  const catalog = path.join(outputDir, 'catalog.json')
  const result = path.join(outputDir, 'runner.json')
  const journal = path.join(outputDir, 'journal.json')
  const stdout = fs.openSync(path.join(outputDir, 'runner-stdout.log'), 'wx')
  const stderr = fs.openSync(path.join(outputDir, 'runner-stderr.log'), 'wx')
  const child = spawn(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      path.join(overlay, 'neo.config.mjs'),
      '--output',
      path.join(outputDir, 'runner-output'),
    ],
    {
      detached: true,
      stdio: ['ignore', stdout, stderr],
      env: {
        ...process.env,
        PWTEST_UNDER_TEST: '1',
        PWDEBUG: '0',
        PWPAGE_IMPL: '',
        NEO_TESTS_OVERLAY: overlay,
        NEO_TESTS_CASE: entry.key,
        NEO_TESTS_CATALOG: catalog,
        NEO_TESTS_RESULT: result,
        NEO_TESTS_JOURNAL: journal,
        NEO_TESTS_PORT: String(port),
        NEO_TESTS_ENGINE: engine,
      },
    },
  )
  fs.closeSync(stdout)
  fs.closeSync(stderr)
  let timedOut = false
  let kill
  const interrupt = () => {
    timedOut = true
    try {
      process.kill(-child.pid, 'SIGINT')
    } catch {}
    kill = setTimeout(() => {
      try {
        process.kill(-child.pid, 'SIGKILL')
      } catch {}
    }, 10_000)
  }
  const timer = setTimeout(interrupt, 650_000)
  process.once('SIGTERM', interrupt)
  process.once('SIGINT', interrupt)
  const exit = await new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolve({ code, signal }))
  }).finally(() => {
    clearTimeout(timer)
    clearTimeout(kill)
    process.off('SIGTERM', interrupt)
    process.off('SIGINT', interrupt)
  })
  const errors = []
  if (timedOut)
    errors.push(
      'Upstream runner interrupted or exceeded finite supervisor guard',
    )
  const read = (file) => {
    try {
      return JSON.parse(fs.readFileSync(file, 'utf8'))
    } catch (error) {
      errors.push(`Missing or invalid ${path.basename(file)}: ${error.message}`)
      return null
    }
  }
  const raw = read(result)
  // Keep raw runner.json intact. Public errors in the Rust-facing pair use UTF-16
  // tags too, so a lone surrogate in an assertion message cannot break JSON decoding.
  if (raw) {
    raw.errors = raw.errors.map((error) => evidenceCodec.encode(error))
    for (const test of raw.outcomes)
      test.errors = test.errors.map((error) => evidenceCodec.encode(error))
  }
  const skipped =
    raw?.outcomes?.length === 1 && raw.outcomes[0].rawStatus === 'skipped'
  const calls = fs.existsSync(journal) ? read(journal) : null
  if (!calls && !skipped)
    errors.push('Engine fixture did not produce a journal')
  return {
    kind,
    exit,
    runner: raw,
    journal: calls,
    infrastructureErrors: errors,
  }
}

export async function runCase({
  entry,
  overlay,
  outputDir,
  executable,
  worker,
}) {
  const lease = await leasePorts()
  const pair = {
    entry,
    phases: {},
    port: lease.port,
    host: {
      name: process.versions.bun ? 'bun' : 'node',
      version: process.versions.bun || process.versions.node,
    },
  }
  try {
    for (const kind of ['reference', 'neo']) {
      const phaseDir = path.join(outputDir, kind)
      fs.mkdirSync(phaseDir, { recursive: true })
      const errors = []
      let browser
      try {
        browser = await OwnedBrowser.start({ executable, outputDir: phaseDir })
        const script = path.join(phaseDir, 'native.js')
        fs.writeFileSync(script, nativeScript(randomUUID()))
        const engine = path.join(phaseDir, 'engine.json')
        fs.writeFileSync(
          engine,
          JSON.stringify({
            kind,
            executable: worker,
            endpoint: browser.endpoint,
            pageId: browser.pageId,
            version: browser.version,
            script,
            outputDir: phaseDir,
          }),
        )
        pair.phases[kind] = await runner({
          overlay,
          outputDir: phaseDir,
          entry,
          port: lease.port,
          kind,
          engine,
        })
        const observation = await browser.snapshot()
        pair.phases[kind].observation = {
          status: observation.status,
          data: evidenceCodec.encode(observation),
        }
        pair.phases[kind].pageId = browser.pageId
      } catch (error) {
        errors.push(String(error.stack || error))
      } finally {
        if (browser) {
          try {
            await browser.close()
          } catch (error) {
            errors.push(`Cleanup: ${error.stack || error}`)
          }
        }
      }
      pair.phases[kind] ||= { kind, infrastructureErrors: [] }
      pair.phases[kind].infrastructureErrors.push(...errors)
    }
  } finally {
    lease.release()
  }
  fs.writeFileSync(
    path.join(outputDir, 'pair.json.tmp'),
    JSON.stringify(pair, null, 2),
  )
  fs.renameSync(
    path.join(outputDir, 'pair.json.tmp'),
    path.join(outputDir, 'pair.json'),
  )
  return pair
}

if (process.argv[1]?.endsWith('/run-case.mjs')) {
  const settings = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'))
  await runCase(settings)
}
