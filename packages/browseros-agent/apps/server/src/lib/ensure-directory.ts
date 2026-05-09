/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Stats } from 'node:fs'
import { mkdir as defaultMkdir, stat as defaultStat } from 'node:fs/promises'

interface EnsureDirectoryDeps {
  mkdir?: typeof defaultMkdir
  stat?: typeof defaultStat
}

export async function ensureDirectory(
  path: string,
  deps: EnsureDirectoryDeps = {},
): Promise<void> {
  const mkdir = deps.mkdir ?? defaultMkdir
  try {
    await mkdir(path, { recursive: true })
    return
  } catch (err) {
    if (!isAlreadyExistsError(err)) throw err
    const info = await statExistingDirectory(path, err, deps.stat)
    if (!info.isDirectory()) throw err
  }
}

async function statExistingDirectory(
  path: string,
  originalError: unknown,
  stat: typeof defaultStat = defaultStat,
): Promise<Stats> {
  try {
    return await stat(path)
  } catch {
    throw originalError
  }
}

function isAlreadyExistsError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'EEXIST'
  )
}
