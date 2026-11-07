/** Neo: Owns whole-case cancellation, both sequential engine phases, and their private children. */
import { execFileSync, spawn } from 'node:child_process'
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
const abortError = (signal) => signal.reason || new Error('Case cancelled')
const check = (signal) => {
  if (signal?.aborted) throw abortError(signal)
}

/** The parent watchdog and signal handlers share one token, including setup and cleanup.
 * PPID changes detect a directly killed Cargo parent without signalling any parent PID. */
export class CaseCancellation {
  constructor({
    host = process,
    parentPid = host.ppid,
    parentPollMs = 100,
  } = {}) {
    this.host = host
    this.controller = new AbortController()
    this.signal = this.controller.signal
    this.parent = parentPid
    this.handlers = new Map(
      ['SIGINT', 'SIGTERM', 'SIGHUP'].map((name) => [
        name,
        () => this.cancel(`Case received ${name}`),
      ]),
    )
    for (const [name, handler] of this.handlers) host.on(name, handler)
    if (
      !Number.isSafeInteger(parentPid) ||
      parentPid < 1 ||
      host.ppid !== parentPid
    )
      this.cancel('Case parent exited before startup or has invalid identity')
    this.watchdog = setInterval(() => {
      if (host.ppid !== this.parent)
        this.cancel('Case parent exited or changed identity')
    }, parentPollMs)
  }

  cancel(message) {
    if (!this.signal.aborted) this.controller.abort(new Error(message))
  }

  dispose() {
    clearInterval(this.watchdog)
    for (const [name, handler] of this.handlers) this.host.off(name, handler)
  }
}

async function cancellable(operation, signal) {
  check(signal)
  let onAbort
  try {
    return await Promise.race([
      operation,
      new Promise((_, reject) => {
        onAbort = () => reject(abortError(signal))
        signal.addEventListener('abort', onAbort, { once: true })
        if (signal.aborted) onAbort()
      }),
    ])
  } finally {
    signal.removeEventListener('abort', onAbort)
  }
}

export async function leasePorts({ signal } = {}) {
  const root = path.join(os.tmpdir(), 'neo-playwright-tests-ports')
  fs.mkdirSync(root, { recursive: true })
  for (let attempt = 0; attempt < 100; attempt++) {
    check(signal)
    const port = 16000 + randomInt(8000) * 4
    const lock = path.join(root, String(port))
    try {
      fs.mkdirSync(lock)
    } catch (error) {
      if (error.code === 'EEXIST') continue
      throw error
    }
    const sockets = []
    let leased = false
    let released = false
    const release = () => {
      if (released) return
      fs.rmdirSync(lock)
      released = true
    }
    try {
      // The lock coordinates cases, but only actual binding establishes availability.
      // Both phases retain this same block after the temporary reservation sockets close.
      for (let i = 0; i < 4; i++) {
        check(signal)
        const socket = net.createServer()
        sockets.push(socket)
        await new Promise((resolve, reject) => {
          socket.once('error', reject)
          socket.listen(port + i, '127.0.0.1', resolve)
        })
      }
      check(signal)
      leased = true
      return { port, release }
    } catch (error) {
      if (error.code !== 'EADDRINUSE') throw error
    } finally {
      await Promise.all(
        sockets.map(
          (socket) => new Promise((resolve) => socket.close(resolve)),
        ),
      )
      if (!leased) release()
    }
  }
  throw new Error('Could not lease four nonzero server ports')
}

/** A raw report cannot establish successful execution after a crash or inconsistent exit.
 * Expected assertion failures remain valid runner executions and are classified in Rust. */
