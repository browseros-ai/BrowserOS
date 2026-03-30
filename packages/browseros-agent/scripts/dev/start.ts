#!/usr/bin/env bun
import { mkdtempSync } from 'node:fs'
import { createServer as createNetServer } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn, spawnSync } from 'bun'
import pc from 'picocolors'

type Ports = { cdp: number; server: number; extension: number }
type Mode = 'watch' | 'manual'

const MONOREPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../..')
const BROWSEROS_BINARY = '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS'
const AGENT_EXT_DIR = join(MONOREPO_ROOT, 'apps/agent/dist/chrome-mv3-dev')
let USER_DATA_DIR = '/tmp/browseros-dev'

const TAG = {
  server: pc.cyan,
  agent: pc.magenta,
  browser: pc.blue,
  build: pc.yellow,
  info: pc.green,
}

function log(prefix: keyof typeof TAG, message: string): void {
  console.log(`${TAG[prefix](`[${prefix}]`)} ${message}`)
}

function printHelp(): void {
  console.log(`
Usage: bun scripts/dev/start.ts <mode> [options]

Modes:
  --watch     Hot reload mode — wxt launches BrowserOS with HMR, server starts after CDP is ready
  --manual    Manual mode — launches BrowserOS directly, builds both extensions, no HMR

Options:
  --new       Find available ports instead of killing processes on .env ports
  --help      Show this help message
`)
}

function parseArgs(): { mode: Mode; isNew: boolean; help: boolean } {
  const argv = process.argv
  const isWatch = argv.includes('--watch')
  const isManual = argv.includes('--manual')

  if (!isWatch && !isManual) {
    console.error('Error: specify --watch or --manual')
    printHelp()
    process.exit(1)
  }

  return {
    mode: isWatch ? 'watch' : 'manual',
    isNew: argv.includes('--new'),
    help: argv.includes('--help') || argv.includes('-h'),
  }
}

function loadEnvPorts(): Ports {
  const env = process.env
  return {
    cdp: Number(env.BROWSEROS_CDP_PORT) || 9005,
    server: Number(env.BROWSEROS_SERVER_PORT) || 9105,
    extension: Number(env.BROWSEROS_EXTENSION_PORT) || 9305,
  }
}

function killPort(port: number): void {
  spawnSync({
    cmd: ['sh', '-c', `lsof -ti:${port} | xargs kill -9 2>/dev/null || true`],
  })
}

function killPorts(ports: Ports): void {
  log(
    'info',
    `Killing processes on ports ${ports.cdp}, ${ports.server}, ${ports.extension}...`,
  )
  killPort(ports.cdp)
  killPort(ports.server)
  killPort(ports.extension)
  log('info', 'Ports cleared')
}

function isPortAvailable(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = createNetServer()
    server.once('error', () => resolve(false))
    server.once('listening', () => {
      server.close()
      resolve(true)
    })
    server.listen(port, '127.0.0.1')
  })
}

async function findAvailablePort(startPort: number): Promise<number> {
  let port = startPort
  while (!(await isPortAvailable(port))) {
    port++
  }
  return port
}

async function findAvailablePorts(base: Ports): Promise<Ports> {
  return {
    cdp: await findAvailablePort(base.cdp),
    server: await findAvailablePort(base.server),
    extension: await findAvailablePort(base.extension),
  }
}

