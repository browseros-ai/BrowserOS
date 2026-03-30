/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Low-level MCP server process management.
 * Use setup.ts:ensureBrowserOS() for the full test environment.
 */
import { type ChildProcess, spawn } from 'node:child_process'
import { dirname, resolve } from 'node:path'

const SERVER_ENTRYPOINT_PATH = resolve(
  dirname(import.meta.path),
  '../../src/index.ts',
)

export interface ServerConfig {
  cdpPort: number
  serverPort: number
  extensionPort: number
}

interface ServerState {
  process: ChildProcess
  config: ServerConfig
}

let serverState: ServerState | null = null

export async function isServerRunning(port: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/health`, {
      signal: AbortSignal.timeout(1000),
    })
    return response.ok
  } catch {
    return false
  }
}

async function waitForHealth(port: number, maxAttempts = 30): Promise<void> {
  for (let i = 0; i < maxAttempts; i++) {
    if (await isServerRunning(port)) {
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`Server failed to start on port ${port} within timeout`)
}

export function getServerState(): ServerState | null {
  return serverState
}

export async function spawnServer(config: ServerConfig): Promise<ServerState> {
  if (
    serverState &&
    JSON.stringify(serverState.config) === JSON.stringify(config)
  ) {
    if (await isServerRunning(config.serverPort)) {
      console.log(`Reusing existing server on port ${config.serverPort}`)
      return serverState
    }
  }

  if (serverState) {
    console.log('Config changed, cleaning up existing server...')
    await killServer()
  }

  console.log(`Starting BrowserOS Server on port ${config.serverPort}...`)
  const process = spawn(
    'bun',
    [
      SERVER_ENTRYPOINT_PATH,
      '--cdp-port',
      config.cdpPort.toString(),
      '--server-port',
      config.serverPort.toString(),
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...globalThis.process.env, NODE_ENV: 'test' },
    },
  )

  process.stdout?.on('data', (_data) => {
    // Uncomment for debugging
    // console.log(`[SERVER] ${_data.toString().trim()}`)
  })

  process.stderr?.on('data', (_data) => {
    // Uncomment for debugging
    // console.error(`[SERVER] ${_data.toString().trim()}`)
  })

  process.on('error', (error) => {
    console.error('Failed to start server:', error)
  })

  console.log('Waiting for server to be ready...')
  await waitForHealth(config.serverPort)
  console.log('Server is ready')

  serverState = { process, config }
  return serverState
}

export async function killServer(): Promise<void> {
  if (!serverState) {
    return
  }

  console.log('Shutting down server...')
  serverState.process.kill('SIGTERM')

  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      serverState?.process.kill('SIGKILL')
      resolve()
    }, 5000)

    serverState?.process.on('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
  })

  console.log('Server stopped')
  serverState = null
}
