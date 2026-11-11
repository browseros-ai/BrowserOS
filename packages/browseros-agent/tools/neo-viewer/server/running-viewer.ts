/**
 * An instance's state folder: which instance owns it (`instance`), and the viewer process serving
 * it (`viewer.json`), so a later start can tell it is still running.
 */

import { readdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import type { StartState } from './neo-container'

/**
 * Claims the state folder for `instance`, or refuses before anything in it changes when another
 * instance owns it: a start clears the folder's history and rewrites its token, so two instances
 * given the same `--data-dir` would clear each other's.
 */
export async function claimStateFolder(
  dir: string,
  instance: string,
): Promise<void> {
  const marker = join(dir, 'instance')
  // Created exclusively: of two starts claiming a new folder at once, one writes it and the
  // other reads the winner's name.
  const created = await writeFile(marker, `${instance}\n`, { flag: 'wx' }).then(
    () => true,
    (error: NodeJS.ErrnoException) => {
      if (error.code === 'EEXIST') return false
      throw error
    },
  )
  if (created) return
  const owner = (await readFile(marker, 'utf8')).trim()
  if (owner !== instance) {
    throw new Error(
      `neo-viewer: ${dir} holds instance ${owner}'s state; give instance ${instance} its own --data-dir (or none)`,
    )
  }
}

/**
 * Whether the viewer's port is free on 127.0.0.1, checked before any Docker work: a start that
 * cannot listen would otherwise replace a running viewer's container and then fail. Catches a
 * viewer of the same instance whatever its state folder, and anything else on the port.
 */
export function portFree(port: number): boolean {
  if (port === 0) return true
  try {
    Bun.serve({
      hostname: '127.0.0.1',
      port,
      fetch: () => new Response(null),
    }).stop(true)
    return true
  } catch {
    return false
  }
}

export interface ViewerRecord {
  pid: number
  port: number
  url: string
}

export async function readViewerFile(
  path: string,
): Promise<ViewerRecord | null> {
  try {
    const record = JSON.parse(
      await readFile(path, 'utf8'),
    ) as Partial<ViewerRecord>
    if (typeof record.pid !== 'number' || typeof record.port !== 'number')
      return null
    return { pid: record.pid, port: record.port, url: String(record.url ?? '') }
  } catch {
    return null
  }
}

/**
 * The recorded viewer when its process is alive and is a viewer; null otherwise. A crashed
 * viewer leaves its file behind and the OS can give its pid to anything, so the process's
 * command line must still name this tool's entry.
 */
export async function liveViewer(
  path: string,
  commandOf: (pid: number) => Promise<string | null> = processCommand,
): Promise<ViewerRecord | null> {
  const record = await readViewerFile(path)
  if (!record || record.pid === process.pid) return null
  const command = await commandOf(record.pid)
  return command?.includes('server/main.ts') ? record : null
}

async function processCommand(pid: number): Promise<string | null> {
  const child = Bun.spawn(['ps', '-o', 'command=', '-p', String(pid)], {
    stdout: 'pipe',
    stderr: 'ignore',
  })
  const [out, code] = await Promise.all([
    new Response(child.stdout).text(),
    child.exited,
  ])
  return code === 0 ? out.trim() : null
}

/**
 * The start states of the other instances: the folders beside this one, and those under the state
 * root (`~/.local/state/neo-viewer/<name>`), where instances without `--data-dir` live. An image
 * another instance last built is one its next start would reuse, so it is not this start's to remove.
 */
export async function otherInstanceStates(
  dataDir: string,
  root: string,
): Promise<StartState[]> {
  const own = resolve(dataDir)
  const folders = new Set<string>()
  for (const parent of new Set([dirname(own), resolve(root)])) {
    const entries = await readdir(parent, { withFileTypes: true }).catch(
      () => [],
    )
    for (const entry of entries)
      if (entry.isDirectory()) folders.add(join(parent, entry.name))
  }
  folders.delete(own)
  const states: StartState[] = []
  for (const folder of folders) {
    try {
      states.push(
        JSON.parse(
          await readFile(join(folder, 'start-state.json'), 'utf8'),
        ) as StartState,
      )
    } catch {
      // not an instance's folder, or it never built
    }
  }
  return states
}

/**
 * Writes `text` to a temporary file beside `path` and renames it into place, so a reader (another
 * instance's start, reading this one's `start-state.json`) sees the old file or the new one, never
 * a truncated one it would skip as unreadable.
 */
export async function writeFileAtomically(
  path: string,
  text: string,
): Promise<void> {
  const temporary = `${path}.${process.pid}.tmp`
  try {
    await writeFile(temporary, text)
    await rename(temporary, path)
  } catch (error) {
    await rm(temporary, { force: true })
    throw error
  }
}
