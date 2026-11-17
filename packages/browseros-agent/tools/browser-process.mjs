/** Owns a private browser's process group, home, readiness, and cleanup for live test callers. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

/** Readers may recover a dead owner; publish each receipt as a complete JSON document. */
export function writeReceipt(file, value) {
  const temporary = `${file}.${process.pid}.tmp`
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, {
    flag: 'wx',
    mode: 0o600,
  })
  fs.renameSync(temporary, file)
}

/** Keep ps output numeric until the owned group is selected: busy agent hosts have huge argv. */
export function groupMembers(group) {
  return execFileSync('ps', ['-axo', 'pid=,pgid=,stat='], {
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  })
    .split('\n')
    .flatMap((line) => {
      const [pid, pgid, state] = line.trim().split(/\s+/)
      return Number(pgid) === group && !state?.startsWith('Z')
        ? [Number(pid)]
        : []
    })
}

export function isAlive(pid) {
  try {
    process.kill(pid, 0)
    return pid > 0 || groupMembers(-pid).length > 0
  } catch (error) {
    if (error.code === 'ESRCH') return false
    if (error.code === 'EPERM') return pid > 0 || groupMembers(-pid).length > 0
    throw error
  }
}

function groupOwned(pid, marker) {
  const members = groupMembers(pid)
  if (!members.length) return false
  let output
  try {
    output = execFileSync(
      'ps',
      ['-p', members.join(','), '-o', 'pgid=,command='],
      {
        encoding: 'utf8',
        maxBuffer: 16 * 1024 * 1024,
      },
    )
  } catch (error) {
    if (error.status === 1) return false
    throw error
  }
  return output.split('\n').some((line) => {
    const match = line.trim().match(/^(\d+)\s+(.*)$/)
    return Number(match?.[1]) === pid && match[2].includes(marker)
  })
}

