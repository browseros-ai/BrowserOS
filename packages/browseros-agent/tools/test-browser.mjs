#!/usr/bin/env node
/** One command owns a fresh CDP browser and a test command, including cancellation and cleanup. */
/* biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This interactive CLI is not run as a cached Turbo task. */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import {
  cleanupBrowser,
  groupMembers,
  startOwnedBrowser,
  stopProcessGroup,
  writeReceipt as write,
} from './browser-process.mjs'

const self = fileURLToPath(import.meta.url)
const signalStatus = (signal) => 128 + os.constants.signals[signal]
const status = (exit) =>
  exit.code ?? (exit.signal ? signalStatus(exit.signal) : 125)
const read = (file) => JSON.parse(fs.readFileSync(file, 'utf8'))
const report = (message) => console.error(`[test-browser] ${message}`)

function completionCode(exit, cancellationReason, failed) {
  if (cancellationReason)
    return os.constants.signals[cancellationReason]
      ? signalStatus(cancellationReason)
      : 143
  const code = exit ? status(exit) : 125
  return code === 0 && failed ? 125 : code
}

function send(message) {
  return new Promise((resolve) => {
    if (!process.connected) return resolve()
    process.send(message, () => resolve())
  })
}

function executable() {
  const selected =
    process.env.NEO_E2E_BROWSER ||
    (process.platform === 'darwin'
      ? '/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo'
      : 'browserclaw')
  const candidates = selected.includes(path.sep)
    ? [path.resolve(selected)]
    : (process.env.PATH || '')
        .split(path.delimiter)
        .map((dir) => path.join(dir, selected))
  for (const candidate of candidates) {
    try {
      fs.accessSync(candidate, fs.constants.X_OK)
      return candidate
    } catch {}
  }
  throw new Error(
    `Browser executable not found: ${selected}; set NEO_E2E_BROWSER`,
  )
}

function cancellation() {
  const controller = new AbortController()
  const stop = (reason) => controller.abort(reason)
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP'])
    process.on(signal, () => stop(signal))
  process.on('disconnect', () => stop('launcher disconnected'))
  return controller
}

/** The keeper stays in the command group after its main child exits, preserving ownership proof. */
async function commandKeeper() {
  let started = false
  let closing
  const close = () =>
    (closing ??= (async () => {
      // The keeper handles its own TERM while descendants drain, keeping the group identifiable.
      process.kill(-process.pid, 'SIGTERM')
      const deadline = Date.now() + 2000
      while (Date.now() < deadline) {
        if (groupMembers(process.pid).every((pid) => pid === process.pid)) {
          if (process.connected) process.disconnect()
          return
        }
        await sleep(50)
      }
      process.kill(-process.pid, 'SIGKILL')
    })())
  process.on('SIGTERM', () => {
    void close()
  })
  process.on('SIGINT', () => {
    void close()
  })
  process.on('SIGHUP', () => {
    void close()
  })
  process.on('disconnect', () => {
    void close()
  })
  process.on('message', (message) => {
    if (message.type !== 'start' || started || closing) return
    started = true
    const child = spawn(message.command[0], message.command.slice(1), {
      cwd: message.cwd,
      env: message.env,
      stdio: 'inherit',
    })
    child.once('error', (error) => {
      report(`cannot start command: ${error.message}`)
      void send({
        type: 'result',
        exit: { code: error.code === 'ENOENT' ? 127 : 126 },
      })
    })
    child.once('exit', (code, signal) => {
      void send({ type: 'result', exit: { code, signal } })
    })
  })
}

