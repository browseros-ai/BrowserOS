/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Reads Firefox's SQLite files while Firefox keeps running. Firefox holds its
 * databases open in WAL mode, so the newest writes live in `<db>-wal` until a
 * checkpoint. Copying the file and its WAL into a private directory gives a
 * consistent view without touching the live profile; the copy is opened
 * writable so SQLite may replay the WAL. The copy holds secrets (cookie
 * values, the login key), so it is deleted as soon as the reader returns.
 *
 * The two files cannot be copied atomically. If Firefox checkpoints or writes
 * between the copies, the WAL may not belong to the copied database and the
 * replay can mix old and new pages. So each copy is checked against the size
 * and mtime of both files before and after, and retried until they held still.
 */

import { Database } from 'bun:sqlite'
import { copyFileSync, mkdtempSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

/** Enough for a busy profile: one copy takes milliseconds, writes are bursts. */
const COPY_ATTEMPTS = 5

function isMissing(error: unknown): boolean {
  return (error as NodeJS.ErrnoException)?.code === 'ENOENT'
}

/** Size and mtime of a file, or null when it does not exist. */
function fingerprint(path: string): string | null {
  try {
    const stat = statSync(path)
    return `${stat.size}:${stat.mtimeMs}`
  } catch (error) {
    if (isMissing(error)) return null
    throw error
  }
}

/**
 * Copies a file that may vanish at any moment: Firefox deletes the WAL when
 * it closes the database, so an exists-then-copy check would race with it.
 */
function copyIfPresent(source: string, target: string): boolean {
  try {
    copyFileSync(source, target)
    return true
  } catch (error) {
    if (isMissing(error)) return false
    throw error
  }
}

/** Copies the database and its WAL until both stayed unchanged during the copy. */
function copyStableSnapshot(source: string, target: string): void {
  const files = (path: string) => [path, `${path}-wal`]
  for (let attempt = 1; attempt <= COPY_ATTEMPTS; attempt++) {
    const before = files(source).map(fingerprint)
    if (before[0] === null || !copyIfPresent(source, target)) {
      throw new Error(`${source} not found`)
    }
    if (!copyIfPresent(`${source}-wal`, `${target}-wal`)) {
      // A WAL left from an earlier attempt would be replayed onto this copy.
      rmSync(`${target}-wal`, { force: true })
    }
    const after = files(source).map(fingerprint)
    if (before.every((value, i) => value === after[i])) return
  }
  throw new Error(
    `${source} kept changing while it was copied; try again in a moment`,
  )
}

export function withSqliteSnapshot<T>(
  profileDir: string,
  fileName: string,
  read: (db: Database) => T,
): T {
  const source = join(profileDir, fileName)
  if (fingerprint(source) === null) {
    throw new Error(`${fileName} not found in ${profileDir}`)
  }
  const dir = mkdtempSync(join(tmpdir(), 'browseros-firefox-sync-'))
  try {
    const target = join(dir, fileName)
    copyStableSnapshot(source, target)
    const db = new Database(target)
    try {
      return read(db)
    } finally {
      db.close()
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}
