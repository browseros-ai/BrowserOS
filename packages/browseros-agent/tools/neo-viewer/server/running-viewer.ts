/**
 * The viewer process serving an instance, recorded in the instance's state folder
 * (`viewer.json`), so a later start of the same instance can tell it is still running.
 */

import { readFile } from 'node:fs/promises'

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
