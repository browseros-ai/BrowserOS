/** Neo: Owns private browser groups; a separate guardian survives worker and launcher death. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import {
  browserFlags,
  cleanupBrowser,
  isAlive,
  startOwnedBrowser as startBrowser,
  stopBrowser,
} from '../../../../tools/browser-process.mjs'
import { verifyViewport } from './browser-defaults.mjs'

export { isAlive, stopBrowser }

export function browserVersion(executable) {
  if (!executable || process.platform !== 'darwin') return undefined
  const plist = path.join(path.dirname(path.dirname(executable)), 'Info.plist')
  return execFileSync(
    '/usr/libexec/PlistBuddy',
    ['-c', 'Print :CFBundleShortVersionString', plist],
    { encoding: 'utf8' },
  ).trim()
}

export function browserLaunch(
  executable,
  profile = '<worker profile>',
  hostedMode = process.env.NEO_NODE_HOSTED_MODE === '1',
) {
  return {
    executable,
    flags: [
      ...browserFlags(profile),
      '--lang=en-US',
      hostedMode ? '--window-size=1280,841' : '--window-size=1520,801',
      '--force-device-scale-factor=1',
      'about:blank',
    ],
    env: { BROWSERCLAW_HOSTED_MODE: hostedMode ? '1' : null },
    viewportReason:
      'Window sizes fit BrowserOS 0.52.1.1 chrome (240x81px normal, 0x121px hosted); other releases are resized to the measured chrome, and both launch and native tabs must verify 1280x720@1',
  }
}

/** Corpus-specific viewport calibration wraps the shared process lifecycle. */
export function startOwnedBrowser(executable, outputDir, signal) {
  return startBrowser(executable, outputDir, signal, {
    launch: browserLaunch,
    prepare: async (port) => ({ viewport: await verifyViewport(port) }),
  })
}

export async function launchBrowser(
  executable,
  outputDir,
  { slot, launcherPid } = {},
) {
  fs.mkdirSync(outputDir, { recursive: true })
  const guardianFile = fileURLToPath(
    new URL('./browser-owner.mjs', import.meta.url),
  )
  const log = fs.openSync(path.join(outputDir, 'guardian.log'), 'wx')
  let child
  try {
    child = spawn(process.execPath, [guardianFile, outputDir], {
      detached: true,
      stdio: ['ignore', log, log, 'ipc'],
    })
  } finally {
    fs.closeSync(log)
  }
  let ready
  let fail
  const started = new Promise((resolve, reject) => {
    ready = resolve
    fail = reject
  })
  let reportExit
  const exited = new Promise((resolve) => {
    reportExit = resolve
  })
  let error
  let closing
  child.on('message', (message) => {
    if (message.type === 'ready') ready(message)
    if (message.type === 'browser-exited') reportExit(message.exit)
    if (message.type === 'error') {
      error = new Error(message.message)
      fail(error)
    }
  })
  const stopped = new Promise((resolve) => {
    child.once('error', (cause) => {
      error = cause
      fail(cause)
      resolve()
    })
    child.once('exit', (code, signal) => {
      fail(
        error ||
          new Error(`Browser guardian exited before ready: ${code ?? signal}`),
      )
      reportExit({ code, signal, error: error?.message })
      resolve()
    })
  })
  const close = () =>
    (closing ??= (async () => {
      if (child.connected) child.send({ type: 'stop' }, () => {})
      await stopped
      if (error) throw error
    })())
  if (child.connected)
    child.send({ type: 'start', executable, slot, launcherPid }, (cause) => {
      if (cause) fail(cause)
    })
  try {
    const info = await started
    return { ...info, exited, close }
  } catch (error) {
    await close()
    throw error
  }
}

/** Launcher fallback after Playwright has exited. Guardians normally finish before this. */
export async function cleanupWorker(dir) {
  const cleaned = path.join(dir, 'cleanup.json')
  if (fs.existsSync(cleaned)) return
  const record = path.join(dir, 'owner.json')
  if (!fs.existsSync(record)) return
  const owner = JSON.parse(fs.readFileSync(record))
  await cleanupBrowser({ ...owner, parentFallback: true }, dir)
}

export async function cleanupWorkers(outputDir) {
  for (const entry of fs.readdirSync(outputDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('worker-')) continue
    const dir = path.join(outputDir, entry.name)
    const cleaned = path.join(dir, 'cleanup.json')
    for (let attempt = 0; !fs.existsSync(cleaned) && attempt < 100; attempt++) {
      const record = path.join(dir, 'guardian.json')
      if (
        !fs.existsSync(record) ||
        !isAlive(JSON.parse(fs.readFileSync(record)).pid)
      )
        break
      await sleep(100)
    }
    if (fs.existsSync(cleaned)) continue
    await cleanupWorker(dir)
  }
}