/** A private argv marker proves current ownership before each signal, including escalation. */
export async function stopProcessGroup({ pid, marker }, label = 'process') {
  if (!pid) return
  for (const [signal, attempts] of [
    ['SIGTERM', 40],
    ['SIGKILL', 60],
  ]) {
    if (!isAlive(-pid)) return
    if (!groupOwned(pid, marker)) {
      if (!isAlive(-pid)) return
      throw new Error(`Cannot revalidate ${label} group ${pid}`)
    }
    try {
      process.kill(-pid, signal)
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
    for (let attempt = 0; attempt < attempts; attempt++) {
      if (!isAlive(-pid)) return
      await sleep(50)
    }
  }
  throw new Error(`Owned ${label} process group ${pid} survived cleanup`)
}

export function stopBrowser(owner) {
  return stopProcessGroup(
    { pid: owner.pid, marker: `--user-data-dir=${owner.profile}` },
    'browser',
  )
}

/** Recovery callers have no child handle; validate paths and prove the group stopped before deletion. */
export async function cleanupBrowser(
  owner,
  outputDir,
  { exited, retainState = false } = {},
) {
  for (const [key, prefix] of [
    ['profile', 'profile-'],
    ['home', 'home-'],
  ]) {
    if (
      owner[key] &&
      (path.dirname(owner[key]) !== outputDir ||
        !path.basename(owner[key]).startsWith(prefix))
    )
      throw new Error(`Invalid owned ${key} path`)
  }
  if (
    owner.temp &&
    (path.dirname(owner.temp) !== '/tmp' ||
      !path.basename(owner.temp).startsWith('browseros-test-'))
  )
    throw new Error('Invalid owned temporary path')
  await stopBrowser(owner)
  const exit = await exited
  // A different owned group may still be unproven; stopping this browser must not erase evidence.
  if (!retainState) {
    if (owner.profile)
      fs.rmSync(owner.profile, { recursive: true, force: true })
    if (owner.home) fs.rmSync(owner.home, { recursive: true, force: true })
    if (owner.temp) fs.rmSync(owner.temp, { recursive: true, force: true })
  }
  writeReceipt(path.join(outputDir, 'cleanup.json'), {
    ...owner,
    exit,
    retained: retainState || undefined,
  })
}

export function browserFlags(profile) {
  return [
    '--headless=new',
    '--disable-browseros-server',
    '--disable-browseros-server-updater',
    '--disable-browseros-extensions',
    '--remote-debugging-port=0',
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--use-mock-keychain',
  ]
}

function browserEnvironment(home, temp, overrides) {
  // A Chromium profile does not isolate Rust server state or coding harness homes.
  // These overrides belong only to the browser; the supplied command keeps the caller's env.
  const roots = {
    HOME: home,
    USERPROFILE: home,
    CFFIXED_USER_HOME: home,
    XDG_CONFIG_HOME: path.join(home, '.config'),
    XDG_DATA_HOME: path.join(home, '.local/share'),
    XDG_CACHE_HOME: path.join(home, '.cache'),
    APPDATA: path.join(home, '.config'),
    LOCALAPPDATA: path.join(home, '.local/share'),
    CLAUDE_CONFIG_DIR: path.join(home, '.claude'),
    CODEX_HOME: path.join(home, '.codex'),
    COPILOT_HOME: path.join(home, '.copilot'),
    BROWSERCLAW_DIR: path.join(home, '.browserclaw'),
    TMPDIR: temp,
  }
  for (const dir of Object.values(roots))
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
  return Object.fromEntries(
    Object.entries({ ...process.env, ...overrides, ...roots }).filter(
      ([, value]) => value !== null,
    ),
  )
}

function health(port, signal) {
  // Use loopback HTTP directly, independent of inherited fetch/proxy configuration.
  return new Promise((resolve, reject) => {
    const request = http.get(
      { hostname: '127.0.0.1', port, path: '/json/version', signal },
      (response) => {
        let body = ''
        response.setEncoding('utf8')
        response.on('data', (chunk) => {
          body += chunk
          if (body.length > 64 * 1024)
            request.destroy(new Error('Oversized CDP readiness response'))
        })
        response.on('error', reject)
        response.on('end', () => {
          try {
            const info = JSON.parse(body)
            if (response.statusCode !== 200 || !info.webSocketDebuggerUrl)
              throw new Error('CDP is not ready')
            resolve(info)
          } catch (error) {
            reject(error)
          }
        })
      },
    )
    // A peer sending partial data must not extend the readiness deadline indefinitely.
    const timer = setTimeout(
      () => request.destroy(new Error('CDP readiness timed out')),
      500,
    )
    request.once('close', () => clearTimeout(timer))
    request.on('error', reject)
  })
}

/** Runs only inside a lifetime owner. The corpus supplies its own launch/viewport preparation. */
export async function startOwnedBrowser(
  executable,
  outputDir,
  signal,
  options = {},
) {
  const owner = {}
  let child
  let exit
  let exited
  let closing
  const close = ({ retainState = false } = {}) =>
    (closing ??= (async () => {
      try {
        await cleanupBrowser(owner, outputDir, { exited, retainState })
      } catch (error) {
        // An unproven group must remain untouched, but its handle must not pin a failed owner.
        child?.unref()
        throw error
      }
    })())
  try {
    signal.throwIfAborted()
    owner.profile = fs.mkdtempSync(path.join(outputDir, 'profile-'))
    owner.home = fs.mkdtempSync(path.join(outputDir, 'home-'))
    // Chromium's Linux singleton socket must fit in 108 bytes. Artifact roots can be much longer;
    // keep TMPDIR short and private, and record it for the same proven cleanup as home/profile.
    owner.temp = fs.mkdtempSync('/tmp/browseros-test-')
    const { profile, home, temp } = owner
    const launch = options.launch?.(executable, profile) || {
      executable,
      flags: [...browserFlags(profile), 'about:blank'],
    }
    fs.writeFileSync(
      path.join(outputDir, 'launch.json'),
      JSON.stringify(launch, null, 2),
    )
    let stdout
    let stderr
    try {
      stdout = fs.openSync(
        path.join(outputDir, 'browser.stdout.log'),
        'wx',
        0o600,
      )
      stderr = fs.openSync(
        path.join(outputDir, 'browser.stderr.log'),
        'wx',
        0o600,
      )
      child = spawn(executable, launch.flags, {
        detached: true,
        stdio: ['ignore', stdout, stderr],
        env: browserEnvironment(home, temp, launch.env),
      })
    } finally {
      if (stdout !== undefined) fs.closeSync(stdout)
      if (stderr !== undefined) fs.closeSync(stderr)
    }
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
    // Persist ownership without an await after spawn, so cancellation always has a cleanup record.
    writeReceipt(path.join(outputDir, 'owner.json'), owner)
    if (!child.pid) throw new Error('Browser did not start')
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline) {
      signal.throwIfAborted()
      if (exit)
        throw new Error(`Browser exited at startup: ${JSON.stringify(exit)}`)
      try {
        const [rawPort] = fs
          .readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8')
          .split('\n')
        const port = Number(rawPort)
        if (/^\d+$/.test(rawPort) && port > 0 && port <= 65535) {
          let info
          try {
            info = await health(port, signal)
          } catch {
            signal.throwIfAborted()
            await sleep(50, undefined, { signal })
            continue
          }
          Object.assign(launch, await options.prepare?.(port))
          launch.browser = info.Browser
          fs.writeFileSync(
            path.join(outputDir, 'launch.json'),
            JSON.stringify(launch, null, 2),
          )
          return { ...owner, port, exited, close }
        }
      } catch (error) {
        if (error.code !== 'ENOENT') throw error
      }
      await sleep(50, undefined, { signal })
    }
    throw new Error(
      'Browser did not publish a ready CDP port within 30 seconds',
    )
  } catch (error) {
    try {
      await close()
    } catch (cleanup) {
      throw new AggregateError(
        [error, cleanup],
        'Browser startup and cleanup failed',
      )
    }
    throw error
  }
}