export function validateRunnerExit(exit, raw) {
  const errors = []
  if (!exit || exit.signal !== null || !Number.isInteger(exit.code))
    errors.push(
      `Upstream runner did not exit normally: ${JSON.stringify(exit)}`,
    )
  const expected = { passed: 0, failed: 1, timedout: 1 }
  if (!raw || !Object.hasOwn(expected, raw.status))
    errors.push(
      `Unknown or incomplete upstream runner status: ${String(raw?.status)}`,
    )
  else if (exit?.code !== expected[raw.status])
    errors.push(
      `Upstream runner exit ${exit?.code} contradicts status ${raw.status}`,
    )
  return errors
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
function signalGroup(pid, signal) {
  if (!Number.isSafeInteger(pid) || pid < 1) return false
  try {
    process.kill(-pid, signal)
    return true
  } catch (error) {
    if (error.code === 'ESRCH') return false
    throw error
  }
}

function runnerGroupAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid < 1) return false
  const listing = execFileSync('ps', ['-Ao', 'pgid=,state='], {
    encoding: 'utf8',
    timeout: 2_000,
    maxBuffer: 16 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  // kill(-pgid, 0) can return EPERM for a freshly killed macOS group. Zombie
  // entries have no writers left and must not trigger signals or a false leak.
  return listing.split('\n').some((line) => {
    const match = /^\s*(\d+)\s+(\S+)/.exec(line)
    return match && Number(match[1]) === pid && !match[2].startsWith('Z')
  })
}

async function cleanupRunnerGroup(pid, errors, { groupGraceMs, killMs }) {
  if (!runnerGroupAlive(pid)) return
  const deadline = Date.now() + groupGraceMs
  while (runnerGroupAlive(pid) && Date.now() < deadline) await sleep(25)
  if (!runnerGroupAlive(pid)) return
  errors.push('Upstream runner left live descendants after exiting')
  try {
    signalGroup(pid, 'SIGKILL')
  } catch (error) {
    if (error.code !== 'EPERM' || runnerGroupAlive(pid)) throw error
    return
  }
  const killedBy = Date.now() + killMs
  while (runnerGroupAlive(pid) && Date.now() < killedBy) await sleep(25)
  if (runnerGroupAlive(pid))
    errors.push('Upstream runner group survived SIGKILL')
}

export async function runner(
  { overlay, outputDir, entry, port, kind, engine, signal },
  {
    spawnChild = spawn,
    cli = path.join(source, 'packages/playwright/cli.js'),
    timeoutMs = 650_000,
    interruptMs = 10_000,
    killMs = 2_000,
    groupGraceMs = 250,
  } = {},
) {
  check(signal)
  const catalog = path.join(outputDir, 'catalog.json')
  const result = path.join(outputDir, 'runner.json')
  const journal = path.join(outputDir, 'journal.json')
  const errors = []
  let stdout
  let stderr
  let child
  try {
    stdout = fs.openSync(path.join(outputDir, 'runner-stdout.log'), 'wx')
    stderr = fs.openSync(path.join(outputDir, 'runner-stderr.log'), 'wx')
    child = spawnChild(
      process.execPath,
      [
        cli,
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
  } finally {
    if (stdout !== undefined) fs.closeSync(stdout)
    if (stderr !== undefined) fs.closeSync(stderr)
  }
  let killTimer
  let reapTimer
  let interrupted = false
  let stopWaiting
  const incompleteExit = new Promise((resolve) => {
    stopWaiting = resolve
  })
  const interrupt = (reason) => {
    if (interrupted) return
    interrupted = true
    errors.push(reason)
    try {
      signalGroup(child.pid, 'SIGINT')
    } catch (error) {
      errors.push(String(error))
    }
    // The detached group also contains upstream workers and the native child. Reaping
    // only the CLI PID would leave those writers alive after the case was finalized.
    killTimer = setTimeout(() => {
      try {
        signalGroup(child.pid, 'SIGKILL')
      } catch (error) {
        errors.push(String(error))
      }
      reapTimer = setTimeout(() => {
        errors.push('Upstream runner did not reap after SIGKILL')
        child.unref()
        stopWaiting({ code: null, signal: null })
      }, killMs)
    }, interruptMs)
  }
  const onAbort = () => interrupt(String(abortError(signal)))
  const timer = setTimeout(
    () => interrupt('Upstream runner exceeded finite supervisor guard'),
    timeoutMs,
  )
  const exited = new Promise((resolve) => {
    child.once('error', (error) => {
      errors.push(`Upstream runner spawn failed: ${error}`)
      resolve({ code: null, signal: null })
    })
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
  signal?.addEventListener('abort', onAbort, { once: true })
  if (signal?.aborted) onAbort()
  let exit
  try {
    exit = await Promise.race([exited, incompleteExit])
    await cleanupRunnerGroup(child.pid, errors, { groupGraceMs, killMs })
  } finally {
    clearTimeout(timer)
    clearTimeout(killTimer)
    clearTimeout(reapTimer)
    signal?.removeEventListener('abort', onAbort)
  }
  const read = (file) => {
    try {
      return JSON.parse(fs.readFileSync(file, 'utf8'))
    } catch (error) {
      errors.push(`Missing or invalid ${path.basename(file)}: ${error.message}`)
      return null
    }
  }
  const raw = read(result)
  errors.push(...validateRunnerExit(exit, raw))
  // runner.json remains raw. The pair encodes public errors as UTF-16 so assertion
  // messages containing lone surrogates survive the Rust evidence boundary.
  if (raw) {
    if (!Array.isArray(raw.errors) || !Array.isArray(raw.outcomes))
      errors.push('Malformed upstream runner report')
    else {
      raw.errors = raw.errors.map((error) => evidenceCodec.encode(error))
      for (const test of raw.outcomes) {
        if (!Array.isArray(test.errors))
          errors.push('Malformed upstream test errors')
        else
          test.errors = test.errors.map((error) => evidenceCodec.encode(error))
      }
    }
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

function publishPair(outputDir, pair) {
  const temporary = path.join(outputDir, 'pair.json.tmp')
  fs.writeFileSync(temporary, JSON.stringify(pair, null, 2))
  fs.renameSync(temporary, path.join(outputDir, 'pair.json'))
}

const pendingPhase = (kind) => ({
  kind,
  exit: { code: null, signal: null },
  runner: null,
  journal: null,
  infrastructureErrors: ['Engine phase did not complete'],
})

/** The CLI always uses the pinned runner and owned browser. The second argument
 * exposes lifecycle fault seams to offline tests without changing upstream bodies. */
export async function runCase(
  { entry, overlay, outputDir, executable, worker, parentPid },
  {
    cancellation = new CaseCancellation({ parentPid }),
    lease = leasePorts,
    startBrowser = (settings) => OwnedBrowser.start(settings),
    runRunner = runner,
  } = {},
) {
  const { signal } = cancellation
  const pair = {
    entry,
    phases: Object.fromEntries(
      ['reference', 'neo'].map((kind) => [kind, pendingPhase(kind)]),
    ),
    port: null,
    cancellation: null,
    host: {
      name: process.versions.bun ? 'bun' : 'node',
      version: process.versions.bun || process.versions.node,
    },
  }
  let ports
  try {
    fs.mkdirSync(outputDir, { recursive: true })
    publishPair(outputDir, pair)
    check(signal)
    ports = await lease({ signal })
    pair.port = ports.port
    for (const kind of ['reference', 'neo']) {
      check(signal)
      const phaseDir = path.join(outputDir, kind)
      fs.mkdirSync(phaseDir, { recursive: true })
      const errors = []
      let browser
      try {
        // Startup owns its abort cleanup before returning a handle. A late successful
        // startup must never escape cancellation or start an upstream test afterward.
        browser = await startBrowser({
          executable,
          outputDir: phaseDir,
          signal,
        })
        check(signal)
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
        check(signal)
        pair.phases[kind] = await runRunner({
          overlay,
          outputDir: phaseDir,
          entry,
          port: ports.port,
          kind,
          engine,
          signal,
        })
        check(signal)
        const observation = await cancellable(browser.snapshot(), signal)
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
      pair.phases[kind].infrastructureErrors.push(...errors)
      publishPair(outputDir, pair)
    }
  } catch (error) {
    for (const phase of Object.values(pair.phases))
      phase.infrastructureErrors.push(String(error.stack || error))
  } finally {
    try {
      ports?.release()
    } catch (error) {
      for (const phase of Object.values(pair.phases))
        phase.infrastructureErrors.push(
          `Port lease cleanup: ${error.stack || error}`,
        )
    }
    if (signal.aborted) {
      pair.cancellation = String(abortError(signal))
      for (const phase of Object.values(pair.phases))
        phase.infrastructureErrors.push(pair.cancellation)
    }
    // Keep handlers installed until cleanup and atomic publication finish. An interrupted
    // case retains both incomplete phase records instead of losing the pair entirely.
    try {
      publishPair(outputDir, pair)
    } finally {
      cancellation.dispose()
    }
  }
  return pair
}

if (process.argv[1]?.endsWith('/run-case.mjs')) {
  const settings = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'))
  const pair = await runCase(settings)
  if (pair.cancellation) process.exitCode = 1
}
