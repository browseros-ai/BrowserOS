/** Owns every child, profile and socket for one run. A child handle (never a global process-name
 * kill) is the cleanup authority. The native oracle explicitly drops the image's hosted default. */
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import {
  closeSync,
  cpSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { listen } from './fixture-server.mjs'
import { sha } from './upstream.mjs'
import { sizePages } from './viewport-rig.mjs'

const delay = (ms) => new Promise((r) => setTimeout(r, ms))
export async function start({
  ports,
  browser,
  neoBinary,
  assetsDir,
  runDir,
  gitSha,
  keep = false,
  signal,
}) {
  const root = mkdtempSync(join(tmpdir(), 'neo-cf-')),
    children = []
  const viewportRigs = {}
  let fixture, stopping
  function stop() {
    stopping ??= cleanup()
    return stopping
  }
  async function cleanup() {
    const snapshots = []
    for (const [side, rig] of Object.entries(viewportRigs)) {
      const health = rig.health()
      await rig.stop()
      snapshots.push([side, { health, pages: rig.pages() }])
    }
    for (const child of [...children].reverse()) {
      if (
        child.launchError ||
        !child.pid ||
        child.exitCode !== null ||
        child.signalCode !== null
      )
        continue
      child.kill('SIGTERM')
      await Promise.race([once(child, 'exit'), delay(5000)])
      if (child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL')
        await once(child, 'exit')
      }
    }
    await fixture?.close()
    try {
      if (keep)
        cpSync(root, join(runDir, 'state'), {
          recursive: true,
          filter: (path) => {
            const stat = lstatSync(path)
            return (
              path !== join(root, 'token') &&
              (stat.isFile() || stat.isDirectory() || stat.isSymbolicLink())
            )
          },
        })
    } finally {
      // Chromium helpers may finish a profile write just after the browser process exits.
      rmSync(root, {
        recursive: true,
        force: true,
        maxRetries: 5,
        retryDelay: 100,
      })
    }
    for (const [side, snapshot] of snapshots) {
      const file = join(runDir, `viewport-rig-${side}.json`)
      // A leak restart replaces the connections but retains evidence from earlier browsers.
      const runs = JSON.parse(
        readFileSync(file, { encoding: 'utf8', flag: 'a+' }) || '[]',
      )
      writeFileSync(file, `${JSON.stringify([...runs, snapshot], null, 2)}\n`)
    }
  }
  function launch(name, exe, args, env) {
    const fd = openSync(join(runDir, 'logs', `${name}.log`), 'a')
    const child = spawn(exe, args, { env, stdio: ['ignore', fd, fd] })
    closeSync(fd)
    children.push(child)
    writeFileSync(
      join(runDir, 'processes.json'),
      JSON.stringify(
        children.map((child) => ({
          pid: child.pid,
          spawnargs: child.spawnargs,
        })),
        null,
        2,
      ),
    )
    child.on('error', (error) => {
      child.launchError = error
    })
    return child
  }
  async function ready(url, child) {
    const end = Date.now() + 60000
    while (Date.now() < end) {
      signal?.throwIfAborted()
      if (child.launchError || child.exitCode !== null)
        throw (
          child.launchError ||
          new Error(`child exited ${child.exitCode}: ${url}`)
        )
      try {
        const r = await fetch(url, { signal: AbortSignal.timeout(1000) })
        if (r.ok) return await r.json()
      } catch {}
      await delay(100)
    }
    throw new Error(`readiness deadline: ${url}`)
  }
  try {
    signal?.throwIfAborted()
    mkdirSync(join(runDir, 'logs'), { recursive: true })
    for (let port = ports; port < ports + 4; port++) {
      const probe = net.createServer()
      await new Promise((ok, fail) => {
        probe.once('error', fail)
        probe.listen(port, '0.0.0.0', ok)
      })
      await new Promise((r) => probe.close(r))
    }
    fixture = await listen(ports + 3, assetsDir)
    const flags = [
      '--headless=new',
      '--window-size=1280,720',
      '--disable-browseros-server',
      '--disable-browseros-server-updater',
      '--disable-browseros-extensions',
      '--host-resolver-rules=MAP hang.localtest.me 127.0.0.1',
      '--no-first-run',
      '--no-default-browser-check',
    ]
    if (process.platform === 'linux') flags.push('--no-sandbox')
    const nativeEnv = { ...process.env }
    delete nativeEnv.BROWSERCLAW_HOSTED_MODE
    const native = launch(
      'browser-native',
      browser,
      [
        ...flags,
        `--user-data-dir=${root}/native`,
        `--remote-debugging-port=${ports}`,
        'about:blank',
      ],
      nativeEnv,
    )
    const hosted = launch(
      'browser-neo',
      browser,
      [
        ...flags,
        `--user-data-dir=${root}/neo`,
        `--remote-debugging-port=${ports + 1}`,
        'about:blank',
      ],
      { ...nativeEnv, BROWSERCLAW_HOSTED_MODE: '1' },
    )
    const p = await ready(`http://127.0.0.1:${ports}/json/version`, native)
    const n = await ready(`http://127.0.0.1:${ports + 1}/json/version`, hosted)
    if (p.Browser !== n.Browser) throw new Error('browser version mismatch')
    const nativeCdp = `http://127.0.0.1:${ports}`,
      neoCdp = `http://127.0.0.1:${ports + 1}`
    // Both initial targets must be sized before the server can hand a page to a script.
    for (const [side, cdpHttp] of [
      ['native', nativeCdp],
      ['neo', neoCdp],
    ]) {
      signal?.throwIfAborted()
      const rig = await sizePages({ cdpHttp })
      viewportRigs[side] = rig
      const failed = rig.pages().find((entry) => entry.error)
      if (rig.health() === 'lost' || failed)
        throw new Error(
          `viewport rig (${side}): ${failed?.error || 'connection lost'}`,
        )
    }
    const hostToken = randomBytes(32).toString('hex'),
      hostSocket = join(root, 'host.sock'),
      tokenFile = join(root, 'token')
    writeFileSync(tokenFile, hostToken, { mode: 0o600 })
    const config = join(root, 'launch.json')
    writeFileSync(
      config,
      JSON.stringify({
        ports: { server: ports + 2, cdp: ports + 1 },
        directories: {
          resources: 'r',
          execution: join(root, 'neo/.browseros'),
        },
      }),
    )
    const neo = launch('neo', neoBinary, ['--config', config], {
      HOME: join(root, 'home'),
      BROWSERCLAW_DIR: join(root, 'state'),
      BROWSERCLAW_HOSTED_MODE: '1',
      CLAW_HOST_TOKEN_FILE: tokenFile,
      CLAW_HOST_SOCKET: hostSocket,
      CLAW_XVNC_ADDR: '127.0.0.1:9',
    })
    await ready(`http://127.0.0.1:${ports + 2}/system/ready`, neo)
    return {
      nativeCdp,
      neoCdp,
      viewportRigs,
      neoMcp: `http://127.0.0.1:${ports + 2}/mcp`,
      hostToken,
      hostSocket,
      server: fixture,
      browserVersion: p.Browser,
      binarySha256: sha(readFileSync(neoBinary)),
      browserSha256: sha(readFileSync(browser)),
      gitSha,
      flags,
      ports,
      stop,
    }
  } catch (error) {
    await stop()
    throw error
  }
}
