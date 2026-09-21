/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pino-based logger with:
 * - Source tracking (file:line:function) in development
 * - Async file writes with daily rotation
 * - Pretty colored console in dev, structured JSON in prod
 * - Metadata truncation in console output
 */

import fs from 'node:fs'
import path from 'node:path'
import { CONTENT_LIMITS } from '@browseros/shared/constants/limits'
import type { LoggerInterface, LogLevel } from '@browseros/shared/types/logger'
import pino from 'pino'

const isDev = process.env.NODE_ENV === 'development'
const LOG_FILE_NAME = 'browseros-server.log'
const LOG_FILE_MAX_AGE_MS = 24 * 60 * 60 * 1000 // 1 day
const LOG_FILE_MAX_BYTES = 50 * 1024 * 1024 // 50 MB
const LOG_ROTATE_CHECK_MS = 5 * 60 * 1000 // check log size every 5 minutes

type LogDestination = ReturnType<typeof pino.destination>

let stderrGuarded = false

/**
 * A write to process.stderr can fail asynchronously (EPIPE when its reader has
 * closed the pipe); that 'error' event escapes the try/catch in guardDestination
 * and, with no listener, would crash the process the guard exists to protect.
 * Register a one-time no-op listener so it stays non-fatal.
 */
function ensureStderrGuard(): void {
  if (stderrGuarded) return
  stderrGuarded = true
  process.stderr.on('error', () => {
    // The last-resort output is gone; there is nowhere safe left to write.
  })
}

/**
 * Degrade a failed destination write (ENOSPC, EACCES, EPIPE) to stderr instead
 * of letting it surface as an uncaught error. Without this, a full disk turns
 * one failed log flush into a crash-and-capture loop.
 */
function guardDestination(destination: LogDestination, label: string): void {
  ensureStderrGuard()
  destination.on('error', (error: NodeJS.ErrnoException) => {
    try {
      process.stderr.write(
        `[logger] ${label} destination write failed: ${error?.code ?? error?.message ?? error}\n`,
      )
    } catch {
      // stderr itself is gone; nothing safe is left to do.
    }
  })
}

/** Rotate when the log is older than the max age or larger than the max size. */
export function shouldRotateLog(
  sizeBytes: number,
  mtimeMs: number,
  now: number,
): boolean {
  return now - mtimeMs > LOG_FILE_MAX_AGE_MS || sizeBytes > LOG_FILE_MAX_BYTES
}

/** Move the current log aside to a single `.old` backup. */
function moveToBackup(logPath: string): void {
  const backupPath = `${logPath}.old`
  try {
    fs.unlinkSync(backupPath)
  } catch {
    // No previous backup; fine.
  }
  fs.renameSync(logPath, backupPath)
}

/**
 * Parse caller info from stack trace.
 * Returns "filename:line:function" or null if parsing fails.
 */
function parseCallerInfo(stack: string): string | null {
  const lines = stack.split('\n')

  for (const line of lines) {
    // Skip internal frames
    if (line.includes('logger.ts')) continue
    if (line.includes('node_modules/pino')) continue
    if (line.includes('node_modules\\pino')) continue
    if (!line.includes(' at ')) continue

    // Match: "at functionName (file:line:col)" or "at file:line:col"
    const match = line.match(/at\s+(?:(.+?)\s+\()?(.+?):(\d+):(\d+)\)?/)
    if (match) {
      const [, fnName, file, lineNum] = match
      const shortFile = file.replace(/^.*[/\\]/, '')
      const fn = fnName || 'anonymous'
      return `${shortFile}:${lineNum}:${fn}`
    }
  }

  return null
}

/**
 * Truncate long string values in an object for console output.
 */
function truncateForConsole(
  obj: Record<string, unknown>,
  maxLen: number,
): Record<string, unknown> {
  const result: Record<string, unknown> = {}

  for (const [key, value] of Object.entries(obj)) {
    if (typeof value === 'string' && value.length > maxLen) {
      result[key] =
        `${value.slice(0, maxLen)}... (+${value.length - maxLen} chars)`
    } else if (value && typeof value === 'object' && !Array.isArray(value)) {
      result[key] = truncateForConsole(value as Record<string, unknown>, maxLen)
    } else {
      result[key] = value
    }
  }

  return result
}

/** Startup rotation: move the log aside if it is too old or too large. */
function rotateLogIfNeeded(logPath: string): void {
  try {
    const stat = fs.statSync(logPath)
    if (shouldRotateLog(stat.size, stat.mtimeMs, Date.now())) {
      moveToBackup(logPath)
    }
  } catch {
    // File does not exist yet; nothing to rotate.
  }
}

/**
 * Create pino transport configuration for console output.
 * Returns null for production (use sync stdout to avoid thread-stream issues with Bun compile).
 */
