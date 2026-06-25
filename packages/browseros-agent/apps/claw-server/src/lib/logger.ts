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
 */

type LogLevel = 'debug' | 'info' | 'warn' | 'error'

const LEVEL_PRIORITY: Record<LogLevel, number> = {
  debug: 20,
  info: 30,
  warn: 40,
  error: 50,
}

function write(level: LogLevel, msg: string, fields?: Record<string, unknown>) {
  const event = {
    level: LEVEL_PRIORITY[level],
    time: Date.now(),
    msg,
    ...fields,
  }
  // biome-ignore lint/suspicious/noConsole: logger is the sanctioned console wrapper for the package
  console.error(JSON.stringify(event))
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
}
