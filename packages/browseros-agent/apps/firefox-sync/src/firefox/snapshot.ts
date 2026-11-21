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
 */

import { Database } from 'bun:sqlite'
import { copyFileSync, existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export function withSqliteSnapshot<T>(
  profileDir: string,
  fileName: string,
  read: (db: Database) => T,
): T {
  const source = join(profileDir, fileName)
  if (!existsSync(source)) {
    throw new Error(`${fileName} not found in ${profileDir}`)
  }
  const dir = mkdtempSync(join(tmpdir(), 'browseros-firefox-sync-'))
  try {
    const target = join(dir, fileName)
    copyFileSync(source, target)
    if (existsSync(`${source}-wal`)) {
      copyFileSync(`${source}-wal`, `${target}-wal`)
    }
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
