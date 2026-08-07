/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Low-level MCP server process management.
 * Use setup.ts:ensureBrowserOS() for the full test environment.
 */
import { type ChildProcess, spawn } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

const SERVER_ENTRYPOINT_PATH = resolve(
  dirname(import.meta.path),
  '../../src/index.ts',
)
const MONOREPO_ROOT = resolve(dirname(import.meta.path), '../../../..')

export interface ServerConfig {
  cdpPort: number
  serverPort: number
  extensionPort: number
}

export interface ServerState {
  process: ChildProcess
  config: ServerConfig
  configDir: string
}

let serverState: ServerState | null = null

function appendBufferedLog(buffer: string[], chunk: Buffer | string): void {
  const text = chunk.toString()
  const lines = text
    .split('\n')
    .map((line) => line.trimEnd())
    .filter((line) => line.length > 0)
  if (lines.length === 0) {
    return
  }
  buffer.push(...lines)
  const overflow = buffer.length - 40
  if (overflow > 0) {
    buffer.splice(0, overflow)
  }
}

function formatStartupFailure(
  process: ChildProcess,
  port: number,
  stdoutBuffer: string[],
  stderrBuffer: string[],
  reason: string,
): Error {
  const details: string[] = [reason]

  if (process.exitCode !== null) {
    details.push(`exit code: ${process.exitCode}`)
  }
  if (process.signalCode) {
    details.push(`signal: ${process.signalCode}`)
  }

  if (stderrBuffer.length > 0) {
    details.push(`stderr:\n${stderrBuffer.join('\n')}`)
  } else if (stdoutBuffer.length > 0) {
    details.push(`stdout:\n${stdoutBuffer.join('\n')}`)
  }

  return new Error(
    `Server failed to start on port ${port}. ${details.join('\n\n')}`,
  )
}

export async function isServerRunning(port: number): Promise<boolean> {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/system/health`, {
      signal: AbortSignal.timeout(1000),
    })
    return response.ok
  } catch {
    return false
  }
}

async function waitForHealth(
  process: ChildProcess,
  port: number,
  stdoutBuffer: string[],
  stderrBuffer: string[],
  maxAttempts = 60,
): Promise<void> {
  for (let i = 0; i < maxAttempts; i++) {
    if (await isServerRunning(port)) {
      return
    }
    if (process.exitCode !== null || process.signalCode) {
      throw formatStartupFailure(
        process,
        port,
        stdoutBuffer,
        stderrBuffer,
        'Server process exited before /system/health became ready.',
      )
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw formatStartupFailure(
    process,
    port,
    stdoutBuffer,
    stderrBuffer,
    'Timed out waiting for /system/health to become ready.',
  )
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
  const configDir = mkdtempSync(join(tmpdir(), 'browseros-server-config-'))
  const configPath = join(configDir, 'sidecar.json')
  writeFileSync(
    configPath,
    `${JSON.stringify(
      {
        ports: {
          server: config.serverPort,
          cdp: config.cdpPort,
          proxy: config.serverPort,
        },
        directories: {
          resources: join(MONOREPO_ROOT, 'resources'),
          execution: configDir,
        },
        flags: {
          allow_remote_in_mcp: false,
        },
        instance: {
          client_id: '',
          install_id: '',
          browseros_version: '',
          chromium_version: '',
        },
      },
      null,
      2,
    )}\n`,
  )

  const stdoutBuffer: string[] = []
  const stderrBuffer: string[] = []
  const process = spawn(
    'bun',
    [SERVER_ENTRYPOINT_PATH, '--config', configPath],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: {
        ...globalThis.process.env,
        NODE_ENV: 'test',
        BROWSEROS_USE_MOCK_LLM: 'true',
      },
    },
  )

  process.stdout?.on('data', (data) => {
    appendBufferedLog(stdoutBuffer, data)
  })

  process.stderr?.on('data', (data) => {
    appendBufferedLog(stderrBuffer, data)
  })

  process.on('error', (error) => {
    console.error('Failed to start server:', error)
  })

  console.log('Waiting for server to be ready...')
  try {
    await waitForHealth(process, config.serverPort, stdoutBuffer, stderrBuffer)
  } catch (error) {
    process.kill('SIGTERM')
    rmSync(configDir, { recursive: true, force: true })
    throw error
  }
  console.log('Server is ready')

  serverState = { process, config, configDir }
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
  rmSync(serverState.configDir, { recursive: true, force: true })
  serverState = null
}
