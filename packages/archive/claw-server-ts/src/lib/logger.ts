/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Structured JSON logger. Writes one event per line to stderr so
 * downstream log shippers can `tail -F` without competing with
 * stdout traffic. The shape matches @browseros/server's pino output
 * (level, time, msg, plus arbitrary structured fields) so existing
 * log views render both producers identically.
 *
 * `setLogFile` adds an optional file sink with startup-time rotation
 * (rename to `.old` when the file was created over 24h ago or grew
 * past the size cap) so prod runs keep an on-disk record. Deliberately
 * dep-free: pino's async transports bring Bun-compile caveats, and at
 * this log volume sync per-line writes are fine — and survive crashes,
 * which is when the file matters most.
 *
 * Known limitation: rotation assumes one claw-server per
 * <browserclawDir>. Two instances on different ports sharing a dir can
 * rotate each other's live log; guarding that needs a real file lock,
 * which this basic logger doesn't attempt.
 */

import fs from 'node:fs'
import path from 'node:path'

const LOG_FILE_NAME = 'claw-server.log'
const LOG_FILE_MAX_AGE_MS = 24 * 60 * 60 * 1000
// Backstop for filesystems without birthtime, where an actively
// written log never looks stale: cap growth at restart boundaries.
const LOG_FILE_MAX_SIZE_BYTES = 20 * 1024 * 1024

type LogLevel = 'debug' | 'info' | 'warn' | 'error'

const LEVEL_PRIORITY: Record<LogLevel, number> = {
  debug: 20,
  info: 30,
  warn: 40,
  error: 50,
}

let fileFd: number | null = null
let filePath: string | null = null

/**
 * Rotate the log file if it was created more than max age ago or
 * outgrew the size cap. Startup-time only: renames current to
 * `.old`, replacing any previous backup.
 */
function rotateLogIfNeeded(logPath: string): void {
  let stale = false
  try {
    const stat = fs.statSync(logPath)
    // Keyed on creation time, not mtime: every write refreshes mtime,
    // so an mtime key would never rotate a log with regular traffic.
    // birthtime is 0 on filesystems that don't track it.
    const createdMs = stat.birthtimeMs > 0 ? stat.birthtimeMs : stat.mtimeMs
    stale =
      Date.now() - createdMs > LOG_FILE_MAX_AGE_MS ||
      stat.size > LOG_FILE_MAX_SIZE_BYTES
  } catch {
    return // No log file yet, nothing to rotate
  }
  if (!stale) return

  const backupPath = `${logPath}.old`
  try {
    // rename replaces an existing backup atomically; unlink-first
    // would lose the old backup if the rename then failed.
    fs.renameSync(logPath, backupPath)
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code
    // The unlink fallback is only for dest-exists failures (Windows
    // semantics), which require an existing backup; unlinking on any
    // other error would destroy the backup without enabling the
    // rename, and the original error is the one worth reporting.
    const destExists =
      (code === 'EEXIST' || code === 'EPERM') && fs.existsSync(backupPath)
    if (!destExists) {
      warnRotationFailed(logPath, error)
      return
    }
    try {
      fs.unlinkSync(backupPath)
      fs.renameSync(logPath, backupPath)
    } catch (retryError) {
      warnRotationFailed(logPath, retryError)
    }
  }
}

function warnRotationFailed(logPath: string, error: unknown): void {
  write('warn', 'log rotation failed; appending to stale log', {
    logPath,
    error: error instanceof Error ? error.message : String(error),
  })
}

function closeLogFile(): void {
  filePath = null
  if (fileFd === null) return
  const fd = fileFd
  fileFd = null
  try {
    fs.closeSync(fd)
  } catch {
    // Already closed or invalid; sink is detached either way
  }
}

/**
 * Point the file sink at `<logDir>/claw-server.log`, rotating a stale
 * file first. Never throws, and only swaps sinks once the new file is
 * open — a broken log dir leaves any working sink (or stderr-only
 * logging) intact rather than taking the server down.
 */
function setLogFile(logDir: string): void {
  const logPath = path.join(logDir, LOG_FILE_NAME)
  let fd: number
  try {
    fs.mkdirSync(logDir, { recursive: true })
    // Never rotate the file the live sink is writing to: if the open
    // below failed, the kept fd would silently follow the rename into
    // the .old backup.
    if (fileFd === null || filePath !== logPath) {
      rotateLogIfNeeded(logPath)
    }
    fd = fs.openSync(logPath, 'a')
  } catch (error) {
    write('warn', 'could not open log file; file sink unchanged', {
      logPath,
      error: error instanceof Error ? error.message : String(error),
    })
    return
  }
  closeLogFile()
  fileFd = fd
  filePath = logPath
}

function write(level: LogLevel, msg: string, fields?: Record<string, unknown>) {
  // Envelope keys stay first (pino-canonical order) and win over any
  // stray msg/level/time in caller data, so downstream log views keep
  // parsing every line.
  const { level: _level, time: _time, msg: _msg, ...rest } = fields ?? {}
  const event = {
    level: LEVEL_PRIORITY[level],
    time: Date.now(),
    msg,
    ...rest,
  }
  let line: string
  try {
    line = JSON.stringify(event)
  } catch {
    // Circular or BigInt field: drop fields rather than throw out of
    // a log call.
    line = JSON.stringify({
      level: LEVEL_PRIORITY[level],
      time: Date.now(),
      msg,
      logSerializationFailed: true,
    })
  }
  // biome-ignore lint/suspicious/noConsole: logger is the sanctioned console wrapper for the package
  console.error(line)
  if (fileFd !== null) {
    try {
      fs.writeSync(fileFd, `${line}\n`)
    } catch {
      // Dead sink (disk full, fd invalidated): detach so every later
      // log call doesn't re-fail; stderr keeps working.
      closeLogFile()
    }
  }
}

export const logger = {
  debug: (msg: string, fields?: Record<string, unknown>) =>
    write('debug', msg, fields),
  info: (msg: string, fields?: Record<string, unknown>) =>
    write('info', msg, fields),
  warn: (msg: string, fields?: Record<string, unknown>) =>
    write('warn', msg, fields),
  error: (msg: string, fields?: Record<string, unknown>) =>
    write('error', msg, fields),
  setLogFile,
  closeLogFile,
}
