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

/**
 * Rotate log file if it's older than max age.
 * Simple startup-time rotation - deletes old backup, renames current to .old
 */
function rotateLogIfNeeded(logPath: string): void {
  try {
    const stat = fs.statSync(logPath)
    const ageMs = Date.now() - stat.mtimeMs

    if (ageMs > LOG_FILE_MAX_AGE_MS) {
      const backupPath = `${logPath}.old`
      try {
        fs.unlinkSync(backupPath)
      } catch {
        // Backup doesn't exist, that's fine
      }
      fs.renameSync(logPath, backupPath)
    }
  } catch {
    // File doesn't exist, nothing to rotate
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
    return pino(options, pino.destination({ dest: 1, sync: false }))
  }

  /**
   * Configure file logging with async writes and rotation.
   */
  setLogFile(logDir: string): void {
    const logPath = path.join(logDir, LOG_FILE_NAME)

    // Rotate old logs on startup
    rotateLogIfNeeded(logPath)

    // Create async file destination
    const fileDestination = pino.destination({
      dest: logPath,
      sync: false,
      mkdir: true,
    })

    // File logger: always JSON, no source tracking (for performance)
    this.fileLogger = pino({ level: this.level }, fileDestination)
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