/** Detached from the CLI so IPC closure still drains both groups when the CLI is hard-killed. */
async function owner(runDir) {
  const controller = cancellation()
  const signal = controller.signal
  const receiptFile = path.join(runDir, 'result.json')
  const receipt = { owner: process.pid, state: 'starting' }
  let browser
  let keeper
  let keeperExited
  let commandExit
  let failure
  const errors = []
  const message = await new Promise((resolve) => {
    process.once('message', resolve)
    signal.addEventListener('abort', () => resolve(undefined), { once: true })
  })
  try {
    signal.throwIfAborted()
    if (message?.type !== 'start') throw new Error('Missing command')
    write(receiptFile, receipt)
    browser = await startOwnedBrowser(message.executable, runDir, signal)
    receipt.browser = {
      pid: browser.pid,
      profile: browser.profile,
      home: browser.home,
      temp: browser.temp,
      port: browser.port,
    }
    signal.throwIfAborted()
    // A separate keeper anchors this group while descendants of a completed test are drained.
    keeper = spawn(process.execPath, [self, '--command-owner', runDir], {
      detached: true,
      stdio: ['inherit', 'inherit', 'inherit', 'ipc'],
    })
    keeperExited = new Promise((resolve) => {
      keeper.once('error', (error) => resolve({ error: error.message }))
      keeper.once('exit', (code, signal) => resolve({ code, signal }))
    })
    receipt.command = { pid: keeper.pid, marker: runDir }
    receipt.state = 'running'
    write(receiptFile, receipt)
    const result = new Promise((resolve) => {
      keeper.on('message', (event) => {
        if (event.type === 'result')
          resolve({ kind: 'command', exit: event.exit })
      })
    })
    keeper.send({
      type: 'start',
      command: message.command,
      cwd: message.cwd,
      env: { ...process.env, BROWSEROS_CDP_PORT: String(browser.port) },
    })
    const interrupted = new Promise((resolve) => {
      const abort = () => resolve({ kind: 'cancel' })
      signal.addEventListener('abort', abort, { once: true })
      if (signal.aborted) abort()
    })
    const outcome = await Promise.race([
      result,
      interrupted,
      browser.exited.then(() => ({ kind: 'browser' })),
      keeperExited.then(() => ({ kind: 'keeper' })),
    ])
    if (outcome.kind === 'command') commandExit = outcome.exit
    else if (outcome.kind !== 'cancel')
      throw new Error(`${outcome.kind} exited before command completion`)
  } catch (error) {
    if (!signal.aborted) failure = String(error.message || error)
    // Startup may reject only after its own cleanup failed; retain that failure on cancellation too.
    if (error instanceof AggregateError)
      errors.push(...error.errors.slice(1).map((cleanup) => cleanup.message))
  } finally {
    try {
      if (keeper?.pid) await stopProcessGroup(receipt.command, 'command')
      if (keeperExited) await keeperExited
    } catch (error) {
      errors.push(error.message)
      if (keeper?.connected) keeper.disconnect()
      keeper?.unref()
    }
    try {
      await browser?.close({ retainState: errors.length > 0 })
    } catch (error) {
      errors.push(error.message)
    }
    const cancelled = signal.aborted
    Object.assign(receipt, {
      state: 'finished',
      commandExit,
      cancelled: cancelled ? signal.reason : undefined,
      failure,
      cleanupErrors: errors,
      cleaned: errors.length === 0,
      code: completionCode(
        commandExit,
        cancelled ? signal.reason : undefined,
        failure || errors.length,
      ),
    })
    write(receiptFile, receipt)
    if (failure && process.connected) report(failure)
    for (const error of errors)
      if (process.connected) report(`cleanup failed: ${error}`)
    await send({ type: 'complete', code: receipt.code })
    if (process.connected) process.disconnect()
    process.exitCode = receipt.code
  }
}

async function main() {
  if (process.platform === 'win32')
    throw new Error('test:browser requires macOS or Linux process groups')
  const command = process.argv.slice(2)
  if (command[0] === '--') command.shift()
  if (command[0] === '--help' || command.length === 0) {
    console.error(
      'Usage: bun run test:browser -- <command> [arguments...]\nNEO_E2E_BROWSER overrides the installed Neo executable.',
    )
    return command.length ? 0 : 125
  }
  const binary = executable()
  const root = fileURLToPath(new URL('../.llm/test-browser/', import.meta.url))
  fs.mkdirSync(root, { recursive: true })
  const runDir = fs.mkdtempSync(path.join(root, 'run-'))
  report(`artifacts: ${runDir}`)
  const child = spawn(process.execPath, [self, '--owner', runDir], {
    detached: true,
    stdio: ['inherit', 'inherit', 'inherit', 'ipc'],
  })
  let completed
  const stop = (signal) => {
    if (child.connected) child.kill(signal)
  }
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP'])
    process.on(signal, () => stop(signal))
  child.on('message', (message) => {
    if (message.type === 'complete') completed = message.code
  })
  const exited = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code) => resolve(code))
  })
  child.send({ type: 'start', executable: binary, command, cwd: process.cwd() })
  await exited
  if (completed !== undefined) return completed
  // The CLI can recover a failed owner; only the recorded private groups are eligible.
  const resultFile = path.join(runDir, 'result.json')
  if (fs.existsSync(resultFile) && read(resultFile).state === 'finished')
    return read(resultFile).code
  const result = fs.existsSync(resultFile) ? read(resultFile) : {}
  const errors = []
  try {
    if (result.command?.pid) await stopProcessGroup(result.command, 'command')
  } catch (error) {
    errors.push(error.message)
  }
  const browserFile = path.join(runDir, 'owner.json')
  try {
    if (fs.existsSync(browserFile))
      await cleanupBrowser(read(browserFile), runDir, {
        retainState: errors.length > 0,
      })
    else
      errors.push(
        'Browser ownership record unavailable; retained state for inspection',
      )
  } catch (error) {
    errors.push(error.message)
  }
  const failure = 'Browser owner exited unexpectedly'
  write(resultFile, {
    ...result,
    state: 'finished',
    failure,
    code: 125,
    cleanupErrors: errors,
    cleaned: errors.length === 0,
  })
  report(`${failure}; artifacts: ${runDir}`)
  return 125
}

if (process.argv[2] === '--owner' && process.send) {
  await owner(process.argv[3])
} else if (process.argv[2] === '--command-owner' && process.send) {
  await commandKeeper()
} else {
  try {
    process.exitCode = await main()
  } catch (error) {
    report(error.message)
    process.exitCode = 125
  }
}
