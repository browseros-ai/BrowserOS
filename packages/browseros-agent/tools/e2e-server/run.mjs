/** Stages a Rust server for Chromium to supervise and tracks one private BrowserOS run. */
import { execFileSync, spawn } from 'node:child_process'
import {
  closeSync,
  constants,
  copyFileSync,
  cpSync,
  mkdirSync,
  openSync,
  readFileSync,
} from 'node:fs'
import { connect, createServer } from 'node:net'
import { homedir } from 'node:os'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { writeReceipt } from '../browser-process.mjs'

const workspace = fileURLToPath(new URL('../../', import.meta.url))
const launcher = fileURLToPath(import.meta.url)
const defaultBrowser = path.join(
  homedir(),
  'My Applications/BrowserOS.app/Contents/MacOS/BrowserOS',
)
const providerId = '8b6e8888-4c51-4442-9a49-7a90bba578ea'
const provider = {
  type: 'openai-compatible',
  name: 'BrowserOS E2E Mock',
  modelId: 'browseros-e2e',
  contextWindow: 32768,
  baseUrl: 'http://127.0.0.1:1/v1',
}
const origin = 'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk'

function readJson(file) {
  return JSON.parse(readFileSync(file, 'utf8'))
}

async function until(read, description, timeout = 30_000) {
  const deadline = Date.now() + timeout
  let lastError
  do {
    try {
      const value = await read()
      if (value) return value
    } catch (error) {
      lastError = error
    }
    await sleep(100)
  } while (Date.now() < deadline)
  throw new Error(`Timed out waiting for ${description}`, { cause: lastError })
}

async function freePorts(count) {
  const listeners = Array.from({ length: count }, () => createServer())
  try {
    // Hold all sockets until all ports are chosen so none of them can be assigned twice.
    return await Promise.all(
      listeners.map(
        (server) =>
          new Promise((resolve, reject) => {
            server.once('error', reject)
            server.listen(0, '127.0.0.1', () => resolve(server.address().port))
          }),
      ),
    )
  } finally {
    for (const server of listeners) server.close()
  }
}

async function request(port, route, options) {
  const response = await fetch(`http://127.0.0.1:${port}${route}`, {
    signal: AbortSignal.timeout(1500),
    ...options,
  })
  if (!response.ok) {
    throw new Error(
      `${route}: HTTP ${response.status} ${await response.text()}`,
    )
  }
  return response.json()
}

function processCommand(pid) {
  if (!pid) return ''
  try {
    return execFileSync('ps', ['-ww', '-p', String(pid), '-o', 'command='], {
      encoding: 'utf8',
    }).trim()
  } catch (error) {
    if (error.status === 1) return ''
    throw error
  }
}

function ownsBrowser(run) {
  return processCommand(run.browserPid).endsWith(
    `--user-data-dir=${run.profile} about:blank`,
  )
}

function ownsServer(run, pid) {
  return processCommand(pid) === `${run.binary} --config=${run.config}`
}

function ownsListener(pid, port) {
  try {
    const listeners = execFileSync(
      'lsof',
      ['-nP', '-a', '-p', String(pid), `-iTCP:${port}`, '-sTCP:LISTEN', '-t'],
      { encoding: 'utf8' },
    )
    return listeners.trim() === String(pid)
  } catch (error) {
    if (error.status === 1) return false
    throw error
  }
}

function serverPid(run) {
  // The staged executable and private launch file identify the server even if
  // the browser died between a relaunch and writing its supervisor receipt.
  const table = execFileSync('ps', ['-ww', '-axo', 'pid=,command='], {
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  })
  const marker = `${run.binary} --config=${run.config}`
  for (const line of table.trim().split('\n')) {
    const candidate = line.trim()
    const separator = candidate.indexOf(' ')
    if (candidate.slice(separator + 1).trimStart() === marker) {
      return Number(candidate.slice(0, separator))
    }
  }
  return null
}

async function status(run) {
  const browserRunning = ownsBrowser(run)
  const result = {
    browserPid: run.browserPid,
    browserRunning,
    cdpPort: run.cdpPort,
    bridgePid: ownsBridge(run) ? run.bridge.pid : null,
    serverPid: serverPid(run),
    serverPort: null,
    health: null,
  }
  if (result.serverPid) {
    // Chromium can advance the Rust port on a failed bind. Its launch file is authoritative.
    result.serverPort = readJson(run.config).ports.server
    try {
      if (ownsListener(result.serverPid, result.serverPort)) {
        result.health = await request(result.serverPort, '/system/health')
      }
    } catch (error) {
      result.healthError = error.message
    }
  }
  return result
}

