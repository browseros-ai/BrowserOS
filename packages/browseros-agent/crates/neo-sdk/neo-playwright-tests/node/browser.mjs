/** Neo: Owns private browser groups; a separate guardian survives worker and launcher death. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { verifyViewport } from './browser-defaults.mjs'

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
      '--headless=new',
      '--disable-browseros-server',
      '--disable-browseros-extensions',
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--no-first-run',
      '--lang=en-US',
      hostedMode ? '--window-size=1280,841' : '--window-size=1520,801',
      '--force-device-scale-factor=1',
      // DIAGNOSTIC (diag/lh001-flags, never lands): one group's extra switches.
      ...(process.env.NEO_DIAG_CHROME_FLAGS ?? '').split(' ').filter(Boolean),
      'about:blank',
    ],
    env: { BROWSERCLAW_HOSTED_MODE: hostedMode ? '1' : null },
    viewportReason:
      'Window sizes fit BrowserOS 0.52.1.1 chrome (240x81px normal, 0x121px hosted); other releases are resized to the measured chrome, and both launch and native tabs must verify 1280x720@1',
  }
}

export function isAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if (error.code === 'ESRCH') return false
    // macOS can return EPERM while the last sandboxed helper is being reaped.
    // Check existence without treating that transient denial as an ownership failure.
    if (error.code === 'EPERM') {
      if (pid > 0) return true
      return execFileSync('ps', ['-axo', 'pgid='], { encoding: 'utf8' })
        .split('\n')
        .some((group) => Number(group.trim()) === -pid)
    }
    throw error
  }
}

function groupOwned(pid, profile) {
  // Host command lines can exceed execFileSync's output buffer and strand a cleanup slot.
  // Enumerate compact numeric ids first, then read commands only in the owned group.
  const members = execFileSync('ps', ['-axo', 'pid=,pgid='], {
    encoding: 'utf8',
    // Concurrent agents can have very large command lines. Ownership must still be checked
    // before cleanup; Node's 1 MiB default can otherwise strand the private browser group.
    maxBuffer: 16 * 1024 * 1024,
  })
    .split('\n')
    .flatMap((line) => {
      const [member, group] = line.trim().split(/\s+/).map(Number)
      return group === pid && Number.isInteger(member) && member > 0
        ? [member]
        : []
    })
  if (!members.length) return false
  let processes
  try {
    processes = execFileSync(
      'ps',
      ['-p', members.join(','), '-o', 'pgid=,command='],
      {
        encoding: 'utf8',
      },
    )
  } catch (error) {
    // Every listed member can exit between the two reads. Ownership then fails closed.
    if (error.status === 1) return false
    throw error
  }
  // Recheck the group as well as the profile: a pid can be reused between the reads.
  return processes.split('\n').some((line) => {
    const match = line.trim().match(/^(\d+)\s+(.*)$/)
    return (
      Number(match?.[1]) === pid &&
      match[2].includes(`--user-data-dir=${profile}`)
    )
  })
}

/** Revalidate immediately before each signal: recorded process IDs alone prove no ownership. */
export async function stopBrowser(owner) {
  if (!owner.pid || !isAlive(-owner.pid)) return
  for (const [signal, attempts] of [
    ['SIGTERM', 40],
    ['SIGKILL', 60],
  ]) {
    if (!isAlive(-owner.pid)) return
    if (!groupOwned(owner.pid, owner.profile)) {
      // The group can finish exiting between the liveness probe and the ps listing; a group
      // that is gone needs no signal. One that is alive but unproven is never signalled.
      if (!isAlive(-owner.pid)) return
      throw new Error(`Cannot revalidate browser group ${owner.pid}`)
    }
    try {
      process.kill(-owner.pid, signal)
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
    for (let attempt = 0; attempt < attempts; attempt++) {
      if (!isAlive(-owner.pid)) return
      await sleep(50)
    }
  }
  throw new Error(`Owned browser process group ${owner.pid} survived cleanup`)
}

/** Used only inside the guardian, which installs cancellation before acquiring resources. */
export async function startOwnedBrowser(executable, outputDir, signal) {
  const profile = fs.mkdtempSync(path.join(outputDir, 'profile-'))
  const owner = { profile }
  let stdout
  let stderr
  let child
  let exit
  let exited
  let closing
  const close = () =>
    (closing ??= (async () => {
      await stopBrowser(owner)
      if (exited) await exited
      fs.rmSync(profile, { recursive: true, force: true })
      fs.writeFileSync(
        path.join(outputDir, 'cleanup.json'),
        JSON.stringify({ ...owner, exit }),
      )
    })())
  try {
    signal.throwIfAborted()
    stdout = fs.openSync(path.join(outputDir, 'browser.stdout.log'), 'wx')
    stderr = fs.openSync(path.join(outputDir, 'browser.stderr.log'), 'wx')
    const launch = browserLaunch(executable, profile)
    fs.writeFileSync(
      path.join(outputDir, 'launch.json'),
      JSON.stringify(launch, null, 2),
    )
    child = spawn(executable, launch.flags, {
      detached: true,
      stdio: ['ignore', stdout, stderr],
      env: Object.fromEntries(
        Object.entries({ ...process.env, ...launch.env }).filter(
          ([, value]) => value !== null,
        ),
      ),
    })
    exited = new Promise((resolve) => {
      child.once('exit', (code, signal) => {
        exit = { code, signal }
        resolve(exit)
      })
      child.once('error', (error) => {
        exit = { error: error.message }
        resolve(exit)
      })
    })
    owner.pid = child.pid
    // No await between spawn and durable ownership: an IPC disconnect cannot strand a browser.
    fs.writeFileSync(path.join(outputDir, 'owner.json'), JSON.stringify(owner))
    if (!child.pid) throw new Error('Browser did not start')
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline) {
      signal.throwIfAborted()
      if (exit)
        throw new Error(`Browser exited at startup: ${JSON.stringify(exit)}`)
      try {
        const [port] = fs
          .readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8')
          .split('\n')
        if (/^\d+$/.test(port) && Number(port) > 0 && Number(port) !== 9200) {
          launch.viewport = await verifyViewport(Number(port))
          fs.writeFileSync(
            path.join(outputDir, 'launch.json'),
            JSON.stringify(launch, null, 2),
          )
          return { ...owner, port: Number(port), exited, close }
        }
      } catch (error) {
        if (error.code !== 'ENOENT') throw error
      }
      await sleep(50, undefined, { signal })
    }
    throw new Error('Browser did not publish its CDP port')
  } catch (error) {
    await close()
    throw error
  } finally {
    if (stdout !== undefined) fs.closeSync(stdout)
    if (stderr !== undefined) fs.closeSync(stderr)
  }
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
  if (
    path.dirname(owner.profile) !== dir ||
    !path.basename(owner.profile).startsWith('profile-')
  )
    throw new Error('Invalid owned profile path')
  await stopBrowser(owner)
  fs.rmSync(owner.profile, { recursive: true, force: true })
  fs.writeFileSync(cleaned, JSON.stringify({ ...owner, parentFallback: true }))
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
