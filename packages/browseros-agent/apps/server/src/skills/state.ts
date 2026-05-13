import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { getSkillsSourcesPath, getSkillsStatePath } from '../lib/browseros-dir'
import type { SkillsRuntimeState, SkillsSourcesRegistry } from './types'

const EMPTY_SOURCES: SkillsSourcesRegistry = { version: 1, sources: [] }
const EMPTY_STATE: SkillsRuntimeState = { version: 1, skills: {} }

function assertAbsolutePath(path: string): string {
  const resolved = resolve(path)
  if (!resolved.startsWith('/')) {
    throw new Error('External source path must be absolute')
  }
  return resolved
}

async function readJsonFile<T>(filePath: string, fallback: T): Promise<T> {
  try {
    const content = await readFile(filePath, 'utf-8')
    return JSON.parse(content) as T
  } catch {
    return fallback
  }
}

async function writeJsonFile(
  filePath: string,
  payload: unknown,
): Promise<void> {
  await mkdir(dirname(filePath), { recursive: true })
  await writeFile(filePath, `${JSON.stringify(payload, null, 2)}\n`)
}

export async function loadSkillsSources(): Promise<SkillsSourcesRegistry> {
  const payload = await readJsonFile<Partial<SkillsSourcesRegistry>>(
    getSkillsSourcesPath(),
    EMPTY_SOURCES,
  )
  return {
    version: 1,
    sources: (payload.sources ?? []).map((source) => ({
      ...source,
      type: 'external',
      path: assertAbsolutePath(source.path),
    })),
  }
}

export async function saveSkillsSources(
  payload: SkillsSourcesRegistry,
): Promise<void> {
  await writeJsonFile(getSkillsSourcesPath(), {
    version: 1,
    sources: payload.sources.map((source) => ({
      ...source,
      type: 'external',
      path: assertAbsolutePath(source.path),
    })),
  })
}

export async function loadSkillsState(): Promise<SkillsRuntimeState> {
  const payload = await readJsonFile<Partial<SkillsRuntimeState>>(
    getSkillsStatePath(),
    EMPTY_STATE,
  )
  return {
    version: 1,
    skills: payload.skills ?? {},
  }
}

export async function saveSkillsState(
  payload: SkillsRuntimeState,
): Promise<void> {
  await writeJsonFile(getSkillsStatePath(), {
    version: 1,
    skills: payload.skills,
  })
}
