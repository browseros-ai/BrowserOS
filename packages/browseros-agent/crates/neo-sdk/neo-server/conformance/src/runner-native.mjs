/** Each native program has its own Node process. Killing that process after the returned record
 * prevents a timed-out async body or an unhandled rejection from contaminating the next test. */
import { spawn } from 'node:child_process'
import { readFileSync, rmSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect } from '@playwright/test'
import { chromium } from 'playwright-core'
import { sha } from './upstream.mjs'
export async function runNative({
  file,
  row,
  env,
  output,
  budget = 165000,
  signal,
}) {
  signal?.throwIfAborted()
  const workerOutput = `${output}.worker`
  return await new Promise((resolve) => {
    const child = spawn(
      process.execPath,
      [
        fileURLToPath(import.meta.url),
        '--worker',
        file,
        env.nativeCdp,
        workerOutput,
      ],
      { stdio: 'ignore' },
    )
    let failure
    const terminate = (reason) => {
      failure ??= reason
      child.kill('SIGKILL')
    }
    const interrupted = () => terminate(String(signal.reason))
    signal?.addEventListener('abort', interrupted, { once: true })
    const timer = setTimeout(
      () => terminate('native worker deadline exceeded'),
      budget,
    )
    child.once('error', (error) => {
      failure ??= String(error)
    })
    // Only resolve after close. The orchestrator retains the partial attempt and tears down
    // browsers after the worker has exited, including on a signal.
    child.once('close', () => {
      clearTimeout(timer)
      signal?.removeEventListener('abort', interrupted)
      let record
      try {
        if (failure) throw new Error(failure)
        record = JSON.parse(readFileSync(workerOutput))
      } catch {
        record = {
          transport: { error: failure || 'native worker returned no record' },
        }
      }
      rmSync(workerOutput, { force: true })
      record.id ??= row.id
      record.provenance = {
        side: 'native',
        programSha256: sha(readFileSync(file)),
        browserVersion: env.browserVersion,
        playwrightCore: '1.64.1',
      }
      resolve(record)
    })
  })
}
if (process.argv[2] === '--worker') {
  const [file, url, output] = process.argv.slice(3),
    warnings = []
  let phase = 'registration'
  process.on('unhandledRejection', (error) => {
    if (phase === 'test') warnings.push(String(error))
  })
  let browser, record
  try {
    browser = await chromium.connectOverCDP(url, { isLocal: true })
    const context = browser.contexts()[0]
    const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor
    record = await new AsyncFunction(
      'context',
      'expect',
      'setTimeout',
      'clearTimeout',
      'Buffer',
      'console',
      '__conformancePhase',
      readFileSync(file, 'utf8'),
    )(context, expect, setTimeout, clearTimeout, Buffer, console, (value) => {
      phase = value
    })
    record.warnings = warnings
  } catch (error) {
    record = { transport: { error: String(error) } }
  }
  writeFileSync(output, JSON.stringify(record))
  // The parent owns the browsers; this client disconnects without closing the shared profile.
  await browser?.close()
  process.exit(0)
}