function createConsoleTransport(): pino.TransportSingleOptions | null {
  if (isDev) {
    return {
      target: 'pino-pretty',
      options: {
        colorize: true,
        translateTime: 'SYS:HH:MM:ss.l',
        ignore: 'pid,hostname',
      },
    }
  }

  // Production: return null to use synchronous stdout logging.
  // pino.transport() uses thread-stream which doesn't work with Bun compile.
  return null
}

class Logger implements LoggerInterface {
  private consoleLogger: pino.Logger
  private fileLogger: pino.Logger | null = null
  private fileDestination: LogDestination | null = null
  private logPath: string | null = null
  private rotationTimer: ReturnType<typeof setInterval> | null = null
  private level: LogLevel

  constructor(level?: LogLevel) {
    this.level =
      level ||
      (process.env.LOG_LEVEL as LogLevel | undefined) ||
      (isDev ? 'debug' : 'info')
    this.consoleLogger = this.createConsoleLogger()
  }

  private createConsoleLogger(): pino.Logger {
    const options: pino.LoggerOptions = {
      level: this.level,
    }

    // Add source tracking in development
    if (isDev) {
      options.mixin = () => {
        const caller = parseCallerInfo(new Error().stack || '')
        return caller ? { caller } : {}
      }
    }

    const transport = createConsoleTransport()
    if (transport) {
      return pino(options, pino.transport(transport))
    }

    // Production: use pino.destination() for async writes without worker threads.
    // pino.transport() uses thread-stream which fails with Bun compile.
    // pino.destination() uses SonicBoom directly - no workers, bundling-safe.
    const destination = pino.destination({ dest: 1, sync: false })
    guardDestination(destination, 'console')
    return pino(options, destination)
  }

  /**
   * Configure file logging with async writes and rotation.
   */
  setLogFile(logDir: string): void {
    const logPath = path.join(logDir, LOG_FILE_NAME)

    // Rotate old or oversized logs on startup.
    rotateLogIfNeeded(logPath)

    // Create async file destination
    const fileDestination = pino.destination({
      dest: logPath,
      sync: false,
      mkdir: true,
    })
    guardDestination(fileDestination, 'file')

    this.fileDestination = fileDestination
    this.logPath = logPath
    // File logger: always JSON, no source tracking (for performance)
    this.fileLogger = pino({ level: this.level }, fileDestination)

    this.startRotationWatch()
  }

  private startRotationWatch(): void {
    if (this.rotationTimer) clearInterval(this.rotationTimer)
    this.rotationTimer = setInterval(
      () => this.rotateFileIfOversized(),
      LOG_ROTATE_CHECK_MS,
    )
    // Never keep the process alive just to check log size.
    this.rotationTimer.unref?.()
  }

  private rotateFileIfOversized(): void {
    if (!this.logPath || !this.fileDestination) return
    try {
      const stat = fs.statSync(this.logPath)
      if (stat.size <= LOG_FILE_MAX_BYTES) return
      moveToBackup(this.logPath)
      // logrotate pattern: after the file is renamed, SonicBoom reopens a fresh
      // one at the original path.
      this.fileDestination.reopen()
    } catch {
      // A failed stat/rename/reopen (e.g. disk full) must not crash the timer;
      // the destination error guard already handles write failures.
    }
  }

  setLevel(level: LogLevel): void {
    this.level = level
    this.consoleLogger.level = level
    if (this.fileLogger) {
      this.fileLogger.level = level
    }
  }

  private log(
    level: LogLevel,
    message: string,
    meta?: Record<string, unknown>,
  ): void {
    const logFn = this.consoleLogger[level].bind(this.consoleLogger)
    const fileLogFn = this.fileLogger?.[level].bind(this.fileLogger)

    // Console: truncate large values in dev (skip for error/warn so full context is visible)
    if (meta && isDev && level !== 'error' && level !== 'warn') {
      const truncated = truncateForConsole(
        meta,
        CONTENT_LIMITS.CONSOLE_META_CHAR,
      )
      logFn(truncated, message)
    } else if (meta) {
      logFn(meta, message)
    } else {
      logFn(message)
    }

    // File: always log full data, no truncation
    if (fileLogFn) {
      if (meta) {
        fileLogFn(meta, message)
      } else {
        fileLogFn(message)
      }
    }
  }

  debug(message: string, meta?: Record<string, unknown>): void {
    this.log('debug', message, meta)
  }

  info(message: string, meta?: Record<string, unknown>): void {
    this.log('info', message, meta)
  }

  warn(message: string, meta?: Record<string, unknown>): void {
    this.log('warn', message, meta)
  }

  error(message: string, meta?: Record<string, unknown>): void {
    this.log('error', message, meta)
  }
}

export const logger = new Logger()
