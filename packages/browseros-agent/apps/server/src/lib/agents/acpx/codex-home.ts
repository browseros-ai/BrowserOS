/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { constants, type Stats } from 'node:fs'
import {
  access,
  mkdir,
  readFile,
  stat,
  symlink,
  writeFile,
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import type { AgentRuntimePaths } from './runtime-context'

export interface MaterializeCodexHomeInput {
  readonly paths: AgentRuntimePaths
  readonly skillNames: readonly string[]
  readonly sourceCodexHome?: string
}

export async function materializeCodexHome(
  input: MaterializeCodexHomeInput,
): Promise<void> {
  await mkdir(input.paths.codexHome, { recursive: true })
  const source =
    input.sourceCodexHome ??
    process.env.CODEX_HOME?.trim() ??
    join(homedir(), '.codex')
  await symlinkIfPresent(
    join(source, 'auth.json'),
    join(input.paths.codexHome, 'auth.json'),
  )
  await copyIfPresent(
    join(source, 'instructions.md'),
    join(input.paths.codexHome, 'instructions.md'),
  )
  for (const name of input.skillNames) {
    const target = join(input.paths.codexHome, 'skills', name, 'SKILL.md')
    await writeFileIfPresent(
      target,
      await readFile(
        join(input.paths.runtimeSkillsDir, name, 'SKILL.md'),
        'utf8',
      ),
    )
  }
}

async function symlinkIfPresent(source: string, target: string): Promise<void> {
  if (!(await sourceFileExists(source))) return
  await mkdir(dirname(target), { recursive: true })
  try {
    await symlink(source, target)
  } catch (err) {
    if (!isAlreadyExistsError(err)) throw err
  }
}

async function copyIfPresent(source: string, target: string): Promise<void> {
  if (!(await sourceFileExists(source))) return
  const content = await readFile(source, 'utf8')
  await writeFileIfPresent(target, content)
}

async function writeFileIfPresent(
  path: string,
  content: string,
): Promise<void> {
  await mkdir(dirname(path), { recursive: true })
  try {
    await writeFile(path, content, { encoding: 'utf8', flag: 'wx' })
  } catch (err) {
    if (!isAlreadyExistsError(err)) throw err
  }
}

async function sourceFileExists(path: string): Promise<boolean> {
  let info: Stats
  try {
    info = await stat(path)
    await access(path, constants.R_OK)
  } catch (err) {
    if (isNotFoundError(err)) return false
    throw err
  }
  if (!info.isFile()) {
    throw new Error(`Expected source file to be a file: ${path}`)
  }
  return true
}

function isNotFoundError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'ENOENT'
  )
}

function isAlreadyExistsError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'EEXIST'
  )
}
