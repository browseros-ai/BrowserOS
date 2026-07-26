/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { existsSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import { env } from '../../env'
import type { AuditDb } from './db'

interface DrizzleJournalEntry {
  tag: string
}

const sourceMigrationsFolder = resolve(import.meta.dir, '../../../drizzle')
const sourcePackageJson = resolve(import.meta.dir, '../../../package.json')

/** Applies Claw audit DB migrations from packaged resources when available. */
export function runMigrations(db: AuditDb): void {
  migrate(db, { migrationsFolder: resolveMigrationsFolder() })
}

/** Resolves packaged migrations first, with source migrations only in source checkouts. */
export function resolveMigrationsFolder(
  resourcesDir = env.resourcesDir,
): string {
  const packaged = join(resourcesDir, 'db', 'migrations')
  if (hasCompleteMigrationSet(packaged)) return packaged
  if (hasSourceMigrationFallback()) return sourceMigrationsFolder
  throw new Error(
    `Claw migrations not found. Expected packaged migrations at ${packaged}`,
  )
}

function hasCompleteMigrationSet(migrationsFolder: string): boolean {
  const candidateJournal = readDrizzleJournal(
    join(migrationsFolder, 'meta', '_journal.json'),
  )
  if (!candidateJournal) return false

  return candidateJournal.entries.every((entry) =>
    existsSync(join(migrationsFolder, `${entry.tag}.sql`)),
  )
}

function hasSourceMigrationFallback(): boolean {
  return (
    existsSync(sourcePackageJson) &&
    hasCompleteMigrationSet(sourceMigrationsFolder)
  )
}

function readDrizzleJournal(
  path: string,
): { entries: DrizzleJournalEntry[] } | null {
  if (!existsSync(path)) return null

  try {
    const journal = JSON.parse(readFileSync(path, 'utf8')) as unknown
    if (!isDrizzleJournal(journal)) return null
    return journal
  } catch {
    return null
  }
}

function isDrizzleJournal(
  value: unknown,
): value is { entries: DrizzleJournalEntry[] } {
  return (
    typeof value === 'object' &&
    value !== null &&
    'entries' in value &&
    Array.isArray(value.entries) &&
    value.entries.every(
      (entry) =>
        typeof entry === 'object' &&
        entry !== null &&
        'tag' in entry &&
        typeof entry.tag === 'string',
    )
  )
}