function childExit(child) {
  return new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
}

async function seed(run, port, env) {
  const execution = path.join(run.root, 'seed')
  mkdirSync(execution)
  const config = path.join(execution, 'config.json')
  writeReceipt(config, {
    ports: { cdp: run.cdpPort, server: port },
    directories: { resources: run.resources, execution },
  })
  // Initialize the database through the product's migrations and write through its provider API.
  // Wait for the seed process to close SQLite before the browser starts its own server.
  const child = spawn(run.binary, [`--config=${config}`], {
    env: { ...env, BROWSEROS_BROWSER: 'off' },
    stdio: ['ignore', 'inherit', 'inherit'],
  })
  const exited = childExit(child)
  try {
    await Promise.race([
      until(
        () => ownsListener(child.pid, port) && request(port, '/system/health'),
        'seed server health',
      ),
      exited.then((result) => {
        throw new Error(`Seed server exited: ${JSON.stringify(result)}`)
      }),
    ])
    const put = (route, body) =>
      request(port, route, {
        method: 'PUT',
        headers: { Origin: origin, 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
    await put(`/providers/${providerId}`, provider)
    await put('/providers/default', { providerId })
    await request(port, '/system/shutdown', { method: 'POST' })
    const result = await exited
    if (result.code !== 0)
      throw new Error(`Seed server exited: ${JSON.stringify(result)}`)
  } finally {
    if (child.exitCode === null && child.signalCode === null && child.pid) {
      child.kill('SIGTERM')
      await exited
    }
  }
}

function build(target) {
  const env = {
    ...process.env,
    PATH: `${path.join(homedir(), '.cargo/bin')}:${process.env.PATH}`,
    RUSTUP_TOOLCHAIN: '1.98.0',
    CARGO_TARGET_DIR: target,
    CARGO_BUILD_BUILD_DIR: path.join(target, 'build-private'),
    CARGO_PROFILE_DEV_DEBUG: '0',
    CARGO_INCREMENTAL: '0',
  }
  const options = { cwd: workspace, stdio: 'inherit', env }
  execFileSync(
    'cargo',
    [
      'build',
      '-j',
      '4',
      '--locked',
      '-p',
      'browseros-server',
      '--bin',
      'browseros_server',
    ],
    options,
  )
  execFileSync(
    'npx',
    ['--yes', 'bun@1.4.2', 'install', '--frozen-lockfile'],
    options,
  )
  execFileSync('npx', ['--yes', 'bun@1.4.2', 'run', 'build:agent:dev'], options)
}

async function closeBrowser(run) {
  // Browser.close runs Chromium's shutdown path, which must stop its own server.
  if (!ownsListener(run.browserPid, run.cdpPort))
    throw new Error('Browser PID does not own the debugging port')
  const info = await request(run.cdpPort, '/json/version')
  const endpoint = new URL(info.webSocketDebuggerUrl)
  if (
    endpoint.hostname !== '127.0.0.1' ||
    Number(endpoint.port) !== run.cdpPort
  ) {
    throw new Error('Unexpected browser debugging endpoint')
  }
  const socket = new WebSocket(endpoint)
  try {
    const signal = AbortSignal.timeout(1500)
    await new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => reject(signal.reason), {
        once: true,
      })
      socket.addEventListener('open', resolve, { once: true })
      socket.addEventListener('error', reject, { once: true })
    })
    socket.send(JSON.stringify({ id: 1, method: 'Browser.close' }))
    await until(() => !ownsBrowser(run), 'browser exit', 10_000)
  } finally {
    socket.close()
  }
}

async function stop(run) {
  let pid = serverPid(run)
  let browserSignal
  if (ownsBrowser(run)) {
    try {
      await closeBrowser(run)
    } catch (error) {
      if (ownsBrowser(run)) {
        process.kill(run.browserPid, 'SIGTERM')
        browserSignal = 'SIGTERM'
        console.error(`Graceful browser close failed: ${error.message}`)
        await until(
          () => !ownsBrowser(run),
          'browser exit after SIGTERM',
          10_000,
        )
      }
    }
  }
  // Chromium can replace its child between the first process scan and close.
  pid = serverPid(run) ?? pid
  let serverSignal
  try {
    await until(() => !ownsServer(run, pid), 'browser-owned server exit', 6_000)
  } catch (error) {
    if (ownsServer(run, pid)) {
      process.kill(pid, 'SIGTERM')
      serverSignal = 'SIGTERM'
      console.error(error.message)
      await until(
        () => !ownsServer(run, pid),
        'server exit after SIGTERM',
        6_000,
      )
    }
  }
  return {
    browserPid: run.browserPid,
    serverPid: pid,
    browserSignal,
    serverSignal,
  }
}

