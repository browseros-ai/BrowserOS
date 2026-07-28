#!/usr/bin/env bun
import { createHash } from 'node:crypto'
import { mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'bun'
import { config } from 'dotenv'
import pc from 'picocolors'

// WXT + browser, then MCP server; ports from apps/agent/.env.development

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '../..')
const AGENT_ENV = join(ROOT, 'apps/app/.env.development')

const TAG = {
  agent: pc.magenta,
  server: pc.cyan,
  info: pc.green,
} as const

function log(tag: keyof typeof TAG, message: string): void {
  console.log(`${TAG[tag](`[${tag}]`)} ${message}`)
}

function loadStackEnv(): NodeJS.ProcessEnv {
  const { parsed, error } = config({ path: AGENT_ENV })
  if (error) {
    console.warn(pc.yellow(`[info] ${AGENT_ENV}: ${error.message}`))
  }
  return { ...process.env, ...parsed } as NodeJS.ProcessEnv
}

async function waitForCdp(port: number, maxAttempts = 120): Promise<boolean> {
  for (let i = 0; i < maxAttempts; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`, {
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

async function main(): Promise<void> {
  const stackEnv = loadStackEnv()
  const cdpPort = Number(stackEnv.BROWSEROS_CDP_PORT) || 9333
  const serverPort = Number(stackEnv.BROWSEROS_SERVER_PORT) || 9111

  stackEnv.VITE_BROWSEROS_SERVER_PORT = String(serverPort)

  log('info', `Loaded ${AGENT_ENV}`)
  log(
    'info',
    `CDP=${cdpPort}  HTTP server=${serverPort}  (set BROWSEROS_* in that file to change)`,
  )
  log('agent', 'Starting extension builder (WXT)…')

  const agentProc = spawn({
    cmd: ['bun', 'run', '--filter', '@browseros/app', 'dev'],
    cwd: ROOT,
    stdout: 'pipe',
    stderr: 'inherit',
    env: stackEnv,
  })

  log('info', 'Waiting for extension to compile…')
  const reader = agentProc.stdout.getReader()
  const decoder = new TextDecoder()
  
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    const chunk = decoder.decode(value, { stream: true })
    process.stdout.write(chunk)
    if (chunk.includes('Built extension') || chunk.includes('Load "dist')) {
      break
    }
  }

  // Resume forwarding stdout asynchronously
  ;(async () => {
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        process.stdout.write(decoder.decode(value, { stream: true }))
      }
    } catch {}
  })()

  log('info', 'Extension compiled!')

  const browserBinary = stackEnv.BROWSEROS_BINARY || 'chrome'
  const label = 'shimmy'
  const key = createHash('sha256').update(ROOT).digest('hex').slice(0, 8)
  const browserProfile = join(tmpdir(), `browseros-dev-${label}-${key}-custom`)
  mkdirSync(browserProfile, { recursive: true })

  const browserArgs = [
    `--load-unpacked=${join(ROOT, 'apps/app/dist/chrome-mv3-dev')}`,
    `--remote-debugging-port=${cdpPort}`,
    `--user-data-dir=${browserProfile}`,
    '--app=chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk/app.html',
    '--disable-features=EdgeStartupBoost',
    '--no-first-run',
    '--no-default-browser-check',
    '--use-mock-keychain',
    '--show-component-extension-options',
    '--disable-browseros-server',
    '--disable-browseros-server-updater',
    '--disable-browseros-extensions',
    '--browseros-dock-icon=dev',
    '--browseros-product=browseros',
    `--browseros-mcp-port=${serverPort}`,
    `--browseros-server-port=${serverPort}`,
    `--browseros-proxy-port=${serverPort}`,
  ]

  if (stackEnv.BROWSEROS_EXTENSION_PORT) {
    browserArgs.push(`--browseros-extension-port=${stackEnv.BROWSEROS_EXTENSION_PORT}`)
  }

  log('info', `Starting browser: ${browserBinary} (standalone app mode)`)
  const browserProc = spawn({
    cmd: [browserBinary, ...browserArgs],
    stdout: 'inherit',
    stderr: 'inherit',
  })

  log('server', 'Waiting for CDP…')
  const cdpReady = await waitForCdp(cdpPort)
  if (cdpReady) {
    log('server', 'CDP ready')
  } else {
    log('server', pc.yellow('CDP not responding; starting server anyway'))
  }

  log('server', 'Starting MCP server…')
  const serverProc = spawn({
    cmd: ['bun', 'run', '--filter', '@browseros/server', 'start'],
    cwd: ROOT,
    stdout: 'inherit',
    stderr: 'inherit',
    env: stackEnv,
  })

  const cleanup = (): void => {
    agentProc.kill()
    browserProc.kill()
    serverProc.kill()
  }

  process.on('SIGINT', cleanup)
  process.on('SIGTERM', cleanup)

  const [agentCode, browserCode, serverCode] = await Promise.all([
    agentProc.exited,
    browserProc.exited,
    serverProc.exited,
  ])
  cleanup()
  if (agentCode !== 0 || browserCode !== 0 || serverCode !== 0) {
    console.error(`\nExited: agent=${agentCode} browser=${browserCode} server=${serverCode}`)
    process.exit(1)
  }
}

await main()
