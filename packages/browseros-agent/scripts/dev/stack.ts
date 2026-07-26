#!/usr/bin/env bun
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
  log('agent', 'Starting agent (WXT + browser)…')

  const agentProc = spawn({
    cmd: ['bun', 'run', '--filter', '@browseros/app', 'dev'],
    cwd: ROOT,
    stdout: 'inherit',
    stderr: 'inherit',
    env: stackEnv,
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
    serverProc.kill()
  }

  process.on('SIGINT', cleanup)
  process.on('SIGTERM', cleanup)

  const [agentCode, serverCode] = await Promise.all([
    agentProc.exited,
    serverProc.exited,
  ])
  if (agentCode !== 0 || serverCode !== 0) {
    console.error(`\nExited: agent=${agentCode} server=${serverCode}`)
    process.exit(1)
  }
}

await main()