function stageResources(root, target, browser) {
  const resources = path.join(root, 'resources')
  const binary = path.join(resources, 'bin/browseros_server')
  const bun = path.join(resources, 'bin/third_party/bun')
  mkdirSync(path.dirname(bun), { recursive: true })
  copyFileSync(
    path.join(target, 'debug/browseros_server'),
    binary,
    constants.COPYFILE_FICLONE,
  )
  copyFileSync(
    path.resolve(
      path.dirname(browser),
      '../Resources/BrowserOSServer/default/resources/bin/third_party/bun',
    ),
    bun,
    constants.COPYFILE_FICLONE,
  )
  const extension = path.join(root, 'extension')
  cpSync(path.join(workspace, 'apps/app/dist/chrome-mv3-dev'), extension, {
    recursive: true,
  })
  return { resources, binary, extension }
}

function stageMock(root, script) {
  const mockScript = path.join(root, 'mock.json')
  if (script) {
    copyFileSync(path.resolve(script), mockScript)
  } else {
    writeReceipt(mockScript, {
      steps: [{ text: 'BrowserOS E2E mock response.' }],
    })
  }
  return mockScript
}

function runEnvironment(root, mockScript) {
  const home = path.join(root, 'home')
  mkdirSync(home, { mode: 0o700 })
  // The browser's profile and home are private. Chromium passes the state and model
  // environment through to each replacement Rust server.
  const { BROWSEROS_BROWSER: _browserMode, ...env } = process.env
  return {
    ...env,
    HOME: home,
    CFFIXED_USER_HOME: home,
    BROWSEROS_DIR: path.join(root, 'state'),
    BROWSERCLAW_DIR: path.join(home, '.browserclaw'),
    CODEX_HOME: path.join(home, '.codex'),
    CLAUDE_CONFIG_DIR: path.join(home, '.claude'),
    BROWSEROS_USE_MOCK_LLM: 'true',
    BROWSEROS_MOCK_LLM_SCRIPT: mockScript,
  }
}

function browserFlags(run, proxyPort) {
  return [
    '--no-first-run',
    '--no-default-browser-check',
    '--use-mock-keychain',
    '--disable-browseros-extensions',
    '--disable-browseros-server-updater',
    `--browseros-server-resources-dir=${run.resources}`,
    `--browseros-cdp-port=${run.bridge.port}`,
    '--remote-debugging-port=0',
    `--browseros-proxy-port=${proxyPort}`,
    `--load-extension=${run.extension}`,
    `--user-data-dir=${run.profile}`,
    'about:blank',
  ]
}

function spawnLogged(executable, args, env, logPath) {
  const log = openSync(logPath, 'w', 0o600)
  try {
    return spawn(executable, args, {
      detached: true,
      env,
      stdio: ['ignore', log, log],
    })
  } finally {
    closeSync(log)
  }
}

function cdpPort(run) {
  const file = path.join(run.profile, 'DevToolsActivePort')
  return Number(readFileSync(file, 'utf8').split('\n')[0])
}

async function startBrowser(run, file, browser, flags, env) {
  const child = spawnLogged(
    browser,
    flags,
    env,
    path.join(run.root, 'browser.log'),
  )
  const exited = childExit(child)
  run.browserPid = child.pid
  writeReceipt(file, run)
  try {
    run.cdpPort = await Promise.race([
      until(() => {
        const port = cdpPort(run)
        return ownsBrowser(run) && ownsListener(run.browserPid, port) && port
      }, 'private browser CDP'),
      exited.then((result) => {
        throw new Error(`Browser exited: ${JSON.stringify(result)}`)
      }),
    ])
    writeReceipt(file, run)
    return await until(async () => {
      const result = await status(run)
      return result.health?.cdpConnected && result
    }, 'Chromium to start the staged Rust server and connect its engine')
  } catch (error) {
    await stop(run)
    throw error
  } finally {
    child.unref()
  }
}

