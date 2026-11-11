/**
 * An instance's state folder: which instance owns it (`instance`), and the viewer process serving
 * it (`viewer.json`), so a later start can tell it is still running.
 */

import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

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
  const owner = await readFile(marker, 'utf8').then(
    (text) => text.trim(),
    () => null,
  )
  if (owner !== null && owner !== instance) {
    throw new Error(
      `neo-viewer: ${dir} holds instance ${owner}'s state; give instance ${instance} its own --data-dir (or none)`,
    )
  }
  if (owner === null) await writeFile(marker, `${instance}\n`)
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
