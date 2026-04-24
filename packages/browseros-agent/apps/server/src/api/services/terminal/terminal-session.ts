import {
  OPENCLAW_CONTAINER_HOME,
  OPENCLAW_TERMINAL_SHELL,
} from '@browseros/shared/constants/openclaw'
import { buildNerdctlCommand } from '../../../lib/container'
import { logger } from '../../../lib/logger'

export const TERMINAL_HOME_DIR = OPENCLAW_CONTAINER_HOME
const DEFAULT_COLS = 80
const DEFAULT_ROWS = 24
const TERMINAL_NAME = 'xterm-256color'

interface TerminalSessionDeps {
  containerName: string
  limaHome: string
  limactlPath: string
  vmName: string
  workingDir: string
  onExit: (exitCode: number) => void
  onOutput: (data: string) => void
}

export interface TerminalSession {
  close(): void
  resize(cols: number, rows: number): void
  writeInput(data: string): void
}

export function buildTerminalExecCommand(
  limactlPath: string,
  vmName: string,
  containerName: string,
  workingDir: string,
): string[] {
  return [
    limactlPath,
    'shell',
    vmName,
    '--',
    ...buildNerdctlCommand([
      'exec',
      '-it',
      '-w',
      workingDir,
      containerName,
      OPENCLAW_TERMINAL_SHELL,
    ]),
  ]
}

export function buildTerminalEnv(limaHome: string): NodeJS.ProcessEnv {
  return { ...process.env, LIMA_HOME: limaHome, TERM: TERMINAL_NAME }
}

export function createTerminalSession(
  deps: TerminalSessionDeps,
): TerminalSession {
  const decoder = new TextDecoder()
  const proc = Bun.spawn(
    buildTerminalExecCommand(
      deps.limactlPath,
      deps.vmName,
      deps.containerName,
      deps.workingDir,
    ),
    {
      cwd: '/',
      terminal: {
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
        data(_terminal, data) {
          const chunk = decoder.decode(data, { stream: true })
          if (chunk) deps.onOutput(chunk)
        },
      },
      env: buildTerminalEnv(deps.limaHome),
    },
  )
  let closed = false

  void proc.exited.then((exitCode) => {
    const trailing = decoder.decode()
    if (trailing) deps.onOutput(trailing)
    deps.onExit(exitCode)
  })

  logger.debug('Terminal session created', { workingDir: deps.workingDir })

  return {
    writeInput(data) {
      proc.terminal?.write(data)
    },
    resize(cols, rows) {
      proc.terminal?.resize(cols, rows)
    },
    close() {
      if (closed) return
      closed = true
      try {
        proc.terminal?.close()
        proc.kill()
      } catch {
        logger.debug('Terminal session cleanup failed')
      }
      logger.debug('Terminal session destroyed')
    },
  }
}