// Chromium binds an ephemeral CDP listener itself. Its server receives the
// bridge's already-bound port, so even an early engine connect or restart
// cannot land on another browser's debugger.
function serveBridge(root) {
  const file = path.join(root, 'run.json')
  const server = createServer((client) => {
    let run
    let port
    try {
      run = readJson(file)
      port = cdpPort(run)
      if (!ownsBrowser(run) || !ownsListener(run.browserPid, port))
        return client.destroy()
    } catch {
      return client.destroy()
    }
    const browser = connect(port, '127.0.0.1')
    browser.once('connect', () => {
      if (ownsBrowser(run) && ownsListener(run.browserPid, port)) {
        client.pipe(browser).pipe(client)
      } else {
        client.destroy()
        browser.destroy()
      }
    })
    client.on('error', () => browser.destroy())
    browser.on('error', () => client.destroy())
    client.on('close', () => browser.destroy())
    browser.on('close', () => client.destroy())
  })
  server.listen(0, '127.0.0.1', () => {
    writeReceipt(path.join(root, 'bridge.json'), {
      pid: process.pid,
      port: server.address().port,
    })
  })
  const startup = Date.now()
  setInterval(() => {
    try {
      if (!ownsBrowser(readJson(file))) process.exit(0)
    } catch (error) {
      if (error.code !== 'ENOENT' || Date.now() - startup > 30_000)
        process.exit(0)
    }
  }, 500).unref()
}

async function startBridge(run) {
  const child = spawnLogged(
    process.execPath,
    [launcher, 'cdp-bridge', run.root],
    process.env,
    path.join(run.root, 'bridge.log'),
  )
  child.on('error', (error) => console.error(error))
  child.unref()
  try {
    return await until(
      () => {
        const bridge = readJson(path.join(run.root, 'bridge.json'))
        return (
          bridge.pid === child.pid &&
          ownsListener(bridge.pid, bridge.port) &&
          bridge
        )
      },
      'private CDP bridge',
      3_000,
    )
  } catch (error) {
    child.kill()
    throw error
  }
}

function ownsBridge(run) {
  const pid = run.bridge?.pid
  return (
    pid && processCommand(pid).endsWith(` ${launcher} cdp-bridge ${run.root}`)
  )
}

async function stopBridge(run) {
  if (!ownsBridge(run)) return
  process.kill(run.bridge.pid, 'SIGTERM')
  await until(() => !ownsBridge(run), 'CDP bridge exit', 3000)
}

async function up(root, file, script) {
  mkdirSync(root, { mode: 0o700 })
  const target = path.join(workspace, 'target')
  build(target)
  const browser = path.resolve(
    process.env.BROWSEROS_E2E_BROWSER || defaultBrowser,
  )
  const profile = path.join(root, 'profile')
  const run = {
    root,
    profile,
    ...stageResources(root, target, browser),
    config: path.join(profile, '.browseros/config.json'),
  }
  const mockScript = stageMock(root, script)
  const env = runEnvironment(root, mockScript)
  const [proxyPort, seedPort] = await freePorts(2)
  await seed({ ...run, cdpPort: seedPort }, seedPort, env)
  run.bridge = await startBridge(run)
  try {
    const flags = browserFlags(run, proxyPort)
    writeReceipt(path.join(root, 'launch.json'), { browser, flags, mockScript })
    return await startBrowser(run, file, browser, flags, env)
  } catch (error) {
    await stopBridge(run)
    throw error
  }
}

async function down(run) {
  try {
    return await stop(run)
  } finally {
    await stopBridge(run)
  }
}

const [command, directory, script] = process.argv.slice(2)
if (command === 'cdp-bridge' && directory) {
  serveBridge(path.resolve(directory))
} else if (!['up', 'status', 'down'].includes(command) || !directory) {
  console.error(
    'Usage: node tools/e2e-server/run.mjs <up|status|down> <run-dir> [mock-script.json]',
  )
  process.exitCode = 2
} else {
  try {
    const root = path.resolve(directory)
    const file = path.join(root, 'run.json')
    const result =
      command === 'up'
        ? await up(root, file, script)
        : command === 'status'
          ? await status(readJson(file))
          : await down(readJson(file))
    console.log(JSON.stringify(result, null, 2))
  } catch (error) {
    console.error(error)
    process.exitCode = 1
  }
}