async function waitForCdp(cdpPort: number, maxAttempts = 60): Promise<boolean> {
  for (let i = 0; i < maxAttempts; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${cdpPort}/json/version`, {
        signal: AbortSignal.timeout(1000),
      })
      if (response.ok) return true
    } catch {
      // not ready
    }
    await new Promise((r) => setTimeout(r, 500))
  }
  return false
}

function buildExtension(name: string, script: string): void {
  log('build', `Building ${name}...`)
  const result = spawnSync({
    cmd: ['bun', 'run', script],
    cwd: MONOREPO_ROOT,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  if (result.exitCode !== 0) {
    log('build', `${name} build failed`)
    process.exit(1)
  }
  log('build', `${name} built`)
}

function createEnv(ports: Ports): NodeJS.ProcessEnv {
  return {
    ...process.env,
    NODE_ENV: 'development',
    BROWSEROS_CDP_PORT: String(ports.cdp),
    BROWSEROS_SERVER_PORT: String(ports.server),
    BROWSEROS_EXTENSION_PORT: String(ports.extension),
    VITE_BROWSEROS_SERVER_PORT: String(ports.server),
  }
}

function prefixLines(prefix: keyof typeof TAG, text: string): string {
  return text
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => `${TAG[prefix](`[${prefix}]`)} ${line}`)
    .join('\n')
}

async function streamOutput(
  stream: ReadableStream<Uint8Array>,
  prefix: keyof typeof TAG,
): Promise<void> {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    const text = decoder.decode(value)
    console.log(prefixLines(prefix, text))
  }
}

function startWatchMode(env: NodeJS.ProcessEnv): ReturnType<typeof spawn> {
  log('agent', 'Starting agent with HMR (wxt)...\n')
  return spawn({
    cmd: ['bun', 'run', '--filter', '@browseros/agent', 'dev'],
    cwd: MONOREPO_ROOT,
    stdout: 'pipe',
    stderr: 'pipe',
    env,
  })
}

function startManualBrowser(ports: Ports): ReturnType<typeof spawn> {
  const args = [
    '--no-first-run',
    '--no-default-browser-check',
    '--use-mock-keychain',
    '--show-component-extension-options',
    '--disable-browseros-server',
    '--disable-browseros-extensions',
    // TODO: replace with --browseros-cdp-port once we fix the browseros bug
    `--remote-debugging-port=${ports.cdp}`,
    `--browseros-mcp-port=${ports.server}`,
    `--browseros-extension-port=${ports.extension}`,
    `--user-data-dir=${USER_DATA_DIR}`,
    `--load-extension=${AGENT_EXT_DIR}`,
    'chrome://newtab',
  ]

  log('browser', `Launching BrowserOS...`)
  log('browser', `  Profile: ${USER_DATA_DIR}`)
  log('browser', `  CDP: http://127.0.0.1:${ports.cdp}`)

  return spawn({
    cmd: [BROWSEROS_BINARY, ...args],
    stdout: 'pipe',
    stderr: 'pipe',
  })
}

function startServer(env: NodeJS.ProcessEnv): ReturnType<typeof spawn> {
  log('server', 'Starting server...\n')
  return spawn({
    cmd: ['bun', 'run', '--filter', '@browseros/server', 'start'],
    cwd: MONOREPO_ROOT,
    stdout: 'pipe',
    stderr: 'pipe',
    env,
  })
}

async function main() {
  const args = parseArgs()

  if (args.help) {
    printHelp()
    process.exit(0)
  }

  let ports = loadEnvPorts()

  if (args.isNew) {
    log('info', 'Finding available ports...')
    ports = await findAvailablePorts(ports)
    log(
      'info',
      `Ports: CDP=${ports.cdp} Server=${ports.server} Extension=${ports.extension}`,
    )
    USER_DATA_DIR = mkdtempSync(join(tmpdir(), 'browseros-dev-'))
    log('info', `Created fresh profile: ${USER_DATA_DIR}`)
  } else {
    killPorts(ports)
  }

  console.log()
  log('info', `Mode: ${args.mode}`)
  log(
    'info',
    `Ports: CDP=${ports.cdp} Server=${ports.server} Extension=${ports.extension}`,
  )
  log('info', `Profile: ${USER_DATA_DIR}`)
  console.log()

  const env = createEnv(ports)
  const procs: ReturnType<typeof spawn>[] = []
  const streams: Promise<void>[] = []

  if (args.mode === 'manual') {
    buildExtension('agent', 'build:agent:dev')

    const browserProc = startManualBrowser(ports)
    procs.push(browserProc)
    streams.push(
      streamOutput(browserProc.stdout, 'browser'),
      streamOutput(browserProc.stderr, 'browser'),
    )
  } else {
    const agentProc = startWatchMode(env)
    procs.push(agentProc)
    streams.push(
      streamOutput(agentProc.stdout, 'agent'),
      streamOutput(agentProc.stderr, 'agent'),
    )
  }

  log('server', 'Waiting for CDP...')
  const cdpReady = await waitForCdp(ports.cdp)
  if (cdpReady) {
    log('server', 'CDP ready')
  } else {
    log('server', 'Warning: CDP not available, starting server anyway')
  }

  const serverProc = startServer(env)
  procs.push(serverProc)
  streams.push(
    streamOutput(serverProc.stdout, 'server'),
    streamOutput(serverProc.stderr, 'server'),
  )

  const cleanup = () => {
    for (const proc of procs) proc.kill()
  }

  process.on('SIGINT', cleanup)
  process.on('SIGTERM', cleanup)

  await Promise.all(streams)

  const exitCodes = await Promise.all(procs.map((p) => p.exited))
  const failed = exitCodes.some((code) => code !== 0)
  if (failed) {
    console.error(`\nProcesses exited with codes: ${exitCodes.join(', ')}`)
    process.exit(1)
  }
}

main()
