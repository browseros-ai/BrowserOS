/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * File-backed JSON storage under the BrowserClaw state root. Every read and
 * write goes through a zod schema so on-disk shapes can't drift
 * silently. Writes are atomic via a `<name>.tmp` -> rename swap so a
 * crash mid-write leaves either the prior contents or nothing at all,
 * never a half-written file.
 *
 * The relative path argument is always evaluated against
 * `getClawServerDir()`; callers cannot reach outside the BrowserClaw state
 * root. Absolute paths or `..` segments throw `StorageInvalidPathError`
 * so a stray join doesn't accidentally escape.
 */

import { mkdir, readdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, normalize, sep } from 'node:path'
import type { ZodType } from 'zod'
import { resolveClawServerPath } from './browserclaw-dir'

export class StorageNotFoundError extends Error {
  readonly relPath: string
  constructor(relPath: string) {
    super(`storage: file not found at ${relPath}`)
    this.name = 'StorageNotFoundError'
    this.relPath = relPath
  }
}

export class StorageCorruptError extends Error {
  readonly relPath: string
  constructor(relPath: string, cause: unknown) {
    super(`storage: invalid contents at ${relPath}`, { cause })
    this.name = 'StorageCorruptError'
    this.relPath = relPath
  }
}

export class StorageInvalidPathError extends Error {
  readonly relPath: string
  constructor(relPath: string) {
    super(
      `storage: relative path escapes the BrowserClaw state root: ${relPath}`,
    )
    this.name = 'StorageInvalidPathError'
    this.relPath = relPath
  }
}

function guardRelativePath(relPath: string): void {
  if (isAbsolute(relPath)) throw new StorageInvalidPathError(relPath)
  // Inspect the raw input first: `normalize` collapses `agents/../config.json`
  // to `config.json`, which would silently escape the intended
  // subdirectory while still passing the rooted-prefix check below.
  // Reject any `..` segment in the input.
  if (relPath.split(/[\\/]/).includes('..')) {
    throw new StorageInvalidPathError(relPath)
  }
  const normalized = normalize(relPath)
  if (normalized.startsWith('..') || normalized.split(sep).includes('..')) {
    throw new StorageInvalidPathError(relPath)
  }
}

async function ensureParentDir(absolutePath: string): Promise<void> {
  await mkdir(dirname(absolutePath), { recursive: true })
}

function isFsError(err: unknown, code: string): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    (err as { code?: unknown }).code === code
  )
}

export async function ensureDir(relDir: string): Promise<void> {
  guardRelativePath(relDir)
  await mkdir(resolveClawServerPath(relDir), { recursive: true })
}

export async function readJson<T>(
  relPath: string,
  schema: ZodType<T>,
): Promise<T> {
  guardRelativePath(relPath)
  const abs = resolveClawServerPath(relPath)
  let raw: string
  try {
    raw = await readFile(abs, 'utf8')
  } catch (err) {
    if (isFsError(err, 'ENOENT')) throw new StorageNotFoundError(relPath)
    throw err
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch (err) {
    throw new StorageCorruptError(relPath, err)
  }
  const result = schema.safeParse(parsed)
  if (!result.success) throw new StorageCorruptError(relPath, result.error)
  return result.data
}

/**
 * Schema-less read used by migrations that need to peek at fields
 * before deciding whether the file is still valid under the current
 * schema. Throws `StorageNotFoundError` for missing files and
 * `StorageCorruptError` for JSON parse failures; schema-level
 * mismatches are the caller's problem.
 */
export async function readJsonRaw(relPath: string): Promise<unknown> {
  guardRelativePath(relPath)
  const abs = resolveClawServerPath(relPath)
  let raw: string
  try {
    raw = await readFile(abs, 'utf8')
  } catch (err) {
    if (isFsError(err, 'ENOENT')) throw new StorageNotFoundError(relPath)
    throw err
  }
  try {
    return JSON.parse(raw)
  } catch (err) {
    throw new StorageCorruptError(relPath, err)
  }
}

export async function writeJson<T>(
  relPath: string,
  value: T,
  schema: ZodType<T>,
): Promise<void> {
  guardRelativePath(relPath)
  const parseResult = schema.safeParse(value)
  if (!parseResult.success)
    throw new StorageCorruptError(relPath, parseResult.error)
  const abs = resolveClawServerPath(relPath)
  await ensureParentDir(abs)
  const tmp = `${abs}.tmp`
  await writeFile(tmp, JSON.stringify(parseResult.data, null, 2), 'utf8')
  await rename(tmp, abs)
}

/**
 * Returns file names (not paths) in the directory, filtered to
 * `.json` by default. Missing directories resolve to `[]` rather than
 * throwing; that matches the "first-run, nothing saved yet" UX.
 */
export async function listFiles(
  relDir: string,
  options: { extension?: string } = {},
): Promise<string[]> {
  guardRelativePath(relDir)
  const extension = options.extension ?? '.json'
  try {
    const entries = await readdir(resolveClawServerPath(relDir), {
      withFileTypes: true,
    })
    return entries
      .filter((entry) => entry.isFile() && entry.name.endsWith(extension))
      .map((entry) => entry.name)
  } catch (err) {
    if (isFsError(err, 'ENOENT')) return []
    throw err
  }
}
